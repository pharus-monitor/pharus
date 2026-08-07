use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use pharus_common::{
    AgentMsg, Metrics, PingResult, PingTarget, ServerToAgentMsg, SystemInfo, TaskKind,
    UnlockResult, PROTOCOL_VERSION,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System};
use tokio::sync::mpsc;
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

fn collect_metrics(sys: &System, disks: &Disks, rx_diff: u64, tx_diff: u64, rx_total: u64, tx_total: u64, interval_s: u64) -> Metrics {
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
        net_rx_total: rx_total,
        net_tx_total: tx_total,
    }
}

type MsgTx = mpsc::UnboundedSender<AgentMsg>;

struct Shared {
    tcping: Mutex<Vec<PingTarget>>,
}

/* ---------- TCPing ---------- */

async fn tcp_rtt(host: &str, port: u16) -> Option<f64> {
    let start = std::time::Instant::now();
    let addr = format!("{host}:{port}");
    match tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => Some(start.elapsed().as_secs_f64() * 1000.0),
        _ => None,
    }
}

async fn tcping_loop(msg_tx: MsgTx, shared: Arc<Shared>) {
    let mut tick = interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let targets = shared.tcping.lock().unwrap().clone();
        if targets.is_empty() {
            continue;
        }
        let mut results = Vec::with_capacity(targets.len());
        for t in targets {
            let rtt = tcp_rtt(&t.host, t.port).await;
            results.push(PingResult {
                label: t.label,
                rtt_ms: rtt,
                task_id: None,
                rtt_min: rtt,
                rtt_max: rtt,
                loss: if rtt.is_some() { 0.0 } else { 1.0 },
            });
        }
        if msg_tx.send(AgentMsg::Ping { results }).is_err() {
            return;
        }
    }
}

/* ---------- streaming-unlock checks ---------- */

struct UnlockCheck {
    service: &'static str,
    url: &'static str,
    /// 2xx considered reachable; 403 considered blocked
    want_substr: Option<&'static str>,
    /// extract "key":"XX" from body as detail
    detail_key: Option<&'static str>,
}

const UNLOCK_CHECKS: &[UnlockCheck] = &[
    UnlockCheck {
        service: "Netflix",
        url: "https://www.netflix.com/title/70143836",
        want_substr: None,
        detail_key: None,
    },
    UnlockCheck {
        service: "YouTube Premium",
        url: "https://www.youtube.com/premium",
        want_substr: Some("countryCode"),
        detail_key: Some("countryCode"),
    },
    UnlockCheck {
        service: "Disney+",
        url: "https://www.disneyplus.com",
        want_substr: None,
        detail_key: None,
    },
    UnlockCheck {
        service: "ChatGPT",
        url: "https://chat.openai.com/cdn-cgi/trace",
        want_substr: Some("loc="),
        detail_key: Some("loc"),
    },
];

fn extract_detail(body: &str, key: &str) -> Option<String> {
    // matches both `"key":"XX"` and `key=XX`
    for (idx, _) in body.match_indices(key) {
        let rest = &body[idx + key.len()..];
        let rest = rest.trim_start_matches(['"', '=']);
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 && end <= 8 {
            return Some(rest[..end].to_string());
        }
    }
    None
}

async fn run_unlock_checks(client: &reqwest::Client) -> Vec<UnlockResult> {
    let mut out = Vec::with_capacity(UNLOCK_CHECKS.len());
    for c in UNLOCK_CHECKS {
        let result = async {
            let resp = client.get(c.url).send().await?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Ok::<(u16, String), reqwest::Error>((status.as_u16(), body))
        }
        .await;
        let r = match result {
            Ok((code, body)) if (200..300).contains(&code) => {
                let ok = c.want_substr.map(|s| body.contains(s)).unwrap_or(true);
                UnlockResult {
                    service: c.service.into(),
                    status: if ok { "yes".into() } else { "no".into() },
                    detail: c.detail_key.and_then(|k| extract_detail(&body, k)),
                }
            }
            Ok((403, _)) => UnlockResult {
                service: c.service.into(),
                status: "no".into(),
                detail: None,
            },
            Ok((code, _)) => UnlockResult {
                service: c.service.into(),
                status: "fail".into(),
                detail: Some(format!("http {code}")),
            },
            Err(e) => UnlockResult {
                service: c.service.into(),
                status: "fail".into(),
                detail: Some(e.to_string()),
            },
        };
        out.push(r);
    }
    out
}

async fn unlock_loop(msg_tx: MsgTx, client: reqwest::Client) {
    // let metrics flow first after (re)connecting
    sleep(Duration::from_secs(5)).await;
    let mut tick = interval(Duration::from_secs(30 * 60));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let results = run_unlock_checks(&client).await;
        if msg_tx.send(AgentMsg::Unlock { results }).is_err() {
            return;
        }
    }
}

/* ---------- task execution (looking glass / script) ---------- */

fn task_command(kind: TaskKind, target: &str) -> Option<tokio::process::Command> {
    #[cfg(windows)]
    let cmd = match kind {
        TaskKind::Ping => {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "4", target]);
            c
        }
        TaskKind::Traceroute => {
            let mut c = std::process::Command::new("tracert");
            c.arg(target);
            c
        }
        TaskKind::Mtr => return None,
        TaskKind::Script => {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", target]);
            c
        }
    };
    #[cfg(not(windows))]
    let cmd = match kind {
        TaskKind::Ping => {
            let mut c = std::process::Command::new("ping");
            c.args(["-c", "4", "-W", "2", target]);
            c
        }
        TaskKind::Traceroute => {
            let mut c = std::process::Command::new("traceroute");
            c.arg(target);
            c
        }
        TaskKind::Mtr => {
            let mut c = std::process::Command::new("mtr");
            c.args(["-r", "-c", "4", target]);
            c
        }
        TaskKind::Script => {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", target]);
            c
        }
    };
    Some(tokio::process::Command::from(cmd))
}

async fn run_task(kind: TaskKind, target: &str) -> (i32, String) {
    let Some(mut cmd) = task_command(kind, target) else {
        return (-1, "mtr is not supported on Windows agents".into());
    };
    match tokio::time::timeout(Duration::from_secs(30), cmd.output()).await {
        Ok(Ok(o)) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.is_empty() {
                text.push_str(&stderr);
            }
            const MAX: usize = 32 * 1024;
            if text.len() > MAX {
                text.truncate(MAX);
                text.push_str("\n... (truncated)");
            }
            (o.status.code().unwrap_or(-1), text)
        }
        Ok(Err(e)) => (-1, format!("failed to run: {e}")),
        Err(_) => (-1, "task timed out after 30s".into()),
    }
}

/* ---------- session ---------- */

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
        _ => anyhow::bail!("unexpected first server message"),
    }

    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AgentMsg>();
    let shared = Arc::new(Shared { tcping: Mutex::new(Vec::new()) });
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
        .build()
        .context("build http client")?;

    // single writer: serializes every outgoing AgentMsg
    let writer = tokio::spawn(async move {
        while let Some(m) = msg_rx.recv().await {
            let Ok(s) = serde_json::to_string(&m) else { continue };
            if write.send(Message::Text(s)).await.is_err() {
                break;
            }
        }
    });

    // reader: server downlink (config pushes + tasks)
    let reader = {
        let msg_tx = msg_tx.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            while let Some(frame) = read.next().await {
                match frame {
                    Ok(Message::Text(t)) => match serde_json::from_str::<ServerToAgentMsg>(&t) {
                        Ok(ServerToAgentMsg::Config { tcping }) => {
                            info!(targets = tcping.len(), "tcping config updated");
                            *shared.tcping.lock().unwrap() = tcping;
                        }
                        Ok(ServerToAgentMsg::RunTask { task_id, kind, target, .. }) => {
                            let msg_tx = msg_tx.clone();
                            tokio::spawn(async move {
                                let (exit_code, output) = run_task(kind, &target).await;
                                let _ = msg_tx.send(AgentMsg::TaskResult {
                                    task_id,
                                    exit_code,
                                    output,
                                    scheduled_id: None,
                                });
                            });
                        }
                        _ => {}
                    },
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "ws read error");
                        break;
                    }
                }
            }
        })
    };

    let tcping = tokio::spawn(tcping_loop(msg_tx.clone(), shared.clone()));
    let unlock = tokio::spawn(unlock_loop(msg_tx.clone(), http));

    let metrics = metrics_loop(msg_tx.clone(), cfg);

    tokio::select! {
        _ = writer => anyhow::bail!("ws writer ended"),
        _ = reader => anyhow::bail!("ws reader ended"),
        _ = tcping => anyhow::bail!("tcping loop ended"),
        _ = unlock => anyhow::bail!("unlock loop ended"),
        r = metrics => r.map_err(|e| anyhow::anyhow!("metrics loop: {e}")),
    }
}

async fn metrics_loop(
    msg_tx: MsgTx,
    cfg: &Config,
) -> Result<(), mpsc::error::SendError<AgentMsg>> {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    let mut disks = Disks::new_with_refreshed_list();
    let mut networks = Networks::new_with_refreshed_list();

    sys.refresh_cpu_usage();
    sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;

    msg_tx.send(AgentMsg::SysInfo {
        info: collect_sysinfo(&sys),
    })?;

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

        let metrics = collect_metrics(&sys, &disks, rx_diff, tx_diff, rx, tx, elapsed);
        msg_tx.send(AgentMsg::Metrics { data: metrics })?;
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
