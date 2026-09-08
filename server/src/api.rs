//! Public read-only API plus the diagnostics trigger endpoints.

use crate::state::SharedState;
use crate::{db, diag, features, regions, speedtest};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use futures_util::StreamExt;
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
    // The speedtest endpoints are fetched cross-origin when the page is served
    // through a CDN hostname but the test targets the direct server URL.
    let speedtest_cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let speedtest_routes = Router::new()
        .route("/api/speedtest/download", get(speedtest_download))
        .route("/api/speedtest/upload", post(speedtest_upload))
        .route("/api/speedtest/ping", get(speedtest_ping))
        .layer(speedtest_cors);

    Router::new()
        .route("/api/meta", get(meta))
        .route("/api/regions", get(region_list))
        .route("/api/agents/:id/history", get(history))
        .route("/api/agents/:id/ping", get(ping_history))
        .route("/api/agents/:id/streaming", get(streaming))
        .route("/api/diag/lg", post(diag_lg))
        .route("/api/diag/mtr", post(diag_mtr))
        .route("/api/diag/iperf3", post(diag_iperf3))
        .route("/api/diag/speedtest", post(diag_speedtest))
        .route("/api/agents/:id/ping-visitor", post(ping_visitor))
        .route("/api/server-ip", get(server_ip))
        .merge(speedtest_routes)
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
    crate::state::now()
}

/// Derive a family-pinned base URL from the speedtest base by swapping the
/// hostname's `v4.` / `v6.` label (those subdomains carry A-only / AAAA-only
/// DNS records, forcing the client onto the chosen address family). Handles
/// both a bare host (prefix added) and an already family-pinned host (label
/// replaced). Bare-IP bases have no family subdomains and yield `None`.
fn family_base(base: &str, family: &str) -> Option<String> {
    let (scheme, rest) = base.split_once("://")?;
    let (host_port, suffix) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, String::new()),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (host_port, None),
    };
    // Bracketed IPv6 literals: strip the brackets before parsing.
    let host_bare = host.trim_matches(['[', ']']);
    if host_bare.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let stripped = host
        .strip_prefix("v4.")
        .or_else(|| host.strip_prefix("v6."))
        .unwrap_or(host);
    let mut out = format!("{scheme}://{family}.{stripped}");
    if let Some(p) = port {
        out.push(':');
        out.push_str(p);
    }
    out.push_str(&suffix);
    Some(out)
}

/// Return the direct speedtest base URL for CDN bypass.
/// Priority: `speedtest_base` setting (full URL, e.g. https://direct.example.com)
/// > `server_ip` setting (plain IP, joined as http://ip:8080) > none (same-origin).
async fn server_ip(
    State(state): State<SharedState>,
    _req: Request,
) -> Json<serde_json::Value> {
    let (base, ip) = {
        let conn = state.db.lock().unwrap();
        let base = db::get_setting(&conn, "speedtest_base")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let ip = db::get_setting(&conn, "server_ip")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        (base, ip)
    };

    // speedtest_base wins; fall back to http://<server_ip>:8080
    let url = base.clone().or_else(|| ip.map(|ip| format!("http://{ip}:8080")));
    let (v4, v6) = match url.as_deref() {
        Some(u) => (family_base(u, "v4"), family_base(u, "v6")),
        None => (None, None),
    };

    // Computed live so the frontend can disable the test as soon as the
    // monthly quota runs out.
    let quota_exceeded = {
        let conn = state.db.lock().unwrap();
        speedtest::quota_exceeded(&conn).unwrap_or(false)
    };

    Json(serde_json::json!({
        "base": url,
        "base_v4": v4,
        "base_v6": v6,
        "quota_exceeded": quota_exceeded
    }))
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
        "platform": crate::updates::server_platform(),
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

/// Latency probe used by the browser speedtest: the agent pings the VISITOR's
/// own public IP (taken from the request, never from a parameter, so the
/// endpoint cannot be turned into a third-party probe) and the RTT is
/// returned synchronously. Route is symmetric for our purposes: it reads as
/// "visitor ↔ this node" latency.
async fn ping_visitor(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    req: Request,
) -> Response {
    let ip = client_ip(&state, &req);
    // Only public unicast addresses may be pinged — private/loopback/link-
    // local/multicast would measure nothing meaningful and could poke at
    // internal networks behind the node.
    let Ok(addr) = ip.parse::<std::net::IpAddr>() else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "无法确定访客地址");
    };
    fn public_v4(v4: &std::net::Ipv4Addr) -> bool {
        let o = v4.octets();
        !(v4.is_private() || v4.is_loopback() || v4.is_link_local()
            || v4.is_broadcast() || v4.is_multicast() || v4.is_unspecified()
            // CGNAT 100.64.0.0/10
            || (o[0] == 100 && (o[1] & 0xC0) == 64)
            // 0.0.0.0/8 "this network", 240.0.0.0/4 reserved
            || o[0] == 0 || o[0] >= 240
            // documentation 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
            || (o[0] == 192 && o[1] == 0 && o[2] == 2)
            || (o[0] == 198 && o[1] == 51 && o[2] == 100)
            || (o[0] == 203 && o[1] == 0 && o[2] == 113)
            // benchmarking 198.18.0.0/15
            || (o[0] == 198 && (o[1] & 0xFE) == 18))
    }
    let public = match addr {
        std::net::IpAddr::V4(ref v4) => public_v4(v4),
        std::net::IpAddr::V6(ref v6) => {
            // An IPv4-mapped IPv6 address routes as IPv4 — validate it as one.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                public_v4(&mapped)
            } else {
                let s = v6.segments();
                !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                    // ULA fc00::/7, link-local fe80::/10
                    || (s[0] & 0xfe00) == 0xfc00
                    || (s[0] & 0xffc0) == 0xfe80
                    // documentation 2001:db8::/32
                    || (s[0] == 0x2001 && s[1] == 0x0db8)
                    // NAT64 64:ff9b::/96, discard-only 100::/64
                    || (s[0] == 0x0064 && s[1] == 0xff9b)
                    || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0))
            }
        }
    };
    if !public {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "访客地址不可用于延迟探测");
    }
    match diag::ping_visitor(&state, &ip, agent_id, &ip).await {
        Ok(outcome) => Json(serde_json::json!({
            "rtt_min_ms": outcome.rtt_min_ms,
            "rtt_avg_ms": outcome.rtt_avg_ms,
            "reachable": outcome.rtt_min_ms.is_some(),
        }))
        .into_response(),
        Err(diag::DiagError::FeatureDisabled) => err(StatusCode::FORBIDDEN, "该功能已被管理员关闭"),
        Err(diag::DiagError::AgentOffline) => err(StatusCode::CONFLICT, "节点当前离线"),
        Err(diag::DiagError::BadTarget) => err(StatusCode::UNPROCESSABLE_ENTITY, "目标地址不合法"),
        Err(diag::DiagError::TooManyRequests) => {
            err(StatusCode::TOO_MANY_REQUESTS, "诊断请求过于频繁，请稍后再试")
        }
        Err(diag::DiagError::RateLimited) => {
            err(StatusCode::TOO_MANY_REQUESTS, "请求频率超限，请稍后再试")
        }
    }
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

#[derive(Debug, Deserialize)]
struct SpeedtestRequest {
    agent_id: i64,
    size: Option<u64>,
    direction: Option<String>,
}

async fn diag_speedtest(State(state): State<SharedState>, req: Request) -> Response {
    let (ip, body) = match take_json::<SpeedtestRequest>(&state, req).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let size = body.size.unwrap_or(10_485_760).clamp(1_048_576, 104_857_600);
    let direction = body.direction.unwrap_or_else(|| "down".into());
    if direction != "down" && direction != "up" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "direction 必须为 up 或 down");
    }
    diag_response(diag::start_speedtest(&state, &ip, body.agent_id, size, &direction))
}

/// Admission control shared by the speedtest data endpoints: monthly quota
/// (fail closed when unreadable), per-IP hourly budget, global concurrency
/// cap. On success returns the in-flight permit the caller must hold for the
/// duration of the transfer.
fn speedtest_admission(
    state: &SharedState,
    req: &Request,
) -> Result<speedtest::InflightPermit, Response> {
    let exceeded = {
        let conn = state.db.lock().unwrap();
        speedtest::quota_exceeded(&conn)
    };
    match exceeded {
        Ok(true) => return Err(err(StatusCode::FORBIDDEN, "quota_exceeded")),
        Ok(false) => {}
        Err(e) => {
            tracing::error!(error = %e, "speedtest quota check failed");
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "quota check failed"));
        }
    }
    let ip = client_ip(state, req);
    if !speedtest::check_ip_budget(state, &ip) {
        return Err(err(StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    }
    speedtest::acquire_inflight(state)
        .ok_or_else(|| err(StatusCode::TOO_MANY_REQUESTS, "too many concurrent tests"))
}

/// Download endpoint for built-in speedtest: streams random data to the agent.
async fn speedtest_download(State(state): State<SharedState>, req: Request) -> Response {
    let permit = match speedtest_admission(&state, &req) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let size: u64 = req
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "size")
                .and_then(|(_, v)| v.parse().ok())
        })
        .unwrap_or(10_485_760)
        .clamp(1_048_576, 104_857_600);

    // Actually-sent bytes are flushed to the monthly counter when the body
    // stream ends — completed, aborted by the client, or errored.
    struct FlushOnDrop {
        state: SharedState,
        sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
        _permit: speedtest::InflightPermit,
    }
    impl Drop for FlushOnDrop {
        fn drop(&mut self) {
            speedtest::add_bytes(
                &self.state,
                self.sent.load(std::sync::atomic::Ordering::Relaxed),
            );
        }
    }
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let flush = FlushOnDrop {
        state: state.clone(),
        sent: sent.clone(),
        _permit: permit,
    };

    let stream = async_stream::stream! {
        // Owns the flush guard: it drops with the stream in every exit path.
        let _flush = flush;
        // One pre-generated random chunk, reused by reference: per-byte RNG in
        // the hot path caps throughput well below what the network can do.
        let chunk_size = 262_144u64;
        let mut buf = vec![0u8; chunk_size as usize];
        rand::Rng::fill(&mut rand::thread_rng(), &mut buf[..]);
        let chunk = axum::body::Bytes::from(buf);
        let mut remaining = size;
        while remaining > 0 {
            let len = remaining.min(chunk_size) as usize;
            remaining -= len as u64;
            sent.fetch_add(len as u64, std::sync::atomic::Ordering::Relaxed);
            yield Ok::<_, std::convert::Infallible>(chunk.slice(..len));
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", size.to_string())
        .header("Cache-Control", "no-store")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

/// Latency probe for the built-in speedtest: empty response, CORS-enabled.
async fn speedtest_ping(State(state): State<SharedState>) -> Response {
    let exceeded = {
        let conn = state.db.lock().unwrap();
        speedtest::quota_exceeded(&conn)
    };
    match exceeded {
        Ok(true) => err(StatusCode::FORBIDDEN, "quota_exceeded"),
        Ok(false) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "speedtest quota check failed");
            err(StatusCode::SERVICE_UNAVAILABLE, "quota check failed")
        }
    }
}

/// Upload endpoint for built-in speedtest: receives and discards the body.
async fn speedtest_upload(State(state): State<SharedState>, req: Request) -> Response {
    let _permit = match speedtest_admission(&state, &req) {
        Ok(p) => p,
        Err(r) => return r,
    };
    // Reject oversized bodies up front when the length is known.
    if req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|len| len > speedtest::UPLOAD_MAX_BYTES)
    {
        return err(StatusCode::PAYLOAD_TOO_LARGE, "payload too large");
    }
    let start = std::time::Instant::now();
    let body = req.into_body();
    let mut bytes_received: u64 = 0;

    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(data) => {
                bytes_received += data.len() as u64;
                // Chunked/smuggled bodies can dodge the Content-Length
                // pre-check, so enforce the cap while streaming too.
                if bytes_received > speedtest::UPLOAD_MAX_BYTES {
                    return err(StatusCode::PAYLOAD_TOO_LARGE, "payload too large");
                }
            }
            Err(_) => break,
        }
    }

    speedtest::add_bytes(&state, bytes_received);
    let elapsed_ms = start.elapsed().as_millis() as u64;
    axum::Json(serde_json::json!({
        "bytes": bytes_received,
        "elapsed_ms": elapsed_ms,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::family_base;

    #[test]
    fn bare_host_gets_family_prefix() {
        assert_eq!(
            family_base("https://speedtest.example.com", "v4"),
            Some("https://v4.speedtest.example.com".into())
        );
        assert_eq!(
            family_base("https://speedtest.example.com", "v6"),
            Some("https://v6.speedtest.example.com".into())
        );
    }

    #[test]
    fn family_label_is_swapped_not_double_prefixed() {
        assert_eq!(
            family_base("https://v4.speedtest.example.com", "v6"),
            Some("https://v6.speedtest.example.com".into())
        );
        assert_eq!(
            family_base("https://v6.speedtest.example.com", "v4"),
            Some("https://v4.speedtest.example.com".into())
        );
    }

    #[test]
    fn ip_literals_have_no_family_domain() {
        assert_eq!(family_base("http://82.22.31.35:8080", "v4"), None);
        assert_eq!(family_base("http://[2001:db8::1]:8080", "v6"), None);
    }
}
