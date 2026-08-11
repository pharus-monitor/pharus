use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{stream::FuturesUnordered, SinkExt, StreamExt};
use pharus_common::{
    AgentMsg, CustomTaskSpec, Metrics, MtrHop, PingKind, PingResult, PingTarget, PingTaskSpec,
    ServerToAgentMsg, SystemInfo, TaskKind, UnlockResult, PROTOCOL_VERSION,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
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

fn collect_sysinfo(sys: &System, mem_desc: Option<String>) -> SystemInfo {
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
        mem_desc,
        ips: collect_ips(),
    }
}

fn collect_ips() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let networks = sysinfo::Networks::new_with_refreshed_list();
    for data in networks.values() {
        for net in data.ip_networks() {
            let addr = net.addr;
            if addr.is_loopback() || addr.is_unspecified() {
                continue;
            }
            if let std::net::IpAddr::V4(v4) = addr {
                let o = v4.octets();
                let is_cgnat = o[0] == 100 && (64..=127).contains(&o[1]);
                if v4.is_private() || v4.is_link_local() || v4.is_broadcast() || is_cgnat {
                    continue;
                }
            } else if let std::net::IpAddr::V6(v6) = addr {
                let o = v6.octets();
                let is_link_local = o[0] == 0xfe && (o[1] & 0xc0) == 0x80;
                let is_unique_local = (o[0] & 0xfe) == 0xfc;
                if is_link_local || is_unique_local {
                    continue;
                }
            }
            let s = addr.to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Best-effort memory module description, e.g. "Samsung 4800MHz". Runs once at
/// first connect; a missing tool or missing permission yields None instead of
/// failing the session.
async fn collect_mem_desc() -> Option<String> {
    #[cfg(windows)]
    {
        let script = "(Get-CimInstance Win32_PhysicalMemory | Select-Object -First 1 | ForEach-Object { \"$($_.Manufacturer) $($_.ConfiguredClockSpeed)MHz\" })";
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
    #[cfg(not(windows))]
    {
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("dmidecode").args(["-t", "memory"]).output(),
        )
        .await
        .ok()?
        .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        // SMBIOS placeholders reported by VMs and unprogrammed boards carry no
        // information; fall back to the DIMM layout instead of showing "QEMU".
        const PLACEHOLDER_MFR: &[&str] = &[
            "QEMU", "KVM", "VMware", "Bochs", "Xen", "VirtualBox", "Oracle Corporation",
            "Microsoft Corporation", "Not Specified", "No Module Installed", "Unknown",
            "To Be Filled By O.E.M.",
        ];
        let mut manufacturer = None;
        let mut speed = None;
        let mut sizes: Vec<String> = Vec::new();
        for line in text.lines() {
            let l = line.trim();
            if let Some(v) = l.strip_prefix("Manufacturer:") {
                let v = v.trim();
                if !v.is_empty() && v != "Unknown" && manufacturer.is_none() {
                    manufacturer = Some(v.to_string());
                }
            } else if let Some(v) = l.strip_prefix("Speed:") {
                let v = v.trim();
                if !v.is_empty() && v != "Unknown" && speed.is_none() {
                    speed = Some(v.to_string());
                }
            } else if let Some(v) = l.strip_prefix("Size:") {
                let v = v.trim();
                if !v.is_empty() && v != "Unknown" && v != "No Module Installed" {
                    sizes.push(v.to_string());
                }
            }
        }
        if let Some(m) = &manufacturer {
            if PLACEHOLDER_MFR.iter().any(|p| m.eq_ignore_ascii_case(p)) {
                manufacturer = None;
            }
        }
        match (manufacturer, speed) {
            (Some(m), Some(s)) => Some(format!("{m} {s}")),
            (Some(m), None) => Some(m),
            (None, Some(s)) => Some(s),
            (None, None) => match sizes.len() {
                0 => None,
                1 => Some(sizes[0].clone()),
                _ if sizes.iter().all(|s| *s == sizes[0]) => {
                    Some(format!("{} x {}", sizes.len(), sizes[0]))
                }
                _ => Some(sizes.join(" + ")),
            },
        }
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

#[derive(Default)]
struct Shared {
    tcping: Mutex<Vec<PingTarget>>,
    ping_tasks: Mutex<Vec<PingTaskSpec>>,
    custom_tasks: Mutex<Vec<CustomTaskSpec>>,
}

/* ---------- probes ---------- */

async fn tcp_rtt(host: &str, port: u16) -> Option<f64> {
    let start = Instant::now();
    let addr = format!("{host}:{port}");
    match tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => Some(start.elapsed().as_secs_f64() * 1000.0),
        _ => None,
    }
}

async fn http_rtt(client: &reqwest::Client, target: &str) -> Option<f64> {
    let url = match has_http_scheme(target) {
        true => target.to_string(),
        false => format!("https://{target}"),
    };
    let start = Instant::now();
    match client.get(url).send().await {
        Ok(r) if r.status().as_u16() < 500 => Some(start.elapsed().as_secs_f64() * 1000.0),
        _ => None,
    }
}

fn has_http_scheme(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// `(avg, min, max, loss)` — loss is the fraction of probes that got no reply.
type Rtt = (Option<f64>, Option<f64>, Option<f64>, f64);

fn summarize(samples: &[f64], attempts: u32) -> Rtt {
    let loss = 1.0 - samples.len() as f64 / attempts.max(1) as f64;
    if samples.is_empty() {
        return (None, None, None, loss);
    }
    let sum: f64 = samples.iter().sum();
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let max = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (Some(sum / samples.len() as f64), Some(min), Some(max), loss)
}

/// Reads the summary block of `ping`. Handles both the Unix
/// `rtt min/avg/max/mdev = …` form and the Windows `Minimum = …ms` form.
fn parse_ping_summary(text: &str) -> Rtt {
    let mut loss = 1.0;
    for line in text.lines() {
        if let Some(pos) = line.find("% packet loss").or_else(|| line.find("% loss")) {
            let head = &line[..pos];
            let start = head
                .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
                .map(|i| i + 1)
                .unwrap_or(0);
            if let Ok(v) = head[start..].parse::<f64>() {
                loss = v / 100.0;
            }
        }
    }
    for line in text.lines() {
        if line.contains("min/avg/max") {
            if let Some((_, tail)) = line.split_once('=') {
                let nums: Vec<f64> = tail
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('/')
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if nums.len() >= 3 {
                    return (Some(nums[1]), Some(nums[0]), Some(nums[2]), loss);
                }
            }
        }
        if line.contains("Minimum =") && line.contains("Average =") {
            let nums: Vec<f64> = line
                .split(['=', ',', 'm'])
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if nums.len() >= 3 {
                return (Some(nums[2]), Some(nums[0]), Some(nums[1]), loss);
            }
        }
    }
    (None, None, None, loss)
}

async fn icmp_probe(target: &str, count: u32) -> Rtt {
    let (_, text) = run_task(TaskKind::Ping, target, Some(count), 30).await;
    parse_ping_summary(&text)
}

async fn probe_ping_task(spec: &PingTaskSpec, http: &reqwest::Client) -> PingResult {
    let count = spec.count.clamp(1, 20);
    let (avg, min, max, loss) = match spec.kind {
        PingKind::Icmp => icmp_probe(&spec.target, count).await,
        PingKind::Tcp => {
            let port = spec.port.unwrap_or(80);
            let mut samples = Vec::new();
            for _ in 0..count {
                if let Some(v) = tcp_rtt(&spec.target, port).await {
                    samples.push(v);
                }
            }
            summarize(&samples, count)
        }
        PingKind::Http => {
            let mut samples = Vec::new();
            for _ in 0..count {
                if let Some(v) = http_rtt(http, &spec.target).await {
                    samples.push(v);
                }
            }
            summarize(&samples, count)
        }
    };
    PingResult {
        label: spec.label.clone(),
        rtt_ms: avg,
        task_id: Some(spec.id),
        rtt_min: min,
        rtt_max: max,
        loss,
    }
}

/// Every report carries the agent's full current result set, so the server can
/// keep replacing rather than merging — a task deleted server-side simply stops
/// appearing here.
async fn ping_scheduler(msg_tx: MsgTx, shared: Arc<Shared>, http: reqwest::Client) {
    let mut next_run: HashMap<i64, Instant> = HashMap::new();
    let mut task_results: HashMap<i64, PingResult> = HashMap::new();
    let mut legacy: Vec<PingResult> = Vec::new();
    let mut legacy_at = Instant::now();

    let mut tick = interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let now = Instant::now();
        let mut changed = false;

        let targets = shared.tcping.lock().unwrap().clone();
        if targets.is_empty() && !legacy.is_empty() {
            legacy.clear();
            changed = true;
        } else if !targets.is_empty() && now >= legacy_at {
            legacy_at = now + Duration::from_secs(10);
            legacy.clear();
            for t in targets {
                let rtt = tcp_rtt(&t.host, t.port).await;
                legacy.push(PingResult {
                    label: t.label,
                    rtt_ms: rtt,
                    task_id: None,
                    rtt_min: rtt,
                    rtt_max: rtt,
                    loss: if rtt.is_some() { 0.0 } else { 1.0 },
                });
            }
            changed = true;
        }

        let specs = shared.ping_tasks.lock().unwrap().clone();
        let previous_results = task_results.len();
        task_results.retain(|id, _| specs.iter().any(|s| s.id == *id));
        changed |= task_results.len() != previous_results;
        next_run.retain(|id, _| specs.iter().any(|s| s.id == *id));
        for spec in &specs {
            let at = *next_run.entry(spec.id).or_insert(now);
            if now < at {
                continue;
            }
            next_run.insert(spec.id, Instant::now() + Duration::from_secs(spec.interval.max(5)));
            task_results.insert(spec.id, probe_ping_task(spec, &http).await);
            changed = true;
        }

        if changed {
            let mut results = legacy.clone();
            results.extend(task_results.values().cloned());
            if msg_tx.send(AgentMsg::Ping { results }).is_err() {
                return;
            }
        }
    }
}

async fn custom_task_scheduler(msg_tx: MsgTx, shared: Arc<Shared>) {
    let mut next_run: HashMap<i64, Instant> = HashMap::new();
    let mut running = FuturesUnordered::new();
    let mut running_ids = HashSet::new();
    let mut tick = interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            Some(task_id) = running.next(), if !running.is_empty() => {
                running_ids.remove(&task_id);
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let specs = shared.custom_tasks.lock().unwrap().clone();
                next_run.retain(|id, _| specs.iter().any(|s| s.id == *id));
                // interval 0 means the task only ever runs when an operator triggers it
                for spec in specs.iter().filter(|s| s.interval > 0) {
                    let at = *next_run.entry(spec.id).or_insert(now);
                    if now < at {
                        continue;
                    }
                    next_run.insert(spec.id, now + Duration::from_secs(spec.interval.max(10)));
                    if !running_ids.insert(spec.id) {
                        warn!(task_id = spec.id, "scheduled task still running; skipping tick");
                        continue;
                    }
                    let msg_tx = msg_tx.clone();
                    let spec = spec.clone();
                    running.push(async move {
                        let (exit_code, output) =
                            run_task(TaskKind::Script, &spec.command, None, spec.timeout).await;
                        let _ = msg_tx.send(AgentMsg::TaskResult {
                            task_id: format!("sched-{}", spec.id),
                            exit_code,
                            output,
                            scheduled_id: Some(spec.id),
                        });
                        spec.id
                    });
                }
            }
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

async fn run_unlock_checks(
    client: &reqwest::Client,
    prev: &HashMap<String, UnlockResult>,
) -> Vec<UnlockResult> {
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
            // No HTTP response at all (timeout/reset/TLS): transient, keep
            // showing the last known state instead of a spurious failure.
            Err(e) => match prev.get(c.service) {
                Some(p) => p.clone(),
                None => UnlockResult {
                    service: c.service.into(),
                    status: "fail".into(),
                    detail: Some(e.to_string()),
                },
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
    let mut last: HashMap<String, UnlockResult> = HashMap::new();
    loop {
        tick.tick().await;
        let results = run_unlock_checks(&client, &last).await;
        for r in &results {
            last.insert(r.service.clone(), r.clone());
        }
        if msg_tx.send(AgentMsg::Unlock { results }).is_err() {
            return;
        }
    }
}

/* ---------- task execution (looking glass / script) ---------- */

/// The target is always an argv entry, never interpolated into a shell string,
/// so nothing here can be turned into extra arguments or commands.
fn task_command(kind: TaskKind, target: &str, cycles: Option<u32>) -> Option<tokio::process::Command> {
    let cycles = cycles.unwrap_or(4).clamp(1, 30).to_string();
    #[cfg(windows)]
    let cmd = match kind {
        TaskKind::Ping => {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", &cycles, target]);
            c
        }
        TaskKind::Traceroute => {
            let mut c = std::process::Command::new("tracert");
            c.arg(target);
            c
        }
        TaskKind::Mtr => return None,
        TaskKind::Iperf3 => return None,
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
            c.args(["-c", &cycles, "-W", "2", target]);
            c
        }
        TaskKind::Traceroute => {
            let mut c = std::process::Command::new("traceroute");
            c.arg(target);
            c
        }
        TaskKind::Mtr => {
            let mut c = std::process::Command::new("mtr");
            // --raw streams one line per probe, enabling live table updates
            c.args(["--raw", "-c", &cycles, target]);
            c
        }
        TaskKind::Iperf3 => return None,
        TaskKind::Script => {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", target]);
            c
        }
    };
    let mut cmd = tokio::process::Command::from(cmd);
    if kind == TaskKind::Ping {
        cmd.env("LC_ALL", "C").env("LANG", "C");
    }
    Some(cmd)
}

const TASK_OUTPUT_CAP: usize = 32 * 1024;
const TASK_OUTPUT_TRUNCATED: &str = "\n... (truncated)";

#[derive(Default)]
struct TaskOutputBuffer {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

impl TaskOutputBuffer {
    fn append(&mut self, bytes: &[u8], stderr: bool) {
        let used = self.stdout.len().saturating_add(self.stderr.len());
        let kept = bytes.len().min(TASK_OUTPUT_CAP.saturating_sub(used));
        if stderr {
            self.stderr.extend_from_slice(&bytes[..kept]);
        } else {
            self.stdout.extend_from_slice(&bytes[..kept]);
        }
        self.truncated |= kept < bytes.len();
    }
}

async fn read_task_output<R>(
    mut reader: R,
    output: Arc<Mutex<TaskOutputBuffer>>,
    stderr: bool,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        output.lock().unwrap().append(&chunk[..read], stderr);
    }
}

fn truncate_to_char_boundary(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let new_len = text
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|&index| index <= max)
        .last()
        .unwrap_or(0);
    text.truncate(new_len);
    true
}

fn task_output_text(output: &TaskOutputBuffer) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let text_truncated = truncate_to_char_boundary(&mut text, TASK_OUTPUT_CAP);
    if output.truncated || text_truncated {
        text.push_str(TASK_OUTPUT_TRUNCATED);
    }
    text
}

async fn run_task(
    kind: TaskKind,
    target: &str,
    cycles: Option<u32>,
    timeout: u64,
) -> (i32, String) {
    let Some(mut cmd) = task_command(kind, target, cycles) else {
        return (-1, "mtr is not supported on Windows agents".into());
    };
    let timeout = timeout.clamp(1, 600);
    cmd.kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return (-1, format!("failed to run: {e}")),
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill().await;
        return (-1, "failed to run: unable to capture task output".into());
    };

    let output = Arc::new(Mutex::new(TaskOutputBuffer::default()));
    let execution = async {
        let (status, _, _) = tokio::try_join!(
            child.wait(),
            read_task_output(stdout, output.clone(), false),
            read_task_output(stderr, output.clone(), true),
        )?;
        Ok::<_, std::io::Error>(status)
    };
    match tokio::time::timeout(Duration::from_secs(timeout), execution).await {
        Ok(Ok(status)) => {
            let text = task_output_text(&output.lock().unwrap());
            (status.code().unwrap_or(-1), text)
        }
        Ok(Err(e)) => {
            let _ = child.kill().await;
            (-1, format!("failed to run: {e}"))
        }
        Err(_) => {
            let _ = child.kill().await;
            (-1, format!("task timed out after {timeout}s"))
        }
    }
}

/// Per-hop state accumulated from `mtr --raw` lines.
#[derive(Default)]
struct RawHop {
    /// Address from the `h` line, used to recognize the target hop.
    ip: String,
    host: String,
    sent: u32,
    rtts: Vec<f64>,
}

/// Parses one `mtr --raw` line into the hop table. Returns true when the table
/// changed. Format: `x <hop> <seq>` (probe sent), `h <hop> <ip>`,
/// `d <hop> <name>` (DNS, preferred for display), `p <hop> <us> <seq>` (reply).
fn parse_mtr_raw(line: &str, hops: &mut std::collections::BTreeMap<u32, RawHop>) -> bool {
    let mut it = line.split_whitespace();
    let tag = it.next().unwrap_or("");
    let Some(idx) = it.next().and_then(|s| s.parse::<u32>().ok()) else {
        return false;
    };
    let hop = hops.entry(idx).or_default();
    match tag {
        "x" => hop.sent += 1,
        "h" => {
            let v = it.next().unwrap_or("");
            if hop.ip.is_empty() {
                hop.ip = v.into();
            }
            if hop.host.is_empty() {
                hop.host = v.into();
            }
        }
        "d" => {
            let v = it.next().unwrap_or("");
            if !v.is_empty() {
                hop.host = v.into();
            }
        }
        "p" => {
            if let Some(us) = it.next().and_then(|s| s.parse::<f64>().ok()) {
                hop.rtts.push(us / 1000.0);
            }
        }
        _ => return false,
    }
    true
}

fn raw_hops_snapshot(
    hops: &std::collections::BTreeMap<u32, RawHop>,
    target_ips: &[std::net::IpAddr],
) -> Vec<MtrHop> {
    // Like CLI mtr, the table stops at the target: later hops are duplicate
    // answers from the target itself and only confuse the readout.
    let cut = if target_ips.is_empty() {
        None
    } else {
        hops
            .iter()
            .find(|(_, h)| target_ips.iter().any(|ip| ip.to_string() == h.ip))
            .map(|(idx, _)| *idx)
    };
    hops
        .iter()
        .take_while(|(idx, _)| cut.map_or(true, |c| **idx <= c))
        .map(|(idx, h)| {
            let n = h.rtts.len() as f64;
            let (mut best, mut worst, mut sum, mut sq) = (f64::MAX, 0.0f64, 0.0, 0.0);
            for &r in &h.rtts {
                best = best.min(r);
                worst = worst.max(r);
                sum += r;
                sq += r * r;
            }
            let avg = if n > 0.0 { sum / n } else { 0.0 };
            let stdev = if n > 0.0 { (sq / n - avg * avg).max(0.0).sqrt() } else { 0.0 };
            MtrHop {
                hop: idx + 1,
                // the first hop is this machine's gateway — never disclose it
                host: if *idx == 0 { "***".into() } else { h.host.clone() },
                loss: if h.sent > 0 {
                    (1.0 - n / h.sent as f64).max(0.0)
                } else {
                    0.0
                },
                sent: h.sent,
                avg,
                best: if n > 0.0 { best } else { 0.0 },
                worst,
                stdev,
                last: h.rtts.last().copied().unwrap_or(0.0),
            }
        })
        .collect()
}

/// Mask the first hop of traceroute output: ` 1  gateway (1.2.3.4)  0.4 ms …`
/// keeps the hop number and timings but hides the gateway address. Windows
/// tracert prints timings as `<1 ms`, so `<…` tokens stay visible too.
fn mask_traceroute_hop1(line: &str) -> String {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("1 ") {
        return line.to_string();
    }
    let mut out = String::from(" 1 ");
    for tok in trimmed.split_whitespace().skip(1) {
        let visible =
            tok == "*" || tok == "ms" || tok.starts_with('<') || tok.parse::<f64>().is_ok();
        out.push_str(if visible { tok } else { "***" });
        out.push(' ');
    }
    out.trim_end().to_string()
}

/// Per-stream output cap for a streamed diagnostic.
const STREAM_CAP: usize = 128 * 1024;

fn spawn_pump<R>(
    reader: R,
    msg_tx: MsgTx,
    request_id: String,
    stream: &'static str,
    mask_first_hop: bool,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        let mut used = 0usize;
        while let Ok(Some(line)) = lines.next_line().await {
            if used >= STREAM_CAP {
                continue;
            }
            used += line.len() + 1;
            let line = if mask_first_hop {
                mask_traceroute_hop1(&line)
            } else {
                line
            };
            let sent = msg_tx.send(AgentMsg::CmdOutput {
                request_id: request_id.clone(),
                stream: stream.into(),
                data: format!("{line}\n"),
                done: false,
                exit_code: None,
            });
            if sent.is_err() {
                return;
            }
        }
    })
}

/// Runs a browser-initiated diagnostic, streaming output back as it arrives.
/// MTR is reported as one structured result instead, since its report is only
/// meaningful once complete.
async fn stream_task(
    msg_tx: MsgTx,
    request_id: String,
    kind: TaskKind,
    target: String,
    cycles: Option<u32>,
    extra: Option<serde_json::Value>,
) {
    let finish = |data: String, exit_code: i32| AgentMsg::CmdOutput {
        request_id: request_id.clone(),
        stream: "stderr".into(),
        data,
        done: true,
        exit_code: Some(exit_code),
    };

    if kind == TaskKind::Mtr {
        stream_mtr(msg_tx, request_id, &target, cycles).await;
        return;
    }
    if kind == TaskKind::Iperf3 {
        stream_iperf3(msg_tx, request_id, &target, extra).await;
        return;
    }

    let Some(mut cmd) = task_command(kind, &target, cycles) else {
        let _ = msg_tx.send(finish("unsupported diagnostic on this platform".into(), -1));
        return;
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = msg_tx.send(finish(format!("failed to run: {e}\n"), -1));
            return;
        }
    };

    let mask = kind == TaskKind::Traceroute;
    let pumps = [
        child.stdout.take().map(|r| spawn_pump(r, msg_tx.clone(), request_id.clone(), "stdout", mask)),
        child.stderr.take().map(|r| spawn_pump(r, msg_tx.clone(), request_id.clone(), "stderr", mask)),
    ];

    let exit_code = match tokio::time::timeout(Duration::from_secs(120), child.wait()).await {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        Ok(Err(_)) => -1,
        Err(_) => {
            let _ = child.kill().await;
            -1
        }
    };
    // drain both pumps first so the terminal frame really is last
    for p in pumps.into_iter().flatten() {
        let _ = p.await;
    }
    let _ = msg_tx.send(finish(String::new(), exit_code));
}

/// Runs iperf3 in JSON mode (`-J`), streaming the raw output back and ending
/// with a parsed structured result. A missing binary degrades to a message.
async fn stream_iperf3(
    msg_tx: MsgTx,
    request_id: String,
    target: &str,
    extra: Option<serde_json::Value>,
) {
    let finish = |data: String, exit_code: i32| AgentMsg::CmdOutput {
        request_id: request_id.clone(),
        stream: "stderr".into(),
        data,
        done: true,
        exit_code: Some(exit_code),
    };
    let port = extra
        .as_ref()
        .and_then(|e| e.get("port"))
        .and_then(|v| v.as_u64())
        .unwrap_or(5201)
        .clamp(1, 65535);
    let direction = extra
        .as_ref()
        .and_then(|e| e.get("direction"))
        .and_then(|v| v.as_str())
        .unwrap_or("down")
        .to_string();
    let duration = extra
        .as_ref()
        .and_then(|e| e.get("duration"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 15);
    let parallel = extra
        .as_ref()
        .and_then(|e| e.get("parallel"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4)
        .clamp(1, 16);
    let down = direction == "down";
    let protocol = extra
        .as_ref()
        .and_then(|e| e.get("protocol"))
        .and_then(|v| v.as_str())
        .unwrap_or("tcp")
        .to_string();
    let length = extra
        .as_ref()
        .and_then(|e| e.get("length"))
        .and_then(|v| v.as_u64());

    let mut cmd = tokio::process::Command::new("iperf3");
    cmd.arg("-J")
        .arg("-c")
        .arg(target)
        .arg("-p")
        .arg(port.to_string())
        .arg("-t")
        .arg(duration.to_string())
        .arg("-P")
        .arg(parallel.to_string());
    if protocol == "udp" {
        // UDP tests require a target bitrate; 1 Gbps is a sensible default
        // when the caller did not pick a specific one.
        cmd.arg("-u").arg("-b").arg("1G");
    }
    if let Some(length) = length {
        cmd.arg("-l").arg(length.to_string());
    }
    if down {
        cmd.arg("-R");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = msg_tx.send(finish(
                format!("iperf3 is not installed on this agent ({e})\n"),
                -1,
            ));
            return;
        }
    };

    let stdout = read_pipe_string(child.stdout.take()).await;
    let stderr = read_pipe_string(child.stderr.take()).await;
    let exit_code = match tokio::time::timeout(Duration::from_secs(duration + 30), child.wait()).await {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        Ok(Err(_)) => -1,
        Err(_) => {
            let _ = child.kill().await;
            -1
        }
    };
    let text = if stdout.trim().is_empty() { stderr } else { stdout };
    if exit_code != 0 {
        let _ = msg_tx.send(finish(text.clone(), exit_code));
        return;
    }
    // A successful run ends with the structured result; the raw -J JSON is
    // deliberately not streamed so the browser shows a clean summary.
    let (bps, retrans, secs) = parse_iperf3_json(&text, &direction);
    let _ = msg_tx.send(AgentMsg::Iperf3Result {
        request_id,
        direction,
        throughput_bps: bps,
        retransmits: retrans,
        duration_s: secs,
    });
}

async fn read_pipe_string<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut buf = String::new();
    let _ = tokio::io::AsyncReadExt::read_to_string(&mut pipe, &mut buf).await;
    buf
}

/// Extracts the aggregate throughput from `iperf3 -J` output. Download (-R)
/// reports on `sum_received`, upload on `sum_sent`.
fn parse_iperf3_json(text: &str, direction: &str) -> (Option<f64>, Option<u32>, Option<f64>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return (None, None, None);
    };
    let sum = v
        .get("end")
        .and_then(|e| {
            if direction == "up" {
                e.get("sum_sent")
            } else {
                e.get("sum_received")
            }
        });
    let Some(sum) = sum else {
        return (None, None, None);
    };
    let bps = sum.get("bits_per_second").and_then(|b| b.as_f64());
    let retrans = sum.get("retransmits").and_then(|r| r.as_u64()).map(|r| r as u32);
    let secs = sum.get("seconds").and_then(|s| s.as_f64());
    (bps, retrans, secs)
}

/// Streams a live MTR table: parses `mtr --raw` lines as they arrive and sends
/// progressive snapshots (done=false, throttled) plus one terminal snapshot.
/// The run ends as soon as the *target* hop has been probed `cycles` times —
/// mtr's own -c counts discovery rounds, so a slowly-discovered target would
/// otherwise see far fewer probes than requested.
async fn stream_mtr(msg_tx: MsgTx, request_id: String, target: &str, cycles: Option<u32>) {
    let finish = |data: String, exit_code: i32| AgentMsg::CmdOutput {
        request_id: request_id.clone(),
        stream: "stderr".into(),
        data,
        done: true,
        exit_code: Some(exit_code),
    };
    let wanted = cycles.unwrap_or(10).clamp(1, 30);
    // Resolve now so the target hop can be recognized by its h-line address.
    let target_ips: Vec<std::net::IpAddr> = match target.parse() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((target, 0))
            .await
            .map(|v| v.map(|s| s.ip()).collect())
            .unwrap_or_default(),
    };
    // Headroom for TTL ramp-up: up to 30 hops of discovery, then `wanted`
    // probes on the target. The early kill below is what normally ends the run.
    let mtr_cycles = wanted + 30;
    let Some(mut cmd) = task_command(TaskKind::Mtr, target, Some(mtr_cycles)) else {
        let _ = msg_tx.send(finish("mtr is not supported on Windows agents".into(), -1));
        return;
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = msg_tx.send(finish(format!("failed to run: {e}\n"), -1));
            return;
        }
    };
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let mut stderr_task = None;
    if let Some(err) = child.stderr.take() {
        let shared = stderr_buf.clone();
        stderr_task = Some(tokio::spawn(async move {
            let mut r = BufReader::new(err);
            let mut tmp = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut r, &mut tmp).await;
            // keep the tail small; surfaced only when no hops parsed
            let start = tmp.len().saturating_sub(4096);
            shared.lock().unwrap().push_str(&tmp[start..]);
        }));
    }
    let mut hops = std::collections::BTreeMap::new();
    let mut dirty = false;
    let mut last_push = Instant::now() - Duration::from_secs(1);
    let max_secs = wanted as u64 * 3 + 120;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut timed_out = false;
    loop {
        let next = tokio::time::timeout(Duration::from_secs(max_secs), lines.next_line()).await;
        match next {
            Ok(Ok(Some(line))) => {
                dirty |= parse_mtr_raw(&line, &mut hops);
                if dirty && last_push.elapsed() >= Duration::from_millis(300) {
                    let _ = msg_tx.send(AgentMsg::MtrResult {
                        request_id: request_id.clone(),
                        hubs: raw_hops_snapshot(&hops, &target_ips),
                        done: false,
                    });
                    dirty = false;
                    last_push = Instant::now();
                }
                let target_done = !target_ips.is_empty()
                    && hops
                        .values()
                        .find(|h| target_ips.iter().any(|ip| ip.to_string() == h.ip))
                        .map(|h| h.rtts.len() as u32 >= wanted)
                        .unwrap_or(false);
                if target_done {
                    let _ = child.kill().await;
                    break;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => {
                timed_out = true;
                let _ = child.kill().await;
                break;
            }
        }
    }
    let exit_code = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        _ => -1,
    };
    if hops.is_empty() {
        // make sure the stderr pump had its last scheduling slice
        if let Some(t) = stderr_task {
            let _ = tokio::time::timeout(Duration::from_secs(2), t).await;
        }
        let stderr = stderr_buf.lock().unwrap().clone();
        let detail = if timed_out { "mtr timed out\n".to_string() } else { stderr };
        let _ = msg_tx.send(finish(detail, exit_code));
        return;
    }
    let _ = msg_tx.send(AgentMsg::MtrResult {
        request_id,
        hubs: raw_hops_snapshot(&hops, &target_ips),
        done: true,
    });
}

/// Best-effort country lookup, reported once per connection.
async fn detect_region(client: &reqwest::Client) -> Option<String> {
    if let Ok(r) = client.get("https://www.cloudflare.com/cdn-cgi/trace").send().await {
        if let Ok(body) = r.text().await {
            if let Some(code) = body.lines().find_map(|l| l.strip_prefix("loc=")) {
                if code.len() == 2 {
                    return Some(code.to_uppercase());
                }
            }
        }
    }
    let text = client.get("https://ipinfo.io/country").send().await.ok()?.text().await.ok()?;
    let code = text.trim();
    (code.len() == 2).then(|| code.to_uppercase())
}

/* ---------- session ---------- */

struct AbortTasksOnDrop(Vec<tokio::task::AbortHandle>);

impl Drop for AbortTasksOnDrop {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

async fn run_session(cfg: &Config) -> Result<()> {
    info!(server = %cfg.server, "connecting");
    let (ws, _) = connect_async(&cfg.server).await.context("ws connect failed")?;
    let (mut write, mut read) = ws.split();

    let auth = serde_json::to_string(&AgentMsg::Auth {
        token: cfg.token.clone(),
        version: PROTOCOL_VERSION,
        name: sysinfo::System::host_name(),
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
    let shared = Arc::new(Shared::default());
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
                        Ok(ServerToAgentMsg::TasksSync { ping_tasks, custom_tasks }) => {
                            info!(
                                pings = ping_tasks.len(),
                                tasks = custom_tasks.len(),
                                "task list synced"
                            );
                            *shared.ping_tasks.lock().unwrap() = ping_tasks;
                            *shared.custom_tasks.lock().unwrap() = custom_tasks;
                        }
                        // Script runs return one buffered result; the network
                        // diagnostics stream so the browser sees them live.
                        Ok(ServerToAgentMsg::RunTask {
                            task_id,
                            kind: TaskKind::Script,
                            target,
                            cycles: _,
                            timeout,
                            extra: _,
                        }) => {
                            let msg_tx = msg_tx.clone();
                            tokio::spawn(async move {
                                let (exit_code, output) =
                                    run_task(TaskKind::Script, &target, None, timeout.unwrap_or(30))
                                        .await;
                                let _ = msg_tx.send(AgentMsg::TaskResult {
                                    task_id,
                                    exit_code,
                                    output,
                                    scheduled_id: None,
                                });
                            });
                        }
                        Ok(ServerToAgentMsg::RunTask {
                            task_id,
                            kind,
                            target,
                            cycles,
                            timeout: _,
                            extra,
                        }) => {
                            tokio::spawn(stream_task(
                                msg_tx.clone(),
                                task_id,
                                kind,
                                target,
                                cycles,
                                extra,
                            ));
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

    // region lookup is best effort and must never delay reporting
    {
        let msg_tx = msg_tx.clone();
        let http = http.clone();
        tokio::spawn(async move {
            if let Some(code) = detect_region(&http).await {
                let _ = msg_tx.send(AgentMsg::Region { code });
            }
        });
    }

    let pings = tokio::spawn(ping_scheduler(msg_tx.clone(), shared.clone(), http.clone()));
    let tasks = tokio::spawn(custom_task_scheduler(msg_tx.clone(), shared.clone()));
    let unlock = tokio::spawn(unlock_loop(msg_tx.clone(), http));
    // Whichever task ends first aborts the session, and the rest would otherwise
    // keep running against a socket nobody reads while the next attempt opens a
    // second one.
    let _session = AbortTasksOnDrop(vec![
        writer.abort_handle(),
        reader.abort_handle(),
        pings.abort_handle(),
        tasks.abort_handle(),
        unlock.abort_handle(),
    ]);

    let metrics = metrics_loop(msg_tx.clone(), cfg);

    tokio::select! {
        _ = writer => anyhow::bail!("ws writer ended"),
        _ = reader => anyhow::bail!("ws reader ended"),
        _ = pings => anyhow::bail!("ping scheduler ended"),
        _ = tasks => anyhow::bail!("task scheduler ended"),
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

    let mem_desc = collect_mem_desc().await;
    msg_tx.send(AgentMsg::SysInfo {
        info: collect_sysinfo(&sys, mem_desc),
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
    // rustls 0.23 builds without a process-level default provider when feature
    // unification disables defaults; install ring explicitly or wss:// panics.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_truncation_uses_previous_char_boundary() {
        let max = 32 * 1024;
        let mut text = "a".repeat(max - 1);
        text.push('界');

        assert!(truncate_to_char_boundary(&mut text, max));
        assert_eq!(text.len(), max - 1);
    }

    #[test]
    fn http_scheme_requires_separator() {
        assert!(has_http_scheme("http://example.com"));
        assert!(has_http_scheme("https://example.com"));
        assert!(!has_http_scheme("httpbin.org"));
        assert!(!has_http_scheme("http.example.com"));
    }

    #[test]
    fn task_output_buffer_caps_combined_streams() {
        let mut output = TaskOutputBuffer::default();
        output.append(&vec![b'a'; TASK_OUTPUT_CAP], false);
        output.append(b"stderr", true);

        assert_eq!(output.stdout.len() + output.stderr.len(), TASK_OUTPUT_CAP);
        assert!(output.truncated);
    }
}
