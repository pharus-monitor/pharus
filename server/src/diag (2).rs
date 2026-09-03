//! Browser-initiated diagnostics (looking glass, MTR).
//!
//! A request is dispatched to the agent over its live socket and answered
//! asynchronously: output frames arrive on the agent socket and are relayed to
//! every browser as `BrowserMsg::DiagResult`, correlated by `request_id`.

use crate::features;
use crate::regions;
use crate::state::SharedState;
use pharus_common::{BrowserMsg, MtrHop, ServerToAgentMsg, TaskKind};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// A request whose output frames have not finished arriving yet.
#[derive(Debug, Clone)]
pub struct DiagPending {
    pub agent_id: i64,
    /// "ping" | "traceroute" | "mtr"
    pub kind: String,
    pub started: i64,
    /// Output bytes relayed so far, so a misbehaving agent cannot stream
    /// without end.
    pub relayed: usize,
}

/// Requests older than this are assumed dead and dropped.
const PENDING_TTL: i64 = 300;
const MAX_TARGET_LEN: usize = 255;
/// These endpoints are public, so the only thing standing between an anonymous
/// visitor and unlimited `ping`/`mtr` processes on every agent is this cap.
const MAX_IN_FLIGHT_PER_AGENT: usize = 4;
const MAX_IN_FLIGHT_TOTAL: usize = 64;
/// The agent caps its own output, but a compromised agent is exactly the case
/// this guards against, so the server counts the bytes itself.
const MAX_RELAY_BYTES: usize = 256 * 1024;
/// Time-based anti-abuse budgets. Values are configurable via the settings of
/// the same name; these are the fallbacks.
const DIAG_PER_IP_MINUTE_DEFAULT: usize = 12;
const IPERF3_PER_AGENT_HOUR_DEFAULT: usize = 10;

#[derive(Debug)]
pub enum DiagError {
    FeatureDisabled,
    AgentOffline,
    BadTarget,
    TooManyRequests,
    RateLimited,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn setting_usize(state: &SharedState, key: &str, default: usize) -> usize {
    let conn = state.db.lock().unwrap();
    crate::db::get_setting(&conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

/// Sliding per-client request budget across all diagnostic kinds, so one
/// visitor cannot spam `ping`/`mtr`/`iperf3` on every agent.
pub fn check_ip_budget(state: &SharedState, ip: &str) -> Result<(), DiagError> {
    let limit = setting_usize(state, "diag_per_ip_minute", DIAG_PER_IP_MINUTE_DEFAULT);
    let mut map = state.diag_by_ip.lock().unwrap();
    let cutoff = Instant::now() - Duration::from_secs(60);
    map.retain(|_, v| {
        v.retain(|t| *t > cutoff);
        !v.is_empty()
    });
    let bucket = map.entry(ip.to_string()).or_default();
    if bucket.len() >= limit {
        return Err(DiagError::RateLimited);
    }
    bucket.push(Instant::now());
    Ok(())
}

/// Sliding hourly budget of iperf3 runs per agent. iperf3 burns the machine's
/// bandwidth, so this is the backstop against an attacker rotating IPs to keep
/// hammering a single host regardless of where the request comes from.
pub fn check_iperf3_agent_budget(state: &SharedState, agent_id: i64) -> Result<(), DiagError> {
    let limit = setting_usize(state, "iperf3_per_agent_hour", IPERF3_PER_AGENT_HOUR_DEFAULT);
    let mut map = state.iperf3_by_agent.lock().unwrap();
    let cutoff = Instant::now() - Duration::from_secs(3600);
    map.retain(|_, v| {
        v.retain(|t| *t > cutoff);
        !v.is_empty()
    });
    let bucket = map.entry(agent_id).or_default();
    if bucket.len() >= limit {
        return Err(DiagError::RateLimited);
    }
    bucket.push(Instant::now());
    Ok(())
}

/// The target is passed to `ping`/`traceroute`/`mtr` as an argv entry, so shell
/// metacharacters cannot escape — but a leading `-` would still be read as a
/// flag. Restrict to what a hostname or IP literal can contain.
pub fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() <= MAX_TARGET_LEN
        && !target.starts_with('-')
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_' | '[' | ']'))
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    state: &SharedState,
    ip: &str,
    agent_id: i64,
    feature: &str,
    kind: &str,
    msg_kind: TaskKind,
    target: &str,
    cycles: Option<u32>,
    extra: Option<serde_json::Value>,
) -> Result<String, DiagError> {
    if !valid_target(target) {
        return Err(DiagError::BadTarget);
    }
    // Defense in depth: the frontend hides disabled controls, but the check
    // that matters is this one.
    if !features::enabled(state, agent_id, feature) {
        return Err(DiagError::FeatureDisabled);
    }
    // Time-based anti-abuse budgets (per-visitor rate + per-machine iperf3
    // hourly cap). Enforced before any request id is minted.
    check_ip_budget(state, ip)?;
    if feature == "iperf3" {
        check_iperf3_agent_budget(state, agent_id)?;
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let tx = {
        let agents = state.agents.read().unwrap();
        match agents.get(&agent_id) {
            Some(a) if a.online => a.agent_tx.clone(),
            _ => None,
        }
    };
    let Some(tx) = tx else {
        return Err(DiagError::AgentOffline);
    };

    {
        let mut pending = state.diag_pending.lock().unwrap();
        let cutoff = now() - PENDING_TTL;
        pending.retain(|_, p| p.started >= cutoff);
        if pending.len() >= MAX_IN_FLIGHT_TOTAL
            || pending.values().filter(|p| p.agent_id == agent_id).count()
                >= MAX_IN_FLIGHT_PER_AGENT
        {
            return Err(DiagError::TooManyRequests);
        }
        pending.insert(
            request_id.clone(),
            DiagPending {
                agent_id,
                kind: kind.to_string(),
                started: now(),
                relayed: 0,
            },
        );
    }

    let sent = tx.send(ServerToAgentMsg::RunTask {
        task_id: request_id.clone(),
        kind: msg_kind,
        target: target.to_string(),
        cycles,
        timeout: None,
        extra,
    });
    if sent.is_err() {
        state.diag_pending.lock().unwrap().remove(&request_id);
        return Err(DiagError::AgentOffline);
    }
    info!(
        request_id = %request_id,
        agent_id,
        kind = %kind,
        target = %target,
        "diag dispatched"
    );
    Ok(request_id)
}

pub fn start_lg(
    state: &SharedState,
    ip: &str,
    agent_id: i64,
    kind: &str,
    target: &str,
) -> Result<String, DiagError> {
    let task_kind = match kind {
        "ping" => TaskKind::Ping,
        "traceroute" => TaskKind::Traceroute,
        _ => return Err(DiagError::BadTarget),
    };
    dispatch(state, ip, agent_id, "lg", kind, task_kind, target, None, None)
}

pub fn start_mtr(
    state: &SharedState,
    ip: &str,
    agent_id: i64,
    target: &str,
    cycles: Option<u32>,
) -> Result<String, DiagError> {
    let cycles = cycles.map(|c| c.clamp(1, 30));
    dispatch(state, ip, agent_id, "mtr", "mtr", TaskKind::Mtr, target, cycles, None)
}

/// iperf3 params are carried in `extra`; the server host still goes through
/// the same argv-only `target` path so no shell injection is possible.
#[allow(clippy::too_many_arguments)]
pub fn start_iperf3(
    state: &SharedState,
    ip: &str,
    agent_id: i64,
    server: &str,
    port: u16,
    direction: &str,
    duration: u32,
    parallel: u32,
    protocol: &str,
    length: Option<u32>,
) -> Result<String, DiagError> {
    let extra = serde_json::json!({
        "port": port,
        "direction": direction,
        "duration": duration,
        "parallel": parallel,
        "protocol": protocol,
        "length": length,
    });
    let request_id = dispatch(
        state,
        ip,
        agent_id,
        "iperf3",
        "iperf3",
        TaskKind::Iperf3,
        server,
        None,
        Some(extra),
    )?;
    log_iperf3(state, ip, agent_id, server);
    Ok(request_id)
}

/// Best-effort country + ASN lookup for a target IP, trying several free
/// sources in order until one yields data. `None` on total failure so logging
/// never blocks or depends on any single source.
async fn geo_lookup(ip: &str) -> Option<(Option<String>, Option<String>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    if let Some(found) = geo_ipinfo(&client, ip).await {
        return Some(found);
    }
    if let Some(found) = geo_ipwhois(&client, ip).await {
        return Some(found);
    }
    geo_ipapi(&client, ip).await
}

async fn geo_ipinfo(client: &reqwest::Client, ip: &str) -> Option<(Option<String>, Option<String>)> {
    let url = format!("https://ipinfo.io/{ip}/json");
    let resp = client.get(&url).send().await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    let region = json
        .get("country")
        .and_then(serde_json::Value::as_str)
        .map(regions::region_name)
        .map(str::to_string);
    let asn = json
        .get("org")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    (region.is_some() || asn.is_some()).then_some((region, asn))
}

async fn geo_ipwhois(client: &reqwest::Client, ip: &str) -> Option<(Option<String>, Option<String>)> {
    let url = format!("https://ipwhois.app/json/{ip}");
    let resp = client.get(&url).send().await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    if json.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    let region = json
        .get("country_code")
        .and_then(serde_json::Value::as_str)
        .map(regions::region_name)
        .map(str::to_string);
    let asn = json
        .get("as")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            json.get("connection")
                .and_then(|c| c.get("asn"))
                .and_then(serde_json::Value::as_u64)
                .map(|n| format!("AS{n}"))
        });
    (region.is_some() || asn.is_some()).then_some((region, asn))
}

async fn geo_ipapi(client: &reqwest::Client, ip: &str) -> Option<(Option<String>, Option<String>)> {
    let url = format!("http://ip-api.com/json/{ip}?fields=status,countryCode,as");
    let resp = client.get(&url).send().await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    if json.get("status").and_then(serde_json::Value::as_str) != Some("success") {
        return None;
    }
    let region = json
        .get("countryCode")
        .and_then(serde_json::Value::as_str)
        .map(regions::region_name)
        .map(str::to_string);
    let asn = json
        .get("as")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    (region.is_some() || asn.is_some()).then_some((region, asn))
}

/// Persist an accepted iperf3 run (requester IP + target), then enrich it with
/// the target's region/ASN in the background so the geo lookup never delays the
/// dispatch.
fn log_iperf3(state: &SharedState, client_ip: &str, agent_id: i64, target: &str) {
    let id = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        match crate::db::insert_iperf3_log(&conn, agent_id, client_ip, target, now()) {
            Ok(id) => id,
            Err(e) => {
                warn!(error = %e, "iperf3 log insert failed");
                return;
            }
        }
    };
    let state = state.clone();
    let target = target.to_string();
    tokio::spawn(async move {
        let Some((region, asn)) = geo_lookup(&target).await else { return };
        if let Ok(conn) = state.db.lock() {
            if let Err(e) =
                crate::db::update_iperf3_log(&conn, id, region.as_deref(), asn.as_deref())
            {
                warn!(error = %e, "iperf3 log enrich failed");
            }
        }
    });
}

fn take_pending(state: &SharedState, request_id: &str, agent_id: i64, done: bool) -> Option<String> {
    take_pending_bytes(state, request_id, agent_id, done, 0).map(|(kind, _)| kind)
}

/// Returns the request kind and whether the relay budget just ran out, in which
/// case the caller must close the stream out itself.
fn take_pending_bytes(
    state: &SharedState,
    request_id: &str,
    agent_id: i64,
    done: bool,
    bytes: usize,
) -> Option<(String, bool)> {
    let mut pending = state.diag_pending.lock().unwrap();
    let entry = pending.get_mut(request_id)?;
    // A stale or spoofed request_id must not let one agent inject output into
    // another agent's diagnostic stream.
    if entry.agent_id != agent_id {
        warn!(agent_id, request_id, "diag frame from the wrong agent, dropping");
        return None;
    }
    entry.relayed = entry.relayed.saturating_add(bytes);
    let over_budget = entry.relayed > MAX_RELAY_BYTES;
    let kind = entry.kind.clone();
    if done || over_budget {
        pending.remove(request_id);
    }
    Some((kind, over_budget))
}

/// Relay one incremental output frame from the agent to the browsers.
pub fn relay_output(
    state: &SharedState,
    agent_id: i64,
    request_id: String,
    stream: String,
    data: String,
    done: bool,
    exit_code: Option<i32>,
) {
    let Some((kind, over_budget)) =
        take_pending_bytes(state, &request_id, agent_id, done, data.len())
    else {
        return;
    };
    if over_budget && !done {
        warn!(agent_id, request_id, "diag output budget exceeded, closing stream");
        state.broadcast(BrowserMsg::DiagResult {
            request_id,
            agent_id,
            kind,
            stream: Some("stderr".into()),
            data: Some("output limit exceeded\n".into()),
            result: None,
            done: true,
            exit_code: Some(-1),
        });
        return;
    }
    state.broadcast(BrowserMsg::DiagResult {
        request_id,
        agent_id,
        kind,
        stream: Some(stream),
        data: Some(data),
        result: None,
        done,
        exit_code,
    });
}

/// An agent predating `DiagOutput` answers a dispatched request with a plain
/// buffered `TaskResult`. Close the browser's request out with it instead of
/// dropping the reply. Does nothing when the id belongs to something else.
pub fn relay_legacy_result(
    state: &SharedState,
    agent_id: i64,
    request_id: String,
    exit_code: i32,
    output: String,
) {
    let Some(kind) = take_pending(state, &request_id, agent_id, true) else {
        return;
    };
    state.broadcast(BrowserMsg::DiagResult {
        request_id,
        agent_id,
        kind,
        stream: Some("stdout".into()),
        data: Some(output),
        result: None,
        done: true,
        exit_code: Some(exit_code),
    });
}

/// Relay a structured MTR snapshot. Progressive updates (`done: false`) keep
/// the request pending; the terminal frame closes it out.
pub fn relay_mtr(state: &SharedState, agent_id: i64, request_id: String, hops: Vec<MtrHop>, done: bool) {
    if take_pending(state, &request_id, agent_id, done).is_none() {
        return;
    }
    let result = serde_json::to_value(&hops).ok();
    state.broadcast(BrowserMsg::DiagResult {
        request_id,
        agent_id,
        kind: "mtr".into(),
        stream: None,
        data: None,
        result,
        done,
        exit_code: if done { Some(0) } else { None },
    });
}

/// Relay the final structured iperf3 result to the requesting browser.
pub fn relay_iperf3(
    state: &SharedState,
    agent_id: i64,
    request_id: String,
    direction: String,
    throughput_bps: Option<f64>,
    retransmits: Option<u32>,
    duration_s: Option<f64>,
) {
    if take_pending(state, &request_id, agent_id, true).is_none() {
        return;
    }
    let result = serde_json::json!({
        "direction": direction,
        "throughput_bps": throughput_bps,
        "retransmits": retransmits,
        "duration_s": duration_s,
    });
    state.broadcast(BrowserMsg::DiagResult {
        request_id,
        agent_id,
        kind: "iperf3".into(),
        stream: None,
        data: None,
        result: Some(result),
        done: true,
        exit_code: Some(0),
    });
}

/// Close out every in-flight request for an agent that just went away, so the
/// browser stops waiting on output that will never arrive.
pub fn cancel_for_agent(state: &SharedState, agent_id: i64) {
    let orphaned: Vec<(String, String)> = {
        let mut pending = state.diag_pending.lock().unwrap();
        let hit: Vec<(String, String)> = pending
            .iter()
            .filter(|(_, p)| p.agent_id == agent_id)
            .map(|(id, p)| (id.clone(), p.kind.clone()))
            .collect();
        for (id, _) in &hit {
            pending.remove(id);
        }
        hit
    };
    for (request_id, kind) in orphaned {
        state.broadcast(BrowserMsg::DiagResult {
            request_id,
            agent_id,
            kind,
            stream: Some("stderr".into()),
            data: Some("agent disconnected\n".into()),
            result: None,
            done: true,
            exit_code: Some(-1),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::valid_target;

    #[test]
    fn accepts_hosts_and_addresses() {
        assert!(valid_target("example.com"));
        assert!(valid_target("1.1.1.1"));
        assert!(valid_target("2606:4700:4700::1111"));
        assert!(valid_target("my-host_1.internal"));
    }

    #[test]
    fn rejects_flags_and_shell_metacharacters() {
        assert!(!valid_target("-I eth0"));
        assert!(!valid_target("--help"));
        assert!(!valid_target("a.com; rm -rf /"));
        assert!(!valid_target("a.com && id"));
        assert!(!valid_target("$(id)"));
        assert!(!valid_target("a b"));
        assert!(!valid_target(""));
        assert!(!valid_target(&"a".repeat(256)));
    }
}
