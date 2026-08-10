//! Public read-only API plus the diagnostics trigger endpoints.

use crate::state::SharedState;
use crate::{db, diag, features, regions};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

pub fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/meta", get(meta))
        .route("/api/regions", get(region_list))
        .route("/api/agents/:id/history", get(history))
        .route("/api/agents/:id/ping", get(ping_history))
        .route("/api/agents/:id/streaming", get(streaming))
        .route("/api/diag/lg", post(diag_lg))
        .route("/api/diag/mtr", post(diag_mtr))
}

/// Newest-first row cap, chosen so a 7 day window at the 60s history interval
/// still fits in one response.
const MAX_POINTS: i64 = 10_080;

fn range_seconds(range: Option<&str>) -> i64 {
    match range.unwrap_or("6h") {
        "1h" => 3_600,
        "24h" => 86_400,
        "7d" => 604_800,
        _ => 21_600,
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn meta(State(state): State<SharedState>) -> Response {
    let (theme, enabled, expiry_days, site_name, site_url, default_language, admin_enabled) = {
        let conn = state.db.lock().unwrap();
        let theme = db::get_setting(&conn, "current_theme")
            .ok()
            .flatten()
            .unwrap_or_else(|| "default".into());
        let global = features::global_defaults(&conn).unwrap_or_default();
        let enabled: Vec<String> = features::FEATURES
            .iter()
            .filter(|f| *global.get(**f).unwrap_or(&true))
            .map(|f| (*f).to_string())
            .collect();
        let get = |key: &str| -> Option<String> { db::get_setting(&conn, key).ok().flatten() };
        let expiry_days = get("expiry_alert_days").and_then(|v| v.parse::<i64>().ok()).unwrap_or(3);
        let site_name = get("site_name");
        let site_url = get("site_url");
        let default_language = get("default_language");
        let admin_enabled = db::count_users(&conn).unwrap_or(0) > 0;
        (
            theme,
            enabled,
            expiry_days,
            site_name,
            site_url,
            default_language,
            admin_enabled,
        )
    };
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "theme": theme,
        "features": enabled,
        "admin_enabled": admin_enabled,
        "expiry_alert_days": expiry_days,
        "site_name": site_name,
        "site_url": site_url,
        "default_language": default_language,
    }))
    .into_response()
}

async fn region_list() -> Response {
    let list: Vec<_> = regions::all()
        .iter()
        .map(|(code, name)| serde_json::json!({ "code": code, "name": name }))
        .collect();
    Json(list).into_response()
}

#[derive(Debug, Deserialize)]
struct RangeQuery {
    range: Option<String>,
}

async fn history(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Query(q): Query<RangeQuery>,
) -> Response {
    let since = now() - range_seconds(q.range.as_deref());
    let points = {
        let conn = state.db.lock().unwrap();
        db::metrics_history(&conn, agent_id, since, MAX_POINTS)
    };
    match points {
        Ok(points) => Json(serde_json::json!({ "points": points })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct PingQuery {
    range: Option<String>,
    task_id: Option<i64>,
}

async fn ping_history(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Query(q): Query<PingQuery>,
) -> Response {
    let since = now() - range_seconds(q.range.as_deref());
    let loaded = {
        let conn = state.db.lock().unwrap();
        db::ping_tasks_for(&conn, agent_id)
            .and_then(|tasks| Ok((tasks, db::ping_history(&conn, agent_id, q.task_id, since, MAX_POINTS)?)))
    };
    let (tasks, points) = match loaded {
        Ok(v) => v,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let tasks: Vec<_> = tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "label": t.label,
                "kind": t.kind,
                "target": t.target,
            })
        })
        .collect();
    Json(serde_json::json!({ "tasks": tasks, "points": points })).into_response()
}

async fn streaming(State(state): State<SharedState>, Path(agent_id): Path<i64>) -> Response {
    if !features::enabled(&state, agent_id, "streaming") {
        return err(StatusCode::FORBIDDEN, "该功能已被管理员关闭");
    }
    // Prefer the live in-memory results; fall back to the last persisted run so
    // the panel is not empty while an agent is offline.
    let live = {
        let agents = state.agents.read().unwrap();
        agents
            .get(&agent_id)
            .map(|a| a.unlock.clone())
            .filter(|r| !r.is_empty())
    };
    let results = match live {
        Some(r) => r,
        None => {
            let conn = state.db.lock().unwrap();
            match db::list_streaming(&conn, agent_id) {
                Ok(r) => r,
                Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
    };
    Json(serde_json::json!({ "results": results })).into_response()
}

fn diag_response(result: Result<String, diag::DiagError>) -> Response {
    match result {
        Ok(request_id) => Json(serde_json::json!({ "request_id": request_id })).into_response(),
        Err(diag::DiagError::FeatureDisabled) => {
            err(StatusCode::FORBIDDEN, "该功能已被管理员关闭")
        }
        Err(diag::DiagError::AgentOffline) => err(StatusCode::CONFLICT, "节点当前离线"),
        Err(diag::DiagError::BadTarget) => {
            err(StatusCode::UNPROCESSABLE_ENTITY, "目标地址不合法")
        }
        Err(diag::DiagError::TooManyRequests) => {
            err(StatusCode::TOO_MANY_REQUESTS, "诊断请求过于频繁，请稍后再试")
        }
    }
}

#[derive(Debug, Deserialize)]
struct LgRequest {
    agent_id: i64,
    kind: String,
    target: String,
}

async fn diag_lg(State(state): State<SharedState>, Json(body): Json<LgRequest>) -> Response {
    diag_response(diag::start_lg(&state, body.agent_id, &body.kind, &body.target))
}

#[derive(Debug, Deserialize)]
struct MtrRequest {
    agent_id: i64,
    target: String,
    cycles: Option<u32>,
}

async fn diag_mtr(State(state): State<SharedState>, Json(body): Json<MtrRequest>) -> Response {
    diag_response(diag::start_mtr(&state, body.agent_id, &body.target, body.cycles))
}
