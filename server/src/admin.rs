use crate::api::err;
use crate::state::SharedState;
use crate::{billing, crypto, db, diag, features, notify, regions, themes};
use axum::{
    extract::{Extension, Multipart, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post, put},
    Router,
};
use chrono::TimeZone;
use pharus_common::{
    AgentMsg, BillingCycle, BillingInfo, BrowserMsg, Currency, ServerToAgentMsg, TaskKind,
    TrafficUsage,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const CHANNEL_KINDS: &[&str] = &[
    "telegram", "webhook", "email", "bark", "feishu", "dingtalk", "wecom", "discord",
];
const ALERT_METRICS: &[&str] = &["cpu", "mem", "disk", "load", "traffic", "loss"];

pub fn router(state: SharedState) -> Router<SharedState> {
    let protected = Router::new()
        .route("/api/admin/check", post(check))
        .route("/api/admin/agents", get(list_agents))
        .route("/api/admin/agents/:id/billing", put(update_billing))
        .route("/api/admin/agents/:id/name", put(rename_agent))
        .route("/api/admin/agents/:id/region", put(update_region))
        .route(
            "/api/admin/agents/:id/features",
            get(agent_features).put(set_agent_features),
        )
        .route(
            "/api/admin/features",
            get(global_features).put(set_global_features),
        )
        .route(
            "/api/admin/ping-tasks",
            get(list_ping_tasks).post(create_ping_task),
        )
        .route(
            "/api/admin/ping-tasks/:id",
            put(update_ping_task).delete(delete_ping_task),
        )
        .route("/api/admin/tasks", get(list_tasks).post(create_task))
        .route("/api/admin/tasks/:id", put(update_task).delete(delete_task))
        .route("/api/admin/tasks/:id/run", post(run_task))
        .route("/api/admin/task-results", get(task_results))
        .route(
            "/api/admin/alert-rules",
            get(list_alert_rules).post(create_alert_rule),
        )
        .route(
            "/api/admin/alert-rules/:id",
            put(update_alert_rule).delete(delete_alert_rule),
        )
        .route("/api/admin/channels", get(list_channels).post(create_channel))
        .route(
            "/api/admin/channels/:id",
            put(update_channel).delete(delete_channel),
        )
        .route("/api/admin/channels/:id/test", post(test_channel))
        .route("/api/admin/settings", put(update_setting))
        .route(
            "/api/admin/agent-secrets",
            get(list_agent_secrets).put(save_agent_secrets),
        )
        .route("/api/admin/password", put(update_password))
        .route("/api/admin/themes", get(list_themes).post(upload_theme))
        .route(
            "/api/admin/themes/:id/activate",
            post(activate_theme),
        )
        .route("/api/admin/themes/:id", delete(delete_theme))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin))
        // Theme zips can legitimately exceed axum's default 2 MiB body cap;
        // the upload path caps total bytes itself and extraction is bounded.
        .route_layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 4096));
    Router::new()
        .route("/api/admin/login", post(login))
        .route("/api/admin/logout", post(logout))
        .merge(protected)
}

#[derive(Clone)]
struct AdminSession(String);

const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// HttpOnly session cookie. Browsers authenticate with it instead of a Bearer
/// header, so third-party theme code (arbitrary JS) can never read the token.
const SESSION_COOKIE: &str = "pharus_admin";

/// True when the request arrived over TLS directly or via a reverse proxy that
/// sets `X-Forwarded-Proto` (Caddy/Nginx). `Secure` cookies are only sent by
/// the browser over HTTPS, so enabling it on plaintext deployments would lock
/// admins out.
fn request_is_secure(req: &Request) -> bool {
    req.uri().scheme_str() == Some("https")
        || req
            .headers()
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

fn set_session_cookie(response: &mut Response, token: &str, secure: bool) {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    response
        .headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
}

fn clear_session_cookie(response: &mut Response, secure: bool) {
    let mut cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    response
        .headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
}

/// Prefer the HttpOnly cookie, fall back to a Bearer token for programmatic
/// clients (curl, scripts).
fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(raw) = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        let prefix = format!("{SESSION_COOKIE}=");
        for pair in raw.split(';') {
            if let Some(value) = pair.trim().strip_prefix(&prefix) {
                return Some(value.to_string());
            }
        }
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

async fn require_admin(State(state): State<SharedState>, mut req: Request, next: Next) -> Response {
    let token = extract_session_token(req.headers());
    let Some(token) = token else {
        return err(StatusCode::UNAUTHORIZED, "missing bearer token");
    };
    let username = {
        let mut sessions = state.sessions.lock().unwrap();
        let Some((username, created)) = sessions.get(&token) else {
            return err(StatusCode::UNAUTHORIZED, "invalid session");
        };
        if created.elapsed() > SESSION_TTL {
            sessions.remove(&token);
            return err(StatusCode::UNAUTHORIZED, "session expired");
        }
        username.clone()
    };
    req.extensions_mut().insert(AdminSession(username));
    next.run(req).await
}

pub fn hash_password(pw: &str) -> anyhow::Result<String> {
    use argon2::password_hash::rand_core::OsRng;
    use argon2::password_hash::SaltString;
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(anyhow::Error::msg)?
        .to_string())
}

fn verify_password(pw: &str, hash: &str) -> bool {
    use argon2::password_hash::PasswordHash;
    use argon2::{Argon2, PasswordVerifier};
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(pw.as_bytes(), &parsed)
        .is_ok()
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

async fn login(State(state): State<SharedState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let secure = parts.uri.scheme_str() == Some("https")
        || parts
            .headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"));
    let Ok(bytes) = axum::body::to_bytes(body, 64 * 1024).await else {
        return err(StatusCode::BAD_REQUEST, "invalid request body");
    };
    let Ok(body) = serde_json::from_slice::<LoginBody>(&bytes) else {
        return err(StatusCode::BAD_REQUEST, "invalid request body");
    };
    let record = {
        let conn = state.db.lock().unwrap();
        match db::find_user(&conn, &body.username) {
            Ok(r) => r,
            Err(e) => return db_err(e),
        }
    };
    let Some((_id, hash)) = record else {
        return err(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    if !verify_password(&body.password, &hash) {
        return err(StatusCode::UNAUTHORIZED, "invalid credentials");
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(token.clone(), (body.username.clone(), std::time::Instant::now()));
    let mut response = Json(serde_json::json!({ "token": token })).into_response();
    set_session_cookie(&mut response, &token, secure);
    response
}

async fn logout(State(state): State<SharedState>, req: Request) -> Response {
    let token = extract_session_token(req.headers());
    if let Some(token) = token {
        state.sessions.lock().unwrap().remove(&token);
    }
    let mut response = StatusCode::OK.into_response();
    clear_session_cookie(&mut response, request_is_secure(&req));
    response
}

#[derive(Debug, Deserialize)]
pub struct PasswordBody {
    pub old_password: String,
    pub new_password: String,
}

async fn update_password(
    State(state): State<SharedState>,
    Extension(session): Extension<AdminSession>,
    Json(body): Json<PasswordBody>,
) -> Response {
    let username = session.0;
    let hash = {
        let conn = state.db.lock().unwrap();
        match db::find_user(&conn, &username) {
            Ok(Some((_, h))) => h,
            Ok(None) => return err(StatusCode::UNAUTHORIZED, "user not found"),
            Err(e) => return db_err(e),
        }
    };
    if !verify_password(&body.old_password, &hash) {
        return err(StatusCode::UNAUTHORIZED, "invalid current password");
    }
    if body.new_password.is_empty() {
        return bad("new password must not be empty");
    }
    let new_hash = match hash_password(&body.new_password) {
        Ok(h) => h,
        Err(e) => return db_err(e),
    };
    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::update_user_password(&conn, &username, &new_hash) {
            return db_err(e);
        }
    }
    StatusCode::OK.into_response()
}

#[derive(Debug, Deserialize)]
pub struct SettingUpdate {
    pub key: String,
    pub value: String,
}

async fn update_setting(
    State(state): State<SharedState>,
    Json(body): Json<SettingUpdate>,
) -> Response {
    match body.key.as_str() {
        "expiry_alert_days" => match body.value.parse::<i64>() {
            Ok(v) if (1..=365).contains(&v) => {}
            _ => {
                return bad("expiry_alert_days must be a number within 1..=365");
            }
        }
        "default_language" => {
            if !matches!(body.value.as_str(), "en" | "zh-CN" | "ja" | "ru") {
                return bad("default_language must be en, zh-CN, ja or ru");
            }
        }
        "agent_order" | "region_order" | "ping_task_order" => {
            match serde_json::from_str::<Vec<serde_json::Value>>(&body.value) {
                Ok(v) if v.len() <= 4096 => {}
                _ => return bad("order must be a JSON array of at most 4096 entries"),
            }
        }
        "agent_secret" => {
            if body.value.trim().is_empty() {
                return bad("agent_secret must not be empty");
            }
        }
        "site_name" | "site_url" => {}
        _ => return bad("unknown setting key"),
    }
    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::set_setting(&conn, &body.key, &body.value) {
            return db_err(e);
        }
    }
    // the task order lives in a setting but drives what agents run and show
    if body.key == "ping_task_order" {
        state.push_tasks_all();
    }
    StatusCode::OK.into_response()
}

/// Session probe for the frontend: reaching this handler means the bearer
/// token is a valid admin session.
async fn check() -> StatusCode {
    StatusCode::OK
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSecretEntry {
    pub secret: String,
    #[serde(default)]
    pub note: Option<String>,
}

pub fn load_agent_secrets(conn: &rusqlite::Connection) -> Vec<AgentSecretEntry> {
    db::get_setting(conn, "agent_secrets")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn list_agent_secrets(State(state): State<SharedState>) -> Response {
    let list = {
        let conn = state.db.lock().unwrap();
        load_agent_secrets(&conn)
    };
    Json(list).into_response()
}

async fn save_agent_secrets(
    State(state): State<SharedState>,
    Json(body): Json<Vec<AgentSecretEntry>>,
) -> Response {
    let mut cleaned: Vec<AgentSecretEntry> = Vec::new();
    for entry in body {
        let secret = entry.secret.trim().to_string();
        if secret.is_empty() {
            return bad("secret must not be empty");
        }
        if secret.len() < 6 {
            return bad("secret must be at least 6 characters");
        }
        if cleaned.iter().any(|e: &AgentSecretEntry| e.secret == secret) {
            return bad("duplicate secret");
        }
        cleaned.push(AgentSecretEntry {
            secret,
            note: entry.note,
        });
    }
    let json = match serde_json::to_string(&cleaned) {
        Ok(j) => j,
        Err(_) => return bad("invalid secrets"),
    };
    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::set_setting(&conn, "agent_secrets", &json) {
            return db_err(e);
        }
    }
    StatusCode::OK.into_response()
}

fn db_err(e: impl std::fmt::Display) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

fn bad(msg: &str) -> Response {
    err(StatusCode::UNPROCESSABLE_ENTITY, msg)
}

// ------------------------------------------------------------------- agents

async fn list_agents(State(state): State<SharedState>) -> Response {
    let agents = state.agents.read().unwrap();
    let mut list: Vec<_> = agents
        .iter()
        .map(|(id, a)| {
            serde_json::json!({
                "agent_id": id,
                "name": a.name,
                "online": a.online,
                "region": a.region,
                "features": a.features,
                "billing": a.billing,
            })
        })
        .collect();
    list.sort_by_key(|v| v["agent_id"].as_i64().unwrap_or(0));
    Json(list).into_response()
}

#[derive(Debug, Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_agent(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> Response {
    let name = body.name.trim();
    if name.is_empty() {
        return bad("name must not be empty");
    }
    let rows = {
        let conn = state.db.lock().unwrap();
        match db::rename_agent(&conn, agent_id, name) {
            Ok(r) => r,
            Err(e) => return db_err(e),
        }
    };
    if rows == 0 {
        return bad("agent not found");
    }
    {
        let mut agents = state.agents.write().unwrap();
        if let Some(a) = agents.get_mut(&agent_id) {
            a.name = name.to_string();
        }
    }
    StatusCode::OK.into_response()
}

#[derive(Debug, Deserialize)]
struct RegionUpdate {
    /// `None` drops the manual override and lets agent-side detection win again.
    code: Option<String>,
}

async fn update_region(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Json(body): Json<RegionUpdate>,
) -> Response {
    let region = match &body.code {
        Some(raw) => {
            let Some(code) = regions::normalize(raw) else {
                return bad("region code must be two letters");
            };
            Some(db::make_region(&code, "manual"))
        }
        None => None,
    };

    let rows = {
        let conn = state.db.lock().unwrap();
        match region.as_ref() {
            Some(r) => db::set_region(&conn, agent_id, Some(&r.code), "manual"),
            None => db::set_region(&conn, agent_id, None, "auto"),
        }
    };
    match rows {
        Ok(0) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return db_err(e),
        Ok(_) => {}
    }

    {
        let mut agents = state.agents.write().unwrap();
        if let Some(a) = agents.get_mut(&agent_id) {
            a.region = region.clone();
        }
    }
    state.broadcast(BrowserMsg::RegionUpdate {
        agent_id,
        region: region.clone(),
    });
    Json(serde_json::json!({ "region": region })).into_response()
}

// ----------------------------------------------------------------- features

async fn global_features(State(state): State<SharedState>) -> Response {
    let conn = state.db.lock().unwrap();
    match features::global_defaults(&conn) {
        Ok(m) => Json(m).into_response(),
        Err(e) => db_err(e),
    }
}

async fn set_global_features(
    State(state): State<SharedState>,
    Json(body): Json<HashMap<String, bool>>,
) -> Response {
    if let Some(k) = body.keys().find(|k| !features::is_valid(k)) {
        return bad(&format!("unknown feature: {k}"));
    }
    {
        let conn = state.db.lock().unwrap();
        for (k, v) in &body {
            if let Err(e) = features::set_global(&conn, k, *v) {
                return db_err(e);
            }
        }
    }
    features::refresh_all(&state);
    let conn = state.db.lock().unwrap();
    match features::global_defaults(&conn) {
        Ok(m) => Json(m).into_response(),
        Err(e) => db_err(e),
    }
}

async fn agent_features(State(state): State<SharedState>, Path(agent_id): Path<i64>) -> Response {
    let conn = state.db.lock().unwrap();
    let (Ok(global), Ok(overrides)) = (
        features::global_defaults(&conn),
        features::overrides_for(&conn, agent_id),
    ) else {
        return db_err("feature lookup failed");
    };
    let effective = features::merge(&global, &overrides);
    Json(serde_json::json!({
        "global": global,
        "overrides": overrides,
        "effective": effective,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct FeatureOverrides {
    /// `null` for a feature clears the override and reverts to the global default.
    overrides: HashMap<String, Option<bool>>,
}

async fn set_agent_features(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Json(body): Json<FeatureOverrides>,
) -> Response {
    if let Some(k) = body.overrides.keys().find(|k| !features::is_valid(k)) {
        return bad(&format!("unknown feature: {k}"));
    }
    {
        let conn = state.db.lock().unwrap();
        for (k, v) in &body.overrides {
            if let Err(e) = features::set_override(&conn, agent_id, k, *v) {
                return db_err(e);
            }
        }
    }
    let effective = features::refresh_agent(&state, agent_id);
    Json(serde_json::json!({ "effective": effective })).into_response()
}

// --------------------------------------------------------------- ping tasks

#[derive(Debug, Deserialize)]
struct PingTaskBody {
    /// Empty = applies to every host.
    #[serde(default)]
    agent_ids: Vec<i64>,
    label: String,
    kind: String,
    target: String,
    #[serde(default)]
    port: Option<u16>,
    interval_sec: u64,
    probe_count: u32,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

/// A ping target ends up as `ping` argv, a TCP connect host, or a reqwest URL,
/// so each kind gets the validation matching where it actually lands.
fn valid_ping_target(kind: &str, target: &str) -> bool {
    if kind == "http" {
        if let Some(rest) = target
            .strip_prefix("http://")
            .or_else(|| target.strip_prefix("https://"))
        {
            return !rest.is_empty()
                && target.len() <= 2048
                && !target.chars().any(|c| c.is_whitespace() || c.is_control());
        }
    }
    diag::valid_target(target)
}

impl PingTaskBody {
    fn validate(self) -> Result<db::PingTaskRow, String> {
        if self.label.trim().is_empty() {
            return Err("label is required".into());
        }
        if db::ping_kind_from_str(&self.kind).is_none() {
            return Err("kind must be icmp, tcp or http".into());
        }
        let target = self.target.trim().to_string();
        if !valid_ping_target(&self.kind, &target) {
            return Err("target must be a hostname, IP address or http(s) URL".into());
        }
        if self.kind == "tcp" && self.port.is_none() {
            return Err("tcp probes require a port".into());
        }
        if !(5..=86_400).contains(&self.interval_sec) {
            return Err("interval_sec must be within 5..=86400".into());
        }
        if !(1..=100).contains(&self.probe_count) {
            return Err("probe_count must be within 1..=100".into());
        }
        Ok(db::PingTaskRow {
            id: 0,
            agent_id: None,
            agent_ids: self.agent_ids,
            label: self.label,
            kind: self.kind,
            target,
            port: self.port,
            interval_sec: self.interval_sec,
            probe_count: self.probe_count,
            enabled: self.enabled,
        })
    }
}

async fn list_ping_tasks(State(state): State<SharedState>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::list_ping_tasks(&conn) {
        Ok(v) => Json(v).into_response(),
        Err(e) => db_err(e),
    }
}

async fn create_ping_task(
    State(state): State<SharedState>,
    Json(body): Json<PingTaskBody>,
) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let id = {
        let conn = state.db.lock().unwrap();
        db::insert_ping_task(&conn, &row)
    };
    match id {
        Ok(id) => {
            state.push_tasks_all();
            Json(serde_json::json!({ "id": id })).into_response()
        }
        Err(e) => db_err(e),
    }
}

async fn update_ping_task(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<PingTaskBody>,
) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let rows = {
        let conn = state.db.lock().unwrap();
        db::update_ping_task(&conn, id, &row)
    };
    match rows {
        Ok(0) => err(StatusCode::NOT_FOUND, "ping task not found"),
        Ok(_) => {
            state.push_tasks_all();
            StatusCode::OK.into_response()
        }
        Err(e) => db_err(e),
    }
}

async fn delete_ping_task(State(state): State<SharedState>, Path(id): Path<i64>) -> Response {
    let rows = {
        let conn = state.db.lock().unwrap();
        db::delete_ping_task(&conn, id)
    };
    match rows {
        Ok(0) => err(StatusCode::NOT_FOUND, "ping task not found"),
        Ok(_) => {
            state.push_tasks_all();
            StatusCode::OK.into_response()
        }
        Err(e) => db_err(e),
    }
}

// ------------------------------------------------------------- custom tasks

#[derive(Debug, Deserialize)]
struct TaskBody {
    name: String,
    command: String,
    /// Empty = applies to every host.
    #[serde(default)]
    agent_ids: Vec<i64>,
    /// 0 = manual trigger only.
    interval_sec: u64,
    timeout_sec: u64,
    #[serde(default = "yes")]
    enabled: bool,
}

impl TaskBody {
    fn validate(self) -> Result<db::TaskRow, String> {
        if self.name.trim().is_empty() {
            return Err("name is required".into());
        }
        if self.command.trim().is_empty() {
            return Err("command is required".into());
        }
        if self.interval_sec != 0 && !(10..=86_400).contains(&self.interval_sec) {
            return Err("interval_sec must be 0 or within 10..=86400".into());
        }
        if !(1..=600).contains(&self.timeout_sec) {
            return Err("timeout_sec must be within 1..=600".into());
        }
        Ok(db::TaskRow {
            id: 0,
            name: self.name,
            command: self.command,
            agent_id: None,
            agent_ids: self.agent_ids,
            interval_sec: self.interval_sec,
            timeout_sec: self.timeout_sec,
            enabled: self.enabled,
        })
    }
}

async fn list_tasks(State(state): State<SharedState>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::list_tasks(&conn) {
        Ok(v) => Json(v).into_response(),
        Err(e) => db_err(e),
    }
}

async fn create_task(State(state): State<SharedState>, Json(body): Json<TaskBody>) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let id = {
        let conn = state.db.lock().unwrap();
        db::insert_task(&conn, &row)
    };
    match id {
        Ok(id) => {
            state.push_tasks_all();
            Json(serde_json::json!({ "id": id })).into_response()
        }
        Err(e) => db_err(e),
    }
}

async fn update_task(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<TaskBody>,
) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let rows = {
        let conn = state.db.lock().unwrap();
        db::update_task(&conn, id, &row)
    };
    match rows {
        Ok(0) => err(StatusCode::NOT_FOUND, "task not found"),
        Ok(_) => {
            state.push_tasks_all();
            StatusCode::OK.into_response()
        }
        Err(e) => db_err(e),
    }
}

async fn delete_task(State(state): State<SharedState>, Path(id): Path<i64>) -> Response {
    let rows = {
        let conn = state.db.lock().unwrap();
        db::delete_task(&conn, id)
    };
    match rows {
        Ok(0) => err(StatusCode::NOT_FOUND, "task not found"),
        Ok(_) => {
            state.push_tasks_all();
            StatusCode::OK.into_response()
        }
        Err(e) => db_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct RunTaskBody {
    agent_id: i64,
}

/// Custom tasks run with the agent process's privileges, so this stays behind
/// the admin token and is never reachable from the public API.
async fn run_task(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<RunTaskBody>,
) -> Response {
    let task = {
        let conn = state.db.lock().unwrap();
        match db::get_task(&conn, id) {
            Ok(Some(t)) => t,
            Ok(None) => return err(StatusCode::NOT_FOUND, "task not found"),
            Err(e) => return db_err(e),
        }
    };
    if let Some(pinned) = task.agent_id {
        if pinned != body.agent_id {
            return bad("task is pinned to a different agent");
        }
    }
    if !features::enabled(&state, body.agent_id, "tasks") {
        return err(StatusCode::FORBIDDEN, "该节点的自定义任务功能已关闭");
    }

    let tx = {
        let agents = state.agents.read().unwrap();
        match agents.get(&body.agent_id) {
            Some(a) if a.online => a.agent_tx.clone(),
            _ => None,
        }
    };
    let Some(tx) = tx else {
        return err(StatusCode::CONFLICT, "节点当前离线");
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let (otx, orx) = tokio::sync::oneshot::channel();
    state
        .task_waiters
        .lock()
        .unwrap()
        .insert(request_id.clone(), otx);

    let sent = tx.send(ServerToAgentMsg::RunTask {
        task_id: request_id.clone(),
        kind: TaskKind::Script,
        target: task.command.clone(),
        cycles: None,
        timeout: Some(task.timeout_sec),
        extra: None,
    });
    if sent.is_err() {
        state.task_waiters.lock().unwrap().remove(&request_id);
        return err(StatusCode::CONFLICT, "节点当前离线");
    }

    let budget = Duration::from_secs(task.timeout_sec.clamp(1, 600) + 10);
    let reply = tokio::time::timeout(budget, orx).await;
    let (exit_code, output) = match reply {
        Ok(Ok(AgentMsg::TaskResult {
            exit_code, output, ..
        })) => (exit_code, output),
        _ => {
            state.task_waiters.lock().unwrap().remove(&request_id);
            return err(StatusCode::GATEWAY_TIMEOUT, "任务执行超时");
        }
    };

    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::insert_task_result(&conn, id, body.agent_id, exit_code, &output) {
            tracing::warn!(task_id = id, error = %e, "manual task result insert failed");
        }
    }
    Json(serde_json::json!({ "exit_code": exit_code, "output": output })).into_response()
}

#[derive(Debug, Deserialize)]
struct ResultsQuery {
    #[serde(default)]
    task_id: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

async fn task_results(
    State(state): State<SharedState>,
    Query(q): Query<ResultsQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let conn = state.db.lock().unwrap();
    match db::list_task_results(&conn, q.task_id, limit) {
        Ok(v) => Json(v).into_response(),
        Err(e) => db_err(e),
    }
}

// -------------------------------------------------------------- alert rules

#[derive(Debug, Deserialize)]
struct AlertRuleBody {
    name: String,
    kind: String,
    #[serde(default)]
    agent_id: Option<i64>,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default = "gt")]
    op: String,
    threshold: f64,
    duration: i64,
    #[serde(default = "one")]
    ratio: f64,
    #[serde(default)]
    channels: Vec<i64>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "default_cooldown")]
    cooldown: i64,
    #[serde(default)]
    task_id: Option<i64>,
    #[serde(default = "default_consecutive")]
    consecutive: i32,
}

fn default_cooldown() -> i64 {
    1800
}

fn default_consecutive() -> i32 {
    1
}

fn gt() -> String {
    ">".into()
}

fn one() -> f64 {
    1.0
}

impl AlertRuleBody {
    fn validate(self) -> Result<db::AlertRuleRow, String> {
        if self.name.trim().is_empty() {
            return Err("name is required".into());
        }
        if !matches!(self.kind.as_str(), "metric" | "offline" | "task") {
            return Err("kind must be metric, offline or task".into());
        }
        if self.op != ">" && self.op != "<" {
            return Err("op must be > or <".into());
        }
        if !self.threshold.is_finite() {
            return Err("threshold must be a number".into());
        }
        if !(0.0..=1.0).contains(&self.ratio) {
            return Err("ratio must be within 0..=1".into());
        }
        if !(0..=86_400).contains(&self.cooldown) {
            return Err("cooldown must be within 0..=86400 seconds".into());
        }
        if !(1..=100).contains(&self.consecutive) {
            return Err("consecutive must be within 1..=100".into());
        }
        if !(30..=86_400).contains(&self.duration) {
            return Err("duration must be within 30..=86400 seconds".into());
        }
        let metric = match self.kind.as_str() {
            "metric" => match self.metric.as_deref() {
                Some(m) if ALERT_METRICS.contains(&m) => Some(m.to_string()),
                _ => return Err(format!("metric must be one of {}", ALERT_METRICS.join(", "))),
            },
            _ => None,
        };
        let task_id = if self.kind == "task" { self.task_id } else { None };
        Ok(db::AlertRuleRow {
            id: 0,
            name: self.name,
            kind: self.kind,
            agent_id: self.agent_id,
            metric,
            op: self.op,
            threshold: self.threshold,
            duration: self.duration,
            ratio: self.ratio,
            channels: self.channels,
            enabled: self.enabled,
            cooldown: self.cooldown,
            task_id,
            consecutive: self.consecutive,
        })
    }
}

async fn list_alert_rules(State(state): State<SharedState>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::list_alert_rules(&conn) {
        Ok(v) => Json(v).into_response(),
        Err(e) => db_err(e),
    }
}

async fn create_alert_rule(
    State(state): State<SharedState>,
    Json(body): Json<AlertRuleBody>,
) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let conn = state.db.lock().unwrap();
    match db::insert_alert_rule(&conn, &row) {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => db_err(e),
    }
}

async fn update_alert_rule(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<AlertRuleBody>,
) -> Response {
    let row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let conn = state.db.lock().unwrap();
    match db::update_alert_rule(&conn, id, &row) {
        Ok(0) => err(StatusCode::NOT_FOUND, "alert rule not found"),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => db_err(e),
    }
}

async fn delete_alert_rule(State(state): State<SharedState>, Path(id): Path<i64>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::delete_alert_rule(&conn, id) {
        Ok(0) => err(StatusCode::NOT_FOUND, "alert rule not found"),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => db_err(e),
    }
}

// ---------------------------------------------------------------- channels

#[derive(Debug, Deserialize)]
struct ChannelBody {
    name: String,
    kind: String,
    #[serde(default)]
    config: serde_json::Value,
    #[serde(default = "yes")]
    enabled: bool,
}

impl ChannelBody {
    fn validate(self) -> Result<db::ChannelRow, String> {
        if self.name.trim().is_empty() {
            return Err("name is required".into());
        }
        if !CHANNEL_KINDS.contains(&self.kind.as_str()) {
            return Err(format!("kind must be one of {}", CHANNEL_KINDS.join(", ")));
        }
        if !self.config.is_object() {
            return Err("config must be a JSON object".into());
        }
        Ok(db::ChannelRow {
            id: 0,
            name: self.name,
            kind: self.kind,
            config: self.config,
            enabled: self.enabled,
            failed_streak: 0,
        })
    }
}

/// A secret field the client sends back as the redaction placeholder means
/// "unchanged", so restore the stored ciphertext instead of overwriting it.
fn keep_unchanged_secrets(existing: &serde_json::Value, incoming: &mut serde_json::Value) {
    let Some(map) = incoming.as_object_mut() else {
        return;
    };
    for field in crypto::SECRET_FIELDS {
        if map.get(*field).and_then(|v| v.as_str()) != Some("***") {
            continue;
        }
        match existing.get(*field) {
            Some(v) => map.insert((*field).to_string(), v.clone()),
            None => map.remove(*field),
        };
    }
}

async fn list_channels(State(state): State<SharedState>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::list_channels(&conn) {
        Ok(v) => {
            let masked: Vec<_> = v
                .into_iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "name": c.name,
                        "kind": c.kind,
                        "config": crypto::redact(&c.config),
                        "enabled": c.enabled,
                        "failed_streak": c.failed_streak,
                    })
                })
                .collect();
            Json(masked).into_response()
        }
        Err(e) => db_err(e),
    }
}

async fn create_channel(
    State(state): State<SharedState>,
    Json(body): Json<ChannelBody>,
) -> Response {
    let mut row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let conn = state.db.lock().unwrap();
    if let Err(e) = crypto::seal(&conn, &mut row.config) {
        return db_err(e);
    }
    match db::insert_channel(&conn, &row) {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => db_err(e),
    }
}

async fn update_channel(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<ChannelBody>,
) -> Response {
    let mut row = match body.validate() {
        Ok(r) => r,
        Err(m) => return bad(&m),
    };
    let conn = state.db.lock().unwrap();
    match db::get_channel(&conn, id) {
        Ok(Some(existing)) => keep_unchanged_secrets(&existing.config, &mut row.config),
        Ok(None) => return err(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => return db_err(e),
    }
    if let Err(e) = crypto::seal(&conn, &mut row.config) {
        return db_err(e);
    }
    match db::update_channel(&conn, id, &row) {
        Ok(0) => err(StatusCode::NOT_FOUND, "channel not found"),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => db_err(e),
    }
}

async fn delete_channel(State(state): State<SharedState>, Path(id): Path<i64>) -> Response {
    let conn = state.db.lock().unwrap();
    match db::delete_channel(&conn, id) {
        Ok(0) => err(StatusCode::NOT_FOUND, "channel not found"),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => db_err(e),
    }
}

async fn test_channel(State(state): State<SharedState>, Path(id): Path<i64>) -> Response {
    match notify::test_channel(&state, id).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

// ------------------------------------------------------------------ billing

#[derive(Debug, Deserialize)]
pub struct BillingUpdate {
    pub reset_day: Option<u8>,
    pub quota_gb: Option<f64>,
    pub expires_on: Option<String>,
    pub price: Option<f64>,
    pub currency: Option<Currency>,
    pub cycle: Option<BillingCycle>,
    pub bandwidth: Option<f64>,
    pub traffic_mode: Option<String>,
    pub traffic_dir: Option<String>,
}

async fn update_billing(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Json(body): Json<BillingUpdate>,
) -> Response {
    if let Some(rd) = body.reset_day {
        if !(1..=31).contains(&rd) {
            return bad("reset_day must be within 1..=31");
        }
    }
    if let Some(g) = body.quota_gb {
        if !g.is_finite() || g <= 0.0 {
            return bad("quota_gb must be positive");
        }
    }
    if let Some(p) = body.price {
        if !p.is_finite() || p < 0.0 {
            return bad("price must be >= 0");
        }
    }
    if let Some(bw) = body.bandwidth {
        if !bw.is_finite() || bw <= 0.0 {
            return bad("bandwidth must be positive (Mbps)");
        }
    }
    if let Some(mode) = &body.traffic_mode {
        if mode != "bi" && mode != "uni" {
            return bad("traffic_mode must be bi or uni");
        }
    }
    if let Some(dir) = &body.traffic_dir {
        if !matches!(dir.as_str(), "up" | "down" | "max") {
            return bad("traffic_dir must be up, down or max");
        }
    }
    let expires_at = match &body.expires_on {
        Some(s) => {
            let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") else {
                return bad("expires_on must be YYYY-MM-DD");
            };
            let Some(dt) = date.and_hms_opt(0, 0, 0) else {
                return bad("invalid expires_on");
            };
            let Some(local) = chrono::Local.from_local_datetime(&dt).earliest() else {
                return bad("invalid expires_on");
            };
            Some(local.timestamp())
        }
        None => None,
    };

    let billing = BillingInfo {
        reset_day: body.reset_day,
        quota_bytes: body.quota_gb.map(|g| (g * 1_073_741_824.0) as u64),
        expires_at,
        price: body.price,
        currency: body.currency,
        cycle: body.cycle,
        bandwidth: body.bandwidth,
        traffic_mode: body.traffic_mode,
        traffic_dir: body.traffic_dir,
    };

    let rows = {
        let conn = state.db.lock().unwrap();
        match db::set_billing(&conn, agent_id, &billing) {
            Ok(r) => r,
            Err(e) => return db_err(e),
        }
    };
    if rows == 0 {
        return err(StatusCode::NOT_FOUND, "agent not found");
    }

    let traffic = {
        let mut agents = state.agents.write().unwrap();
        agents.get_mut(&agent_id).map(|a| {
            let old_reset = a.billing.as_ref().and_then(|b| b.reset_day);
            if billing.reset_day != old_reset {
                if let Some(rd) = billing.reset_day {
                    // changing the reset day starts a fresh cycle
                    a.traffic.cycle_start = billing::cycle_start_for(rd, chrono::Local::now());
                    a.traffic.rx_bytes = 0;
                    a.traffic.tx_bytes = 0;
                    a.traffic.last_rx_total = None;
                    a.traffic.last_tx_total = None;
                }
            }
            a.billing = Some(billing.clone());
            (
                TrafficUsage {
                    cycle_start: a.traffic.cycle_start,
                    rx_bytes: a.traffic.rx_bytes,
                    tx_bytes: a.traffic.tx_bytes,
                },
                db::TrafficRow {
                    cycle_start: a.traffic.cycle_start,
                    rx_bytes: a.traffic.rx_bytes,
                    tx_bytes: a.traffic.tx_bytes,
                    last_rx_total: a.traffic.last_rx_total,
                    last_tx_total: a.traffic.last_tx_total,
                },
            )
        })
    };

    let (traffic_usage, row) = match traffic {
        Some((u, r)) => (Some(u), Some(r)),
        None => (None, None),
    };
    if let Some(r) = row {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::upsert_traffic(&conn, agent_id, &r) {
            tracing::warn!(agent_id, error = %e, "traffic flush after billing update failed");
        }
    }

    state.broadcast(BrowserMsg::Billing {
        agent_id,
        billing: Some(billing.clone()),
        traffic: traffic_usage.clone(),
    });

    Json(serde_json::json!({
        "billing": billing,
        "traffic": traffic_usage,
    }))
    .into_response()
}

// ---------------------------------------------------------------- themes

const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

fn theme_json(state: &SharedState, t: &db::ThemeRow, current: &Option<String>) -> serde_json::Value {
    let installed = themes::theme_dir(&state.themes_root, &t.id)
        .map(|d| d.is_dir())
        .unwrap_or(false);
    serde_json::json!({
        "id": t.id,
        "name": t.name,
        "version": t.version,
        "author": t.author,
        "description": t.description,
        "preview": t.preview,
        "source": t.source,
        "active": current.as_deref() == Some(t.id.as_str()),
        "installed": installed,
    })
}

async fn list_themes(State(state): State<SharedState>) -> Response {
    let (rows, current) = {
        let conn = state.db.lock().unwrap();
        let current = db::get_setting(&conn, "current_theme").ok().flatten();
        match db::list_themes(&conn) {
            Ok(rows) => (rows, current),
            Err(e) => return db_err(e),
        }
    };
    let mut list: Vec<serde_json::Value> =
        rows.iter().map(|t| theme_json(&state, t, &current)).collect();
    // The builtin `default` theme ships on disk and must always be listed,
    // even before an admin has uploaded anything.
    if !rows.iter().any(|t| t.id == "default")
        && state.themes_root.join("default").join("index.html").is_file()
    {
        list.insert(
            0,
            serde_json::json!({
                "id": "default",
                "name": "Default",
                "version": env!("CARGO_PKG_VERSION"),
                "author": "Pharus",
                "description": null,
                "preview": null,
                "source": "builtin",
                "active": current.as_deref() == Some("default"),
                "installed": true,
            }),
        );
    }
    Json(list).into_response()
}

async fn upload_theme(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Response {
    let mut data: Vec<u8> = Vec::new();
    while let Some(field) = match multipart.next_field().await {
        Ok(f) => f,
        Err(e) => return bad(&format!("multipart 解析失败: {e}")),
    } {
        if field.name() != Some("file") {
            continue;
        }
        use futures_util::TryStreamExt;
        let mut stream = field.into_stream();
        while let Ok(Some(chunk)) = stream.try_next().await {
            if data.len().saturating_add(chunk.len()) > MAX_UPLOAD_BYTES {
                return bad("主题包过大");
            }
            data.extend_from_slice(&chunk);
        }
    }
    if data.is_empty() {
        return bad("缺少主题包文件");
    }
    let manifest = match themes::install_zip(&state.themes_root, &data) {
        Ok(m) => m,
        Err(e) => return bad(&e.to_string()),
    };
    let row = db::ThemeRow {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version,
        author: manifest.author,
        description: manifest.description,
        preview: manifest.preview,
        source: "uploaded".into(),
        dir: format!("themes/{}", manifest.id),
        installed_at: chrono::Utc::now().timestamp(),
    };
    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::upsert_theme(&conn, &row) {
            return db_err(e);
        }
    }
    Json(serde_json::json!({ "id": row.id, "name": row.name, "version": row.version }))
        .into_response()
}

async fn activate_theme(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(dir) = themes::theme_dir(&state.themes_root, &id) else {
        return bad("非法主题 id");
    };
    if !dir.join("index.html").is_file() {
        return bad("主题目录不存在或缺少 index.html");
    }
    {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::set_setting(&conn, "current_theme", &id) {
            return db_err(e);
        }
    }
    StatusCode::OK.into_response()
}

async fn delete_theme(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let conn = state.db.lock().unwrap();
    let current = db::get_setting(&conn, "current_theme").ok().flatten();
    if current.as_deref() == Some(id.as_str()) {
        return bad("不能卸载当前激活的主题");
    }
    let source = match db::get_theme(&conn, &id) {
        Ok(Some(t)) => t.source,
        Ok(None) => {
            if id == "default" {
                return bad("内置主题不能卸载");
            }
            return err(StatusCode::NOT_FOUND, "theme not found");
        }
        Err(e) => return db_err(e),
    };
    if source == "builtin" {
        return bad("内置主题不能卸载");
    }
    if let Some(dir) = themes::theme_dir(&state.themes_root, &id) {
        let _ = std::fs::remove_dir_all(dir);
    }
    match db::delete_theme(&conn, &id) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => db_err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::valid_ping_target;

    #[test]
    fn icmp_and_tcp_targets_must_be_hosts() {
        assert!(valid_ping_target("icmp", "1.1.1.1"));
        assert!(valid_ping_target("tcp", "example.com"));
        assert!(!valid_ping_target("icmp", "-I eth0"));
        assert!(!valid_ping_target("tcp", "example.com; id"));
        assert!(!valid_ping_target("icmp", "https://example.com"));
    }

    #[test]
    fn http_targets_may_be_a_url_or_a_bare_host() {
        assert!(valid_ping_target("http", "https://example.com/health?x=1"));
        assert!(valid_ping_target("http", "http://example.com"));
        // The agent prefixes a scheme itself when the target is a bare host.
        assert!(valid_ping_target("http", "example.com"));
        assert!(!valid_ping_target("http", "https://exa mple.com"));
        assert!(!valid_ping_target("http", "https://"));
        assert!(!valid_ping_target("http", "ftp://example.com"));
    }
}
