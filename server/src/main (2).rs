mod admin;
mod alerts;
mod api;
mod billing;
mod crypto;
mod db;
mod diag;
mod features;
mod notify;
mod regions;
mod ssh_term;
mod state;
mod themes;
mod updates;
mod ws;

use anyhow::{Context, Result};
use axum::{
    extract::{ws::WebSocketUpgrade, Request, State},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use clap::{Parser, Subcommand};
use pharus_common::AgentSnapshot;
use state::{AgentState, AppState, SharedState};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

#[derive(Parser)]
#[command(name = "pharus", about = "Pharus monitoring server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server
    Serve {
        /// Listen address
        #[arg(long, env = "PHARUS_ADDR", default_value = "0.0.0.0:8080")]
        addr: String,
        /// SQLite database path
        #[arg(long, env = "PHARUS_DB", default_value = "pharus.db")]
        db: PathBuf,
        /// Themes root directory
        #[arg(long, env = "PHARUS_THEMES", default_value = "themes")]
        themes: PathBuf,
        /// Admin username; the account is created on first start from the
        /// matching PHARUS_ADMIN_PASSWORD.
        #[arg(long, env = "PHARUS_ADMIN_USER", default_value = "admin")]
        admin_user: String,
        /// Admin password (management API disabled if unset)
        #[arg(long, env = "PHARUS_ADMIN_PASSWORD")]
        admin_password: Option<String>,
    },
    /// Register an agent and print its token
    AddAgent {
        #[arg(long)]
        name: String,
        #[arg(long, env = "PHARUS_DB", default_value = "pharus.db")]
        db: PathBuf,
    },
    /// Set the active theme (takes effect immediately, no restart)
    SetTheme {
        #[arg(long)]
        name: String,
        #[arg(long, env = "PHARUS_DB", default_value = "pharus.db")]
        db: PathBuf,
    },
}

async fn agent_ws(State(state): State<SharedState>, ws: WebSocketUpgrade) -> axum::response::Response {
    ws.on_upgrade(move |socket| ws::handle_agent_socket(state, socket))
}

async fn browser_ws(State(state): State<SharedState>, ws: WebSocketUpgrade) -> axum::response::Response {
    ws.on_upgrade(move |socket| ws::handle_browser_socket(state, socket))
}

async fn term_ws(State(state): State<SharedState>, ws: WebSocketUpgrade, req: Request) -> axum::response::Response {
    // The terminal opens a shell on a host, so it requires an admin session.
    let authorized = crate::admin::authenticate(&state, req.headers())
        .map(|(username, _)| {
            let conn = state.db.lock().unwrap();
            match crate::db::find_user(&conn, &username) {
                Ok(Some((_, _, role, enabled))) => enabled && role == "admin",
                _ => false,
            }
        })
        .unwrap_or(false);
    if !authorized {
        return axum::response::IntoResponse::into_response(axum::http::StatusCode::UNAUTHORIZED);
    }
    ws.on_upgrade(move |socket| ws::handle_term_socket(state, socket))
}

async fn status_json(State(state): State<SharedState>) -> Json<Vec<AgentSnapshot>> {
    let ids: Vec<i64> = state.agents.read().unwrap().keys().copied().collect();
    let list = ids.iter().map(|id| state.snapshot_with_gates(*id)).collect();
    Json(list)
}

fn current_theme_dir(state: &SharedState) -> PathBuf {
    let name = {
        let db = state.db.lock().unwrap();
        db::get_setting(&db, "current_theme").ok().flatten()
    }
    .unwrap_or_else(|| "default".into());
    state.themes_root.join(name)
}

async fn themed_static(State(state): State<SharedState>, req: Request) -> Response {
    let theme_dir = current_theme_dir(&state);
    let svc = ServeDir::new(&theme_dir).fallback(ServeFile::new(theme_dir.join("index.html")));
    match svc.oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(e) => match e {},
    }
}

/// Serve a named page (host.html / admin.html) from the active theme so the
/// UI links can drop the `.html` suffix (`/host`, `/admin`).
async fn themed_page_host(State(state): State<SharedState>, req: Request) -> Response {
    serve_theme_page(&state, "host", req).await
}
async fn themed_page_admin(State(state): State<SharedState>, req: Request) -> Response {
    serve_theme_page(&state, "admin", req).await
}
async fn serve_theme_page(state: &SharedState, page: &str, req: Request) -> Response {
    let theme_dir = current_theme_dir(state);
    let file = theme_dir.join(format!("{page}.html"));
    let svc = if file.is_file() {
        ServeFile::new(file)
    } else {
        ServeFile::new(theme_dir.join("index.html"))
    };
    match svc.oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(e) => match e {},
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Same story as the agent: make the rustls provider deterministic so
    // outbound TLS (lettre SMTP, reqwest webhooks) can't panic on ambiguity.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    match Cli::parse().command {
        Command::AddAgent { name, db } => {
            let conn = rusqlite::Connection::open(&db)
                .with_context(|| format!("open db {}", db.display()))?;
            db::init(&conn)?;
            let (id, token) = db::add_agent(&conn, &name)?;
            println!("agent registered:");
            println!("  id    = {id}");
            println!("  name  = {name}");
            println!("  token = {token}");
            Ok(())
        }
        Command::SetTheme { name, db } => {
            let conn = rusqlite::Connection::open(&db)
                .with_context(|| format!("open db {}", db.display()))?;
            db::init(&conn)?;
            db::set_setting(&conn, "current_theme", &name)?;
            println!("current_theme = {name}");
            Ok(())
        }
        Command::Serve { addr, db, themes, admin_user, admin_password } => {
            serve(addr, db, themes, admin_user, admin_password).await
        }
    }
}

async fn serve(
    addr: String,
    db_path: PathBuf,
    themes_root: PathBuf,
    admin_user: String,
    admin_password: Option<String>,
) -> Result<()> {
    let conn = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("open db {}", db_path.display()))?;
    db::init(&conn)?;

    if let Some(pw) = admin_password {
        let hash = crate::admin::hash_password(&pw)?;
        db::insert_user(&conn, &admin_user, &hash, "admin")?;
    }

    let billing_map = db::list_billing(&conn)?;
    let traffic_map = db::load_traffic(&conn)?;
    let region_map = db::list_regions(&conn)?;
    let mut agents = HashMap::new();
    for (id, name) in db::list_agents(&conn)? {
        let traffic = traffic_map
            .get(&id)
            .map(|r| state::TrafficState {
                cycle_start: r.cycle_start,
                rx_bytes: r.rx_bytes,
                tx_bytes: r.tx_bytes,
                last_rx_total: r.last_rx_total,
                last_tx_total: r.last_tx_total,
            })
            .unwrap_or_default();
        agents.insert(
            id,
            AgentState {
                name,
                billing: billing_map.get(&id).cloned(),
                traffic,
                region: region_map.get(&id).map(|(c, s)| db::make_region(c, s)),
                features: features::effective_for(&conn, id).unwrap_or_default(),
                unlock: db::list_streaming(&conn, id).unwrap_or_default(),
                ..AgentState::default()
            },
        );
    }

    let (browser_tx, _) = broadcast::channel(256);
    let state: SharedState = Arc::new(AppState {
        agents: RwLock::new(agents),
        next_epoch: std::sync::atomic::AtomicU64::new(1),
        db: Mutex::new(conn),
        browser_tx,
        themes_root: themes_root.clone(),
        sessions: Mutex::new(HashMap::new()),
        login_failures: Mutex::new(HashMap::new()),
        task_waiters: Mutex::new(HashMap::new()),
        diag_pending: Mutex::new(HashMap::new()),
        diag_by_ip: Mutex::new(HashMap::new()),
        iperf3_by_agent: Mutex::new(HashMap::new()),
        update_cache: Mutex::new(None),
        theme_store_cache: Mutex::new(None),
        term_sessions: Mutex::new(HashMap::new()),
    });

    alerts::spawn(state.clone());

    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut ticks: u64 = 0;
            loop {
                tick.tick().await;
                ticks += 1;
                let (rows, traffic_rows) = {
                    let agents = state.agents.read().unwrap();
                    let rows: Vec<(i64, pharus_common::Metrics)> = agents
                        .iter()
                        .filter(|(_, a)| a.online)
                        .filter_map(|(id, a)| a.data.clone().map(|d| (*id, d)))
                        .collect();
                    let traffic_rows: Vec<(i64, db::TrafficRow)> = agents
                        .iter()
                        .filter(|(_, a)| a.traffic.last_rx_total.is_some())
                        .map(|(id, a)| {
                            (*id, db::TrafficRow {
                                cycle_start: a.traffic.cycle_start,
                                rx_bytes: a.traffic.rx_bytes,
                                tx_bytes: a.traffic.tx_bytes,
                                last_rx_total: a.traffic.last_rx_total,
                                last_tx_total: a.traffic.last_tx_total,
                            })
                        })
                        .collect();
                    (rows, traffic_rows)
                };
                let db = state.db.lock().unwrap();
                for (id, t) in &traffic_rows {
                    if let Err(e) = db::upsert_traffic(&db, *id, t) {
                        tracing::warn!(agent_id = id, error = %e, "traffic flush failed");
                    }
                }
                for (id, m) in rows {
                    if let Err(e) = db::insert_metrics(&db, id, &m) {
                        tracing::warn!(agent_id = id, error = %e, "history insert failed");
                    }
                }
                // hourly: drop ping samples past the 30 day retention window
                if ticks.is_multiple_of(60) {
                    let cutoff = chrono::Utc::now().timestamp() - 30 * 86_400;
                    if let Err(e) = db::prune_ping_history(&db, cutoff) {
                        tracing::warn!(error = %e, "ping history prune failed");
                    }
                }
            }
        });
    }

    let app = Router::new()
        .route("/ws/agent", get(agent_ws))
        .route("/api/stream", get(browser_ws))
        .route("/ws/term", get(term_ws))
        .route("/api/status", get(status_json))
        .route("/host", get(themed_page_host))
        .route("/admin", get(themed_page_admin))
        .merge(api::router())
        .merge(admin::router(state.clone()))
        .fallback(themed_static)
        .with_state(state);

    let addr: SocketAddr = addr.parse().context("invalid listen addr")?;
    info!(%addr, themes_root = %themes_root.display(), "pharus server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
