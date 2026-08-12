//! Public read-only API plus the diagnostics trigger endpoints.

use crate::state::SharedState;
use crate::{db, diag, features, regions};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

pub fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// Best-effort client identifier for anti-abuse budgets and iperf3 logging.
///
/// A CDN or reverse proxy in front of the site makes the peer socket its own
/// address, so the real visitor IP must come from its forwarding headers. To
/// keep those headers from being spoofable by anyone who reaches the origin
/// directly, they are only trusted when the peer address is in the trusted
/// proxy list (Cloudflare ranges by default, extendable via the
/// `trusted_proxies` setting). Otherwise the peer address itself is used.
pub fn client_ip(state: &SharedState, req: &Request) -> String {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip());
    match peer {
        Some(peer) if is_trusted_proxy(state, &peer) => {
            forwarded_client_ip(req).unwrap_or_else(|| peer.to_string())
        }
        Some(peer) => peer.to_string(),
        None => forwarded_client_ip(req).unwrap_or_else(|| "unknown".into()),
    }
}

/// The real visitor IP as reported by the CDN/proxy forwarding headers.
fn forwarded_client_ip(req: &Request) -> Option<String> {
    for header in ["cf-connecting-ip", "x-real-ip", "true-client-ip"] {
        if let Some(value) = req.headers().get(header).and_then(|v| v.to_str().ok()) {
            let value = value.trim();
            if value.parse::<std::net::IpAddr>().is_ok() {
                return Some(value.to_string());
            }
        }
    }
    if let Some(xff) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        for hop in xff.split(',') {
            let hop = hop.trim();
            if hop.parse::<std::net::IpAddr>().is_ok() {
                return Some(hop.to_string());
            }
        }
    }
    None
}

/// Cloudflare's published edge ranges — the default trusted set. Kept in sync
/// manually; operators can extend or fully replace it via `trusted_proxies`.
const CLOUDFLARE_IPS: &[&str] = &[
    "173.245.48.0/20", "103.21.244.0/22", "103.22.200.0/22", "103.31.4.0/22",
    "141.101.64.0/18", "108.162.192.0/18", "190.93.240.0/20", "188.114.96.0/20",
    "197.234.240.0/22", "198.41.128.0/17", "162.158.0.0/15", "104.16.0.0/13",
    "104.24.0.0/14", "172.64.0.0/13", "131.0.72.0/22",
    "2400:cb00::/32", "2606:4700::/32", "2803:f800::/32", "2405:b500::/32",
    "2405:8100::/32", "2a06:98c0::/29", "2c0f:f248::/32",
];

struct NetRule {
    base: std::net::IpAddr,
    prefix: u8,
}

fn parse_rule(spec: &str) -> Option<NetRule> {
    let (ip, prefix) = match spec.split_once('/') {
        Some((ip, p)) => (ip.trim(), p.trim().parse::<u8>().ok()?),
        None => (spec.trim(), u8::MAX),
    };
    let ip = ip.parse::<std::net::IpAddr>().ok()?;
    let default_prefix = match ip {
        std::net::IpAddr::V4(_) => 32,
        std::net::IpAddr::V6(_) => 128,
    };
    Some(NetRule {
        base: ip,
        prefix: if spec.contains('/') { prefix } else { default_prefix },
    })
}

/// Public validation so the settings UI/API can reject a bad proxy list.
pub fn is_valid_proxy_spec(spec: &str) -> bool {
    parse_rule(spec).is_some()
}

fn ip_in_rule(ip: &std::net::IpAddr, rule: &NetRule) -> bool {
    match (*ip, rule.base) {
        (std::net::IpAddr::V4(a), std::net::IpAddr::V4(b)) => {
            let prefix = rule.prefix.min(32);
            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
            (u32::from(a) & mask) == (u32::from(b) & mask)
        }
        (std::net::IpAddr::V6(a), std::net::IpAddr::V6(b)) => {
            let prefix = rule.prefix.min(128);
            let (a, b) = (a.octets(), b.octets());
            for i in 0..(prefix / 8) as usize {
                if a[i] != b[i] {
                    return false;
                }
            }
            let bits = (prefix % 8) as usize;
            if bits > 0 {
                let mask = 0xFFu8 << (8 - bits);
                if a[prefix as usize / 8] & mask != b[prefix as usize / 8] & mask {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

fn is_trusted_proxy(state: &SharedState, peer: &std::net::IpAddr) -> bool {
    // A reverse proxy running on the same host (Caddy/Nginx forwarding to
    // pharus on 127.0.0.1) is indistinguishable from a local client at the
    // TCP level, and only local processes can reach the loopback interface,
    // so its forwarding headers are trusted.
    if peer.is_loopback() {
        return true;
    }
    let configured = {
        let conn = state.db.lock().unwrap();
        crate::db::get_setting(&conn, "trusted_proxies")
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    // Trusted set = Cloudflare's edge ranges ∪ operator-configured proxies.
    let mut specs: Vec<&str> = CLOUDFLARE_IPS.to_vec();
    for spec in configured.split(',') {
        let spec = spec.trim();
        if !spec.is_empty() {
            specs.push(spec);
        }
    }
    specs.iter().filter_map(|s| parse_rule(s)).any(|rule| ip_in_rule(peer, &rule))
}

async fn take_json<T: serde::de::DeserializeOwned>(
    state: &SharedState,
    req: Request,
) -> Result<(String, T), Response> {
    let ip = client_ip(state, &req);
    let (_, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid request body"))?;
    let parsed = serde_json::from_slice(&bytes)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid request body"))?;
    Ok((ip, parsed))
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
        .route("/api/diag/iperf3", post(diag_iperf3))
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
    let (theme, enabled, expiry_days, site_name, site_url, default_language, agent_order, region_order, admin_enabled) = {
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
        let agent_order = get("agent_order")
            .and_then(|v| serde_json::from_str::<Vec<i64>>(&v).ok());
        let region_order = get("region_order")
            .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok());
        let admin_enabled = db::count_users(&conn).unwrap_or(0) > 0;
        (
            theme,
            enabled,
            expiry_days,
            site_name,
            site_url,
            default_language,
            agent_order,
            region_order,
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
        "agent_order": agent_order,
        "region_order": region_order,
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
    let history = {
        let conn = state.db.lock().unwrap();
        match db::list_streaming_history(&conn, agent_id, now() - 7 * 86_400, 200) {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    };
    Json(serde_json::json!({ "results": results, "history": history })).into_response()
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
        Err(diag::DiagError::RateLimited) => {
            err(StatusCode::TOO_MANY_REQUESTS, "请求频率超限，请稍后再试")
        }
    }
}

#[derive(Debug, Deserialize)]
struct LgRequest {
    agent_id: i64,
    kind: String,
    target: String,
}

async fn diag_lg(State(state): State<SharedState>, req: Request) -> Response {
    let (ip, body) = match take_json::<LgRequest>(&state, req).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    diag_response(diag::start_lg(&state, &ip, body.agent_id, &body.kind, &body.target))
}

#[derive(Debug, Deserialize)]
struct MtrRequest {
    agent_id: i64,
    target: String,
    cycles: Option<u32>,
}

async fn diag_mtr(State(state): State<SharedState>, req: Request) -> Response {
    let (ip, body) = match take_json::<MtrRequest>(&state, req).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    diag_response(diag::start_mtr(&state, &ip, body.agent_id, &body.target, body.cycles))
}

#[derive(Debug, Deserialize)]
struct Iperf3Request {
    agent_id: i64,
    server: String,
    port: Option<u16>,
    direction: Option<String>,
    duration: Option<u32>,
    parallel: Option<u32>,
    protocol: Option<String>,
    length: Option<u32>,
}

async fn diag_iperf3(State(state): State<SharedState>, req: Request) -> Response {
    let (ip, body) = match take_json::<Iperf3Request>(&state, req).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    // The target still passes through diag::valid_target, which rejects shell
    // metacharacters and leading dashes, so a user-supplied address cannot
    // become argument or command injection.
    let direction = body.direction.unwrap_or_else(|| "down".into());
    if direction != "down" && direction != "up" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "direction 必须为 up 或 down");
    }
    let protocol = body.protocol.unwrap_or_else(|| "tcp".into());
    if protocol != "tcp" && protocol != "udp" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "protocol 必须为 tcp 或 udp");
    }
    let duration = body.duration.unwrap_or(10).clamp(1, 15);
    let parallel = body.parallel.unwrap_or(4).clamp(1, 16);
    let length = body.length.map(|l| l.clamp(1, 1_048_576));
    let port = body.port.unwrap_or(5201);
    diag_response(diag::start_iperf3(
        &state,
        &ip,
        body.agent_id,
        &body.server,
        port,
        &direction,
        duration,
        parallel,
        &protocol,
        length,
    ))
}
