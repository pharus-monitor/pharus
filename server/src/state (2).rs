use pharus_common::{
    AgentSnapshot, BrowserMsg, Metrics, PingResult, Region, SystemInfo, TrafficUsage, UnlockResult,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct TrafficState {
    pub cycle_start: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub last_rx_total: Option<u64>,
    pub last_tx_total: Option<u64>,
}

#[derive(Debug, Default)]
pub struct AgentState {
    pub name: String,
    pub online: bool,
    pub conn_epoch: u64,
    pub info: Option<SystemInfo>,
    pub data: Option<Metrics>,
    pub billing: Option<pharus_common::BillingInfo>,
    pub traffic: TrafficState,
    pub pings: Vec<PingResult>,
    pub unlock: Vec<UnlockResult>,
    pub region: Option<Region>,
    /// Diagnostics enabled for this agent after merging global and per-agent settings.
    pub features: Vec<String>,
    /// Agent application version reported at auth (None for old agents).
    pub app_version: Option<String>,
    /// Agent `os-arch` platform string reported at auth.
    pub platform: Option<String>,
    /// Docker containers reported by the agent. None = daemon unreachable
    /// (panel hidden); Some(empty) = Docker running, no containers.
    pub containers: Option<Vec<pharus_common::ContainerInfo>>,
    /// True once the agent reported Docker available; stays on across brief
    /// daemon hiccups so the panel doesn't flicker.
    pub docker_available: bool,
    /// Explicit SSH host[:port] override for the web terminal. None = derive
    /// from the agent-reported addresses.
    pub ssh_host: Option<String>,
    /// Channel to push server→agent messages onto the live socket.
    /// None when the agent is offline.
    pub agent_tx: Option<mpsc::UnboundedSender<pharus_common::ServerToAgentMsg>>,
}

impl AgentState {
    pub fn snapshot(&self, agent_id: i64) -> AgentSnapshot {
        AgentSnapshot {
            agent_id,
            name: self.name.clone(),
            online: self.online,
            info: self.info.clone(),
            data: self.data.clone(),
            billing: self.billing.clone(),
            traffic: Some(TrafficUsage {
                cycle_start: self.traffic.cycle_start,
                rx_bytes: self.traffic.rx_bytes,
                tx_bytes: self.traffic.tx_bytes,
            }),
            pings: self.pings.clone(),
            unlock: self.unlock.clone(),
            region: self.region.clone(),
            features: self.features.clone(),
            app_version: self.app_version.clone(),
            containers: if self.docker_available {
                Some(self.containers.clone().unwrap_or_default())
            } else {
                None
            },
            docker_available: self.docker_available,
        }
    }
}

pub struct AppState {
    pub agents: RwLock<HashMap<i64, AgentState>>,
    pub next_epoch: std::sync::atomic::AtomicU64,
    pub db: Mutex<Connection>,
    pub browser_tx: broadcast::Sender<BrowserMsg>,
    pub themes_root: PathBuf,
    /// Active admin sessions: session token -> (username, creation time).
    /// In-memory only, lost on restart; admins must re-login.
    pub sessions: Mutex<HashMap<String, (String, std::time::Instant)>>,
    /// Per-client failed-login counters for brute-force throttling.
    pub login_failures: Mutex<HashMap<String, LoginThrottle>>,
    /// Pending one-shot waiters for task results keyed by task_id.
    pub task_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<pharus_common::AgentMsg>>>,
    /// In-flight browser-initiated diagnostics keyed by request_id.
    pub diag_pending: Mutex<HashMap<String, crate::diag::DiagPending>>,
    /// Rolling timestamps of accepted diagnostic requests, per client IP, for
    /// a per-visitor request budget.
    pub diag_by_ip: Mutex<HashMap<String, Vec<std::time::Instant>>>,
    /// Rolling timestamps of dispatched iperf3 runs, per agent, for an hourly
    /// per-machine budget (the backstop against IP-rotating abuse).
    pub iperf3_by_agent: Mutex<HashMap<i64, Vec<std::time::Instant>>>,
    /// Cached online-update manifest (fetched from `update_manifest_url`).
    pub update_cache: Mutex<Option<(crate::updates::UpdateManifest, std::time::Instant)>>,
    /// Cached theme-store manifest (fetched from `theme_store_url`).
    pub theme_store_cache: Mutex<Option<(crate::themes::StoreManifest, std::time::Instant)>>,
    /// Browser terminal sessions: session_id -> outbound channel to the browser
    /// WebSocket. Agent terminal output is routed through this map.
    pub term_sessions: Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>,
}

#[derive(Debug, Clone, Copy)]
pub struct LoginThrottle {
    pub failures: u32,
    pub window_start: std::time::Instant,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self {
            failures: 0,
            window_start: std::time::Instant::now(),
        }
    }
}

impl AppState {
    pub fn broadcast(&self, msg: BrowserMsg) {
        let _ = self.browser_tx.send(msg);
    }

    /// Build a snapshot for `agent_id`, dropping `iperf3` from `features` when
    /// the agent's traffic usage has breached its auto-disable threshold, so
    /// the UI hides the control in addition to the endpoint refusing requests.
    pub fn snapshot_with_gates(&self, agent_id: i64) -> AgentSnapshot {
        let snapshot = {
            let agents = self.agents.read().unwrap();
            match agents.get(&agent_id) {
                Some(a) => a.snapshot(agent_id),
                None => AgentSnapshot {
                    agent_id,
                    name: format!("Agent #{agent_id}"),
                    online: false,
                    info: None,
                    data: None,
                    billing: None,
                    traffic: None,
                    pings: Vec::new(),
                    unlock: Vec::new(),
                    region: None,
                    features: Vec::new(),
                    app_version: None,
                    containers: None,
                    docker_available: false,
                },
            }
        };
        if snapshot.features.iter().any(|f| f == "iperf3")
            && crate::features::iperf3_blocked(self, agent_id)
        {
            let mut snap = snapshot;
            snap.features.retain(|f| f != "iperf3");
            snap
        } else {
            snapshot
        }
    }

    /// Push an agent's current scheduled ping + custom task set over its live
    /// socket. Silently does nothing when the agent is offline; the set is
    /// pushed again on reconnect.
    ///
    /// A feature the admin turned off yields an empty list rather than a skipped
    /// sync, because the agent replaces its whole task set on `TasksSync` — that
    /// is what actually stops an already-running schedule.
    pub fn push_tasks(&self, agent_id: i64) {
        let loaded = {
            let conn = self.db.lock().unwrap();
            crate::db::ping_tasks_for(&conn, agent_id)
                .and_then(|p| Ok((p, crate::db::tasks_for(&conn, agent_id)?)))
        };
        let (ping_tasks, custom_tasks) = match loaded {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(agent_id, error = %e, "task sync load failed");
                return;
            }
        };
        let Some((tx, features)) = ({
            let agents = self.agents.read().unwrap();
            agents
                .get(&agent_id)
                .and_then(|a| a.agent_tx.clone().map(|tx| (tx, a.features.clone())))
        }) else {
            return;
        };
        let allows = |f: &str| features.iter().any(|have| have == f);
        let _ = tx.send(pharus_common::ServerToAgentMsg::TasksSync {
            ping_tasks: if allows("ping") { ping_tasks } else { Vec::new() },
            custom_tasks: if allows("tasks") { custom_tasks } else { Vec::new() },
        });
    }

    pub fn push_tasks_all(&self) {
        let ids: Vec<i64> = {
            let agents = self.agents.read().unwrap();
            agents
                .iter()
                .filter(|(_, a)| a.online)
                .map(|(id, _)| *id)
                .collect()
        };
        for id in ids {
            self.push_tasks(id);
        }
    }
}

pub type SharedState = Arc<AppState>;
