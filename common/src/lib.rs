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
    /// Best-effort memory description, e.g. "Samsung 4800MHz". None when the
    /// agent could not read it (no dmidecode / WMI access).
    #[serde(default)]
    pub mem_desc: Option<String>,
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
    /// Manual bandwidth cap in Mbps.
    #[serde(default)]
    pub bandwidth: Option<f64>,
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
    /// Average RTT; None = unreachable / timeout
    pub rtt_ms: Option<f64>,
    /// Set when the probe came from a server-managed ping task; drives history storage
    #[serde(default)]
    pub task_id: Option<i64>,
    #[serde(default)]
    pub rtt_min: Option<f64>,
    #[serde(default)]
    pub rtt_max: Option<f64>,
    /// Packet loss ratio 0.0..=1.0
    #[serde(default)]
    pub loss: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PingKind {
    Icmp,
    Tcp,
    Http,
}

/// A ping/tcping probe the agent should run on its own schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingTaskSpec {
    pub id: i64,
    pub label: String,
    pub kind: PingKind,
    pub target: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub interval: u64,
    pub count: u32,
}

/// An operator-defined command the agent should run on its own schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTaskSpec {
    pub id: i64,
    pub command: String,
    /// Seconds; 0 = manual trigger only
    pub interval: u64,
    pub timeout: u64,
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
pub struct MtrHop {
    pub hop: u32,
    pub host: String,
    pub loss: f64,
    pub sent: u32,
    pub avg: f64,
    pub best: f64,
    pub worst: f64,
    pub stdev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    /// ISO 3166-1 alpha-2, e.g. US / JP / HK
    pub code: String,
    pub name: String,
    /// "auto" (looked up on first connect) or "manual" (operator override)
    pub source: String,
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
    TaskResult {
        task_id: String,
        exit_code: i32,
        output: String,
        /// Set when this is a periodic run of a stored custom task, so the
        /// server persists it instead of routing it to a one-shot waiter.
        #[serde(default)]
        scheduled_id: Option<i64>,
    },
    Unlock { results: Vec<UnlockResult> },
    /// Incremental output of a long-running diagnostic; the final frame carries
    /// `done: true` and the exit code.
    CmdOutput {
        request_id: String,
        /// "stdout" | "stderr"
        stream: String,
        data: String,
        done: bool,
        #[serde(default)]
        exit_code: Option<i32>,
    },
    MtrResult { request_id: String, hubs: Vec<MtrHop> },
    /// Agent-side region detection reported once per connection.
    Region { code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgentMsg {
    AuthOk { agent_id: i64, name: String },
    AuthFail { reason: String },
    /// Legacy TCPing config, kept for agents that predate `TasksSync`.
    Config { tcping: Vec<PingTarget> },
    RunTask {
        task_id: String,
        kind: TaskKind,
        target: String,
        /// Probe/report cycles, currently honoured by MTR.
        #[serde(default)]
        cycles: Option<u32>,
        /// Seconds before the agent kills the process; agent default when unset.
        #[serde(default)]
        timeout: Option<u64>,
    },
    TasksSync {
        ping_tasks: Vec<PingTaskSpec>,
        custom_tasks: Vec<CustomTaskSpec>,
    },
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
    #[serde(default)]
    pub region: Option<Region>,
    /// Diagnostics enabled for this agent after merging global and per-agent settings.
    #[serde(default)]
    pub features: Vec<String>,
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
    /// Relayed diagnostic output for a browser-initiated request.
    DiagResult {
        request_id: String,
        agent_id: i64,
        /// "ping" | "traceroute" | "mtr"
        kind: String,
        #[serde(default)]
        stream: Option<String>,
        #[serde(default)]
        data: Option<String>,
        #[serde(default)]
        result: Option<serde_json::Value>,
        done: bool,
        #[serde(default)]
        exit_code: Option<i32>,
    },
    FeaturesUpdate { agent_id: i64, features: Vec<String> },
    RegionUpdate { agent_id: i64, region: Option<Region> },
}
