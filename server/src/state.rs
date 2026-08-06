use pharus_common::{AgentSnapshot, BrowserMsg, Metrics, SystemInfo, TrafficUsage};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;

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
        }
    }
}

pub struct AppState {
    pub agents: RwLock<HashMap<i64, AgentState>>,
    pub next_epoch: std::sync::atomic::AtomicU64,
    pub db: Mutex<Connection>,
    pub browser_tx: broadcast::Sender<BrowserMsg>,
    pub themes_root: PathBuf,
    pub admin_token: Option<String>,
}

impl AppState {
    pub fn broadcast(&self, msg: BrowserMsg) {
        let _ = self.browser_tx.send(msg);
    }
}

pub type SharedState = Arc<AppState>;
