use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub virtualization: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub cpu_usage: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub load1: f64,
    pub uptime: u64,
    /// Cumulative bytes received since boot (0 = old agent, not reported)
    #[serde(default)]
    pub net_rx_total: u64,
    /// Cumulative bytes transmitted since boot (0 = old agent, not reported)
    #[serde(default)]
    pub net_tx_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    CNY,
    USD,
    EUR,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingCycle {
    Monthly,
    Quarterly,
    Yearly,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BillingInfo {
    pub reset_day: Option<u8>,
    pub quota_bytes: Option<u64>,
    pub expires_at: Option<i64>,
    pub price: Option<f64>,
    pub currency: Option<Currency>,
    pub cycle: Option<BillingCycle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficUsage {
    pub cycle_start: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingTarget {
    pub label: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub label: String,
    /// None = unreachable / timeout
    pub rtt_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Ping,
    Traceroute,
    Mtr,
    Script,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockResult {
    pub service: String,
    /// "yes" | "no" | "fail"
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMsg {
    Auth { token: String, version: u8 },
    SysInfo { info: SystemInfo },
    Metrics { data: Metrics },
    Ping { results: Vec<PingResult> },
    TaskResult { task_id: String, exit_code: i32, output: String },
    Unlock { results: Vec<UnlockResult> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgentMsg {
    AuthOk { agent_id: i64, name: String },
    AuthFail { reason: String },
    Config { tcping: Vec<PingTarget> },
    RunTask { task_id: String, kind: TaskKind, target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub agent_id: i64,
    pub name: String,
    pub online: bool,
    pub info: Option<SystemInfo>,
    pub data: Option<Metrics>,
    #[serde(default)]
    pub billing: Option<BillingInfo>,
    #[serde(default)]
    pub traffic: Option<TrafficUsage>,
    #[serde(default)]
    pub pings: Vec<PingResult>,
    #[serde(default)]
    pub unlock: Vec<UnlockResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserMsg {
    Snapshot { agents: Vec<AgentSnapshot> },
    Metrics { agent_id: i64, online: bool, data: Metrics },
    Status { agent_id: i64, online: bool },
    Billing { agent_id: i64, billing: Option<BillingInfo>, traffic: Option<TrafficUsage> },
    Pings { agent_id: i64, results: Vec<PingResult> },
    Unlock { agent_id: i64, results: Vec<UnlockResult> },
}
