//! Browser-initiated diagnostics (looking glass, MTR).
//!
//! A request is dispatched to the agent over its live socket and answered
//! asynchronously: output frames arrive on the agent socket and are relayed to
//! every browser as `BrowserMsg::DiagResult`, correlated by `request_id`.

use crate::features;
use crate::state::SharedState;
use pharus_common::{BrowserMsg, MtrHop, ServerToAgentMsg, TaskKind};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// A request whose output frames have not finished arriving yet.
#[derive(Debug, Clone)]
pub struct DiagPending {
    pub agent_id: i64,
    /// "ping" | "traceroute" | "mtr"
    pub kind: String,
    pub started: i64,
}

/// Requests older than this are assumed dead and dropped.
const PENDING_TTL: i64 = 300;
const MAX_TARGET_LEN: usize = 255;

#[derive(Debug)]
pub enum DiagError {
    FeatureDisabled,
    AgentOffline,
    BadTarget,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The target is passed to `ping`/`traceroute`/`mtr` as an argv entry, so shell
/// metacharacters cannot escape — but a leading `-` would still be read as a
/// flag. Restrict to what a hostname or IP literal can contain.
fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() <= MAX_TARGET_LEN
        && !target.starts_with('-')
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_' | '[' | ']'))
}

fn dispatch(
    state: &SharedState,
    agent_id: i64,
    feature: &str,
    kind: &str,
    msg_kind: TaskKind,
    target: &str,
    cycles: Option<u32>,
) -> Result<String, DiagError> {
    if !valid_target(target) {
        return Err(DiagError::BadTarget);
    }
    // Defense in depth: the frontend hides disabled controls, but the check
    // that matters is this one.
    if !features::enabled(state, agent_id, feature) {
        return Err(DiagError::FeatureDisabled);
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
        pending.insert(
            request_id.clone(),
            DiagPending {
                agent_id,
                kind: kind.to_string(),
                started: now(),
            },
        );
    }

    let sent = tx.send(ServerToAgentMsg::RunTask {
        task_id: request_id.clone(),
        kind: msg_kind,
        target: target.to_string(),
        cycles,
        timeout: None,
    });
    if sent.is_err() {
        state.diag_pending.lock().unwrap().remove(&request_id);
        return Err(DiagError::AgentOffline);
    }
    Ok(request_id)
}

pub fn start_lg(
    state: &SharedState,
    agent_id: i64,
    kind: &str,
    target: &str,
) -> Result<String, DiagError> {
    let task_kind = match kind {
        "ping" => TaskKind::Ping,
        "traceroute" => TaskKind::Traceroute,
        _ => return Err(DiagError::BadTarget),
    };
    dispatch(state, agent_id, "lg", kind, task_kind, target, None)
}

pub fn start_mtr(
    state: &SharedState,
    agent_id: i64,
    target: &str,
    cycles: Option<u32>,
) -> Result<String, DiagError> {
    let cycles = cycles.map(|c| c.clamp(1, 30));
    dispatch(state, agent_id, "mtr", "mtr", TaskKind::Mtr, target, cycles)
}

fn take_pending(state: &SharedState, request_id: &str, agent_id: i64, done: bool) -> Option<String> {
    let mut pending = state.diag_pending.lock().unwrap();
    let entry = pending.get(request_id)?;
    // A stale or spoofed request_id must not let one agent inject output into
    // another agent's diagnostic stream.
    if entry.agent_id != agent_id {
        warn!(agent_id, request_id, "diag frame from the wrong agent, dropping");
        return None;
    }
    let kind = entry.kind.clone();
    if done {
        pending.remove(request_id);
    }
    Some(kind)
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
    let Some(kind) = take_pending(state, &request_id, agent_id, done) else {
        return;
    };
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

/// Relay a structured MTR result. Always terminal.
pub fn relay_mtr(state: &SharedState, agent_id: i64, request_id: String, hops: Vec<MtrHop>) {
    if take_pending(state, &request_id, agent_id, true).is_none() {
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
