use pharus_common::{BrowserMsg, Metrics, SystemInfo};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;

#[derive(Debug, Default)]
pub struct AgentState {
    pub name: String,
    pub online: bool,
    pub conn_epoch: u64,
    pub info: Option<SystemInfo>,
    pub data: Option<Metrics>,
}

pub struct AppState {
    pub agents: RwLock<HashMap<i64, AgentState>>,
    pub next_epoch: std::sync::atomic::AtomicU64,
    pub db: Mutex<Connection>,
    pub browser_tx: broadcast::Sender<BrowserMsg>,
    pub themes_root: PathBuf,
}

impl AppState {
    pub fn broadcast(&self, msg: BrowserMsg) {
        let _ = self.browser_tx.send(msg);
    }
}

pub type SharedState = Arc<AppState>;
