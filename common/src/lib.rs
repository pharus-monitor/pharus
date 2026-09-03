use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

fn default_true() -> bool {
    true
}

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
    /// Public IPv4/IPv6 addresses reported by the agent (loopback and
    /// private/link-local IPv4 excluded).
    #[serde(default)]
    pub ips: Vec<String>,
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
    /// Disk write rate in bytes/s since the last sample (0 = old agent).
    #[serde(default)]
    pub disk_write_bps: u64,
    /// Disk read rate in bytes/s since the last sample (0 = old agent).
    #[serde(default)]
    pub disk_read_bps: u64,
    /// CPU temperature in °C (best effort; None when the agent can't read it).
    #[serde(default)]
    pub temperature_c: Option<f32>,
    /// Primary GPU name (e.g. "NVIDIA GeForce RTX 4090"); None without a GPU.
    #[serde(default)]
    pub gpu_name: Option<String>,
    /// GPU utilization percent 0..=100 (best effort).
    #[serde(default)]
    pub gpu_util: Option<f32>,
    /// GPU memory used / total in bytes (best effort).
    #[serde(default)]
    pub gpu_mem_used: Option<u64>,
    #[serde(default)]
    pub gpu_mem_total: Option<u64>,
    /// Number of running processes.
    #[serde(default)]
    pub process_count: u32,
    /// Number of open TCP/UDP sockets.
    #[serde(default)]
    pub connection_count: u32,
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
    /// Per-host traffic accounting mode; falls back to "bi" when absent.
    #[serde(default)]
    pub traffic_mode: Option<String>,
    /// Per-host uni-directional traffic pick; falls back to "down" when absent.
    #[serde(default)]
    pub traffic_dir: Option<String>,
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
    /// HTTP status code captured by an http probe (service-health view).
    #[serde(default)]
    pub status: Option<u16>,
    /// Days until the served TLS certificate expires (ssl probe).
    #[serde(default)]
    pub cert_days: Option<f64>,
    /// Subject common name of the served certificate (ssl probe).
    #[serde(default)]
    pub cert_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PingKind {
    Icmp,
    Tcp,
    Http,
    Ssl,
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
    Iperf3,
    Speedtest,
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
    /// RTT of the most recent probe; absent from pre-0.4.2 agents.
    #[serde(default)]
    pub last: f64,
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

/// A Docker container running on the agent host (best effort; absent on hosts
/// without a Docker daemon).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    /// "running" | "exited" | "created" | ...
    pub state: String,
    /// CPU utilization percent, rate-sampled between polls (None before the
    /// second sample).
    #[serde(default)]
    pub cpu_pct: Option<f64>,
    /// Memory used / limit in bytes.
    #[serde(default)]
    pub mem_used: Option<u64>,
    #[serde(default)]
    pub mem_limit: Option<u64>,
    #[serde(default)]
    pub restart_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMsg {
    Auth {
        token: String,
        version: u8,
        /// Hostname reported by the agent, used to match a connection that
        /// authenticates with the shared `agent_secret` instead of a
        /// per-agent token.
        #[serde(default)]
        name: Option<String>,
        /// Agent application version (e.g. "0.6.1"); absent on old agents.
        #[serde(default)]
        app_version: Option<String>,
        /// `os-arch` (e.g. "linux-x86_64", "windows-x86_64"), used to pick the
        /// update asset. Absent on old agents.
        #[serde(default)]
        platform: Option<String>,
    },
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
    /// Structured MTR snapshot. New agents stream progressive updates with
    /// `done: false` and a terminal frame with `done: true`; old agents send a
    /// single frame without the field, which must read as terminal.
    MtrResult {
        request_id: String,
        hubs: Vec<MtrHop>,
        #[serde(default = "default_true")]
        done: bool,
    },
    /// Structured iperf3 result parsed from `iperf3 -J`.
    Iperf3Result {
        request_id: String,
        direction: String,
        #[serde(default)]
        throughput_bps: Option<f64>,
        #[serde(default)]
        retransmits: Option<u32>,
        #[serde(default)]
        duration_s: Option<f64>,
    },
    /// Built-in speedtest result (no external iperf3 server needed).
    SpeedtestResult {
        request_id: String,
        /// "down" | "up"
        direction: String,
        /// Throughput in bits per second.
        throughput_bps: f64,
        /// Elapsed time in milliseconds.
        elapsed_ms: u64,
        /// Total bytes transferred.
        bytes_transferred: u64,
    },
    /// Real-time progress update during a speedtest (sent every ~200ms).
    SpeedtestProgress {
        request_id: String,
        /// "down" | "up"
        direction: String,
        /// Current sample throughput in bits per second.
        throughput_bps: f64,
        /// Total bytes transferred so far.
        bytes_transferred: u64,
        /// Elapsed time in milliseconds for the current phase.
        elapsed_ms: u64,
        /// True when this is the final progress update for the phase.
        #[serde(default)]
        done: bool,
    },
    /// Agent-side region detection reported once per connection.
    Region { code: String },
    /// Progress of a server-triggered online update.
    UpdateStatus {
        request_id: String,
        /// "downloading" | "verifying" | "applying" | "restarting"
        phase: String,
        /// True once the agent is about to restart.
        #[serde(default)]
        done: bool,
        #[serde(default)]
        error: Option<String>,
    },
    /// Docker containers on the agent host (empty when Docker is unavailable).
    Containers { containers: Vec<ContainerInfo> },
    /// Interactive terminal: the agent writes pty output for a session.
    TermOutput {
        session_id: String,
        data: String,
    },
    /// The pty session ended.
    TermExit {
        session_id: String,
        #[serde(default)]
        exit_code: Option<i32>,
    },
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
        /// Extra parameters for kinds that need more than target+cycles, e.g.
        /// iperf3 {server, port, direction, duration, parallel}.
        #[serde(default)]
        extra: Option<serde_json::Value>,
    },
    TasksSync {
        ping_tasks: Vec<PingTaskSpec>,
        custom_tasks: Vec<CustomTaskSpec>,
    },
    /// Ask the agent to download, verify and swap in a new binary, then
    /// restart itself. `kind` is "exe" (Windows raw binary) or "tar_gz"
    /// (Linux archive containing `pharus-agent`).
    Update {
        request_id: String,
        version: String,
        asset_url: String,
        /// Hex sha256 of the asset the agent must verify before applying.
        sha256: String,
        kind: String,
    },
    /// Open an interactive shell (pty) on the agent for the browser terminal.
    TermOpen {
        session_id: String,
        cols: u16,
        rows: u16,
        /// Optional explicit shell (e.g. "bash"); None = platform default.
        #[serde(default)]
        shell: Option<String>,
    },
    TermInput {
        session_id: String,
        data: String,
    },
    TermResize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    TermClose {
        session_id: String,
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
    /// Agent application version reported at auth (None for old agents).
    #[serde(default)]
    pub app_version: Option<String>,
    /// Docker containers reported by the agent (empty when none / unavailable).
    #[serde(default)]
    pub containers: Vec<ContainerInfo>,
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
    /// Progress of an online update (agent or server) for the admin console.
    UpdateStatus {
        #[serde(default)]
        agent_id: Option<i64>,
        /// "agent" | "server"
        kind: String,
        /// "downloading" | "verifying" | "applying" | "restarting"
        phase: String,
        #[serde(default)]
        done: bool,
        #[serde(default)]
        error: Option<String>,
    },
    Containers { agent_id: i64, containers: Vec<ContainerInfo> },
}

/// `os-arch` platform string, e.g. "linux-x86_64" / "windows-x86_64", used by
/// the server to pick the right update asset.
pub fn platform_string() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        other => other,
    };
    format!("{}-{arch}", std::env::consts::OS)
}

/// Compute SHA-256 hash of data and return it as a hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
