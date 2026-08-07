mod admin;
mod alerts;
mod billing;
mod crypto;
mod db;
mod notify;
mod regions;
mod state;
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
        /// Admin token for the billing management API (disabled if unset)
        #[arg(long, env = "PHARUS_ADMIN_TOKEN")]
        admin_token: Option<String>,
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

async fn status_json(State(state): State<SharedState>) -> Json<Vec<AgentSnapshot>> {
    let agents = state.agents.read().unwrap();
    let list = agents.iter().map(|(id, a)| a.snapshot(*id)).collect();
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

#[tokio::main]
async fn main() -> Result<()> {
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
        Command::Serve { addr, db, themes, admin_token } => serve(addr, db, themes, admin_token).await,
    }
}

async fn serve(addr: String, db_path: PathBuf, themes_root: PathBuf, admin_token: Option<String>) -> Result<()> {
    let conn = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("open db {}", db_path.display()))?;
    db::init(&conn)?;

    let billing_map = db::list_billing(&conn)?;
    let traffic_map = db::load_traffic(&conn)?;
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
        admin_token,
        task_waiters: Mutex::new(HashMap::new()),
    });

    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
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
            }
        });
    }

    let app = Router::new()
        .route("/ws/agent", get(agent_ws))
        .route("/api/stream", get(browser_ws))
        .route("/api/status", get(status_json))
        .merge(admin::router(state.clone()))
        .fallback(themed_static)
        .with_state(state);

    let addr: SocketAddr = addr.parse().context("invalid listen addr")?;
    info!(%addr, themes_root = %themes_root.display(), "pharus server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
