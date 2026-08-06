use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use pharus_common::{AgentMsg, Metrics, ServerToAgentMsg, SystemInfo, PROTOCOL_VERSION};
use std::path::PathBuf;
use std::time::Duration;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System};
use tokio::time::{interval, sleep, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "pharus-agent", about = "Pharus monitoring agent")]
struct Args {
    /// Server WebSocket URL, e.g. wss://example.com/ws/agent
    #[arg(long, env = "PHARUS_SERVER")]
    server: Option<String>,

    /// Agent token issued by the server
    #[arg(long, env = "PHARUS_TOKEN")]
    token: Option<String>,

    /// Report interval in seconds
    #[arg(long, env = "PHARUS_INTERVAL")]
    interval: Option<u64>,

    /// Path to a TOML config file
    #[arg(long, env = "PHARUS_CONFIG")]
    config: Option<PathBuf>,
}

#[derive(Debug, serde::Deserialize)]
struct FileConfig {
    server: Option<String>,
    token: Option<String>,
    interval: Option<u64>,
}

struct Config {
    server: String,
    token: String,
    interval: u64,
}

impl Config {
    fn load(args: Args) -> Result<Self> {
        let file: Option<FileConfig> = match &args.config {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("read config {}", path.display()))?;
                Some(toml::from_str(&text).context("parse config toml")?)
            }
            None => None,
        };
        let pick = |cli: Option<String>, file: Option<String>, key: &str| -> Result<String> {
            cli.or(file).with_context(|| format!("missing required config: {key}"))
        };
        Ok(Config {
            server: pick(args.server, file.as_ref().and_then(|f| f.server.clone()), "server")?,
            token: pick(args.token, file.as_ref().and_then(|f| f.token.clone()), "token")?,
            interval: args
                .interval
                .or_else(|| file.as_ref().and_then(|f| f.interval))
                .unwrap_or(3),
        })
    }
}

fn collect_sysinfo(sys: &System) -> SystemInfo {
    let cpu_model = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "unknown".into());
    SystemInfo {
        hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
        os: System::long_os_version().unwrap_or_else(|| "unknown".into()),
        kernel: System::kernel_version().unwrap_or_else(|| "unknown".into()),
        arch: System::cpu_arch(),
        cpu_model,
        cpu_cores: sys.cpus().len(),
        virtualization: None,
    }
}

fn collect_metrics(sys: &System, disks: &Disks, rx_diff: u64, tx_diff: u64, interval_s: u64) -> Metrics {
    let cpu_usage = sys.global_cpu_usage();
    let mut disk_used = 0u64;
    let mut disk_total = 0u64;
    for d in disks.list() {
        disk_total = disk_total.saturating_add(d.total_space());
        disk_used = disk_used.saturating_add(d.total_space().saturating_sub(d.available_space()));
    }
    let load = System::load_average();
    Metrics {
        cpu_usage,
        mem_used: sys.used_memory(),
        mem_total: sys.total_memory(),
        swap_used: sys.used_swap(),
        swap_total: sys.total_swap(),
        disk_used,
        disk_total,
        net_rx_bps: rx_diff / interval_s.max(1),
        net_tx_bps: tx_diff / interval_s.max(1),
        load1: load.one,
        uptime: System::uptime(),
    }
}

async fn run_session(cfg: &Config) -> Result<()> {
    info!(server = %cfg.server, "connecting");
    let (ws, _) = connect_async(&cfg.server).await.context("ws connect failed")?;
    let (mut write, mut read) = ws.split();

    let auth = serde_json::to_string(&AgentMsg::Auth {
        token: cfg.token.clone(),
        version: PROTOCOL_VERSION,
    })?;
    write.send(Message::Text(auth)).await?;

    let reply = read
        .next()
        .await
        .context("connection closed before auth reply")??;
    let reply_text = reply.into_text()?;
    match serde_json::from_str::<ServerToAgentMsg>(&reply_text)? {
        ServerToAgentMsg::AuthOk { agent_id, name } => {
            info!(agent_id, name, "authenticated")
        }
        ServerToAgentMsg::AuthFail { reason } => {
            anyhow::bail!("auth failed: {reason}")
        }
    }

    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    let mut disks = Disks::new_with_refreshed_list();
    let mut networks = Networks::new_with_refreshed_list();

    sys.refresh_cpu_usage();
    sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;

    write
        .send(Message::Text(serde_json::to_string(&AgentMsg::SysInfo {
            info: collect_sysinfo(&sys),
        })?))
        .await?;

    let mut prev_rx: u64 = networks.list().values().map(|n| n.total_received()).sum();
    let mut prev_tx: u64 = networks.list().values().map(|n| n.total_transmitted()).sum();

    let mut tick = interval(Duration::from_secs(cfg.interval.max(1)));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // the first tick fires immediately; discard it so the first network
    // sample accumulates over a full interval
    tick.tick().await;

    let mut last_tick = std::time::Instant::now();
    loop {
        tick.tick().await;
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_tick).as_secs().max(1);
        last_tick = now;

        sys.refresh_cpu_usage();
        sys.refresh_memory();
        disks.refresh(true);
        networks.refresh(false);

        let rx: u64 = networks.list().values().map(|n| n.total_received()).sum();
        let tx: u64 = networks.list().values().map(|n| n.total_transmitted()).sum();
        let rx_diff = rx.saturating_sub(prev_rx);
        let tx_diff = tx.saturating_sub(prev_tx);
        prev_rx = rx;
        prev_tx = tx;

        let metrics = collect_metrics(&sys, &disks, rx_diff, tx_diff, elapsed);
        let msg = serde_json::to_string(&AgentMsg::Metrics { data: metrics })?;
        write.send(Message::Text(msg)).await?;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::load(Args::parse())?;
    info!(interval = cfg.interval, "pharus-agent starting");

    let mut backoff = 1u64;
    loop {
        let started = std::time::Instant::now();
        match run_session(&cfg).await {
            Ok(()) => warn!("session ended, reconnecting"),
            Err(e) => error!(error = %e, "session error, reconnecting"),
        }
        // a session that stayed up for a while was healthy: reset backoff
        if started.elapsed() >= Duration::from_secs(60) {
            backoff = 1;
        }
        sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}
