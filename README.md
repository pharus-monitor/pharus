# Pharus · Lighthouse

> **Pharus** (from Greek *pharos*, lighthouse) — a lightweight server monitoring
> system written in Rust. Every monitored machine is a point at sea; Pharus is
> the lighthouse that keeps watch and tells you who is online and how they are doing.

A lightweight server monitoring system written in Rust.

[简体中文](README.zh-CN.md)

## Features

- **Full Rust stack**: agent idles at 1–3 MB RSS, no GC pauses
- **Single binary**: zero runtime dependencies, statically linked (musl) for any VPS
- **Real-time push**: WebSocket both ways, no browser polling
- **SQLite storage**: single-file, zero-ops, 60s downsampled history
- **Theme system**: the whole frontend is a swappable template (`themes/` + `current_theme`)
- **Multilingual UI**: English / 中文 / 日本語 / Русский, auto-matched to browser language
- **Reconnect & offline detection**: exponential backoff 1s→30s; offline after 15s silence
- **Ping monitoring**: ICMP / TCP / HTTP probe tasks on schedules, with per-host latency & loss history charts (hover a curve to read it out)
- **Live network diagnostics**: Looking Glass (ping / traceroute), a live-updating MTR table, and iperf3 bandwidth tests — streamed from the agent to the browser
- **Streaming-unlock checks**: periodic Netflix / YouTube Premium / Disney+ / ChatGPT availability per host
- **Alert rules**: metric / host-offline / task-failure rules with evaluation windows, sample ratios and cooldowns; notifies via Bark, DingTalk, Discord, Email, Feishu, Telegram, Webhook or WeCom
- **Region grouping & drag reorder**: group host cards by region, drag cards and groups into your own order — saved server-side so every visitor sees the same layout
- **Admin console**: username/password login, billing management, ping tasks, custom script tasks, alert rules, channels, regions, feature toggles
- **Privacy by default**: host IPs are masked on the public dashboard, and the first hop (gateway) is hidden in MTR/traceroute output

## Architecture

The agent dials **out** to the server (NAT-friendly) over a single duplex WebSocket.
The server keeps live state in memory, writes downsampled history to SQLite every
60s, and pushes increments to browsers via `/api/stream`.

## Quick Start

Prebuilt static binaries (x86_64 / i686 / aarch64, musl — runs on any distro)
are on [Releases](https://github.com/pharus-monitor/pharus/releases):

```bash
# 0. Download and unpack the server (contains pharus + themes/ + deploy/)
curl -fLO https://github.com/pharus-monitor/pharus/releases/latest/download/pharus-linux-x86_64.tar.gz
tar -xzf pharus-linux-x86_64.tar.gz

# 1. Register a monitored machine, get a token
./pharus add-agent --name my-vps

# 2. Start the server (default 0.0.0.0:8080)
./pharus serve --themes themes

# 3. Start the agent on the monitored machine
./pharus-agent --server ws://<server>:8080/ws/agent --token <token>
```

Or install the agent with the one-line script (auto-detects arch, sets up systemd):

```bash
curl -fsSL https://raw.githubusercontent.com/pharus-monitor/pharus/main/scripts/install-agent.sh | sudo bash -s -- \
  --server ws://<server>:8080/ws/agent --token <token>
```

Open `http://<server>:8080` to see the live dashboard.

Building from source instead: `cargo build --release`.

> Full production deployment (systemd, Docker, HTTPS reverse proxy, Windows
> agents, troubleshooting): **[docs/deployment.md](docs/deployment.md)**

### Configuration

Both binaries accept CLI flags and environment variables:

| Variable | Description | Default |
|---|---|---|
| `PHARUS_ADDR` | Server listen address | `0.0.0.0:8080` |
| `PHARUS_DB` | SQLite path | `pharus.db` |
| `PHARUS_THEMES` | Themes root directory | `themes` |
| `PHARUS_SERVER` | Agent server URL | — |
| `PHARUS_TOKEN` | Agent token | — |
| `PHARUS_INTERVAL` | Report interval (seconds) | `3` |
| `PHARUS_ADMIN_USER` | Admin console username | `admin` |
| `PHARUS_ADMIN_PASSWORD` | Admin console password (admin API disabled if unset) | — |

The agent also supports a TOML config file (`--config agent.toml`):

```toml
server = "wss://example.com/ws/agent"
token = "..."
interval = 3
```

## Docker

```bash
docker compose up -d                    # server
docker compose --profile agent up -d    # optional same-host agent
```

`themes/` and the SQLite `data/` directory are persisted via volumes.

## Theme Development

A theme is a plain static directory (HTML/CSS/JS) with **no build step and no CDN
dependency**. The server hosts `themes/<current_theme>/` as the site root. Data
comes from the WebSocket `/api/stream` (JSON delta protocol), with REST
`/api/status` as a fallback. Translations live in `i18n/*.json` inside the theme.
See the bundled `server/themes/default/`.

Switch themes at runtime — no restart needed:

```bash
./pharus set-theme --name <theme>
```

## Billing & Traffic

The dashboard tracks per-agent billing details: cycle traffic with a monthly
reset day, an optional traffic quota with a usage bar (amber >80%, red >95%),
an expiry date with countdown (red ≤7 days), and a renewal price
(CNY/USD/EUR · monthly/quarterly/yearly). The header shows a monthly cost
summary grouped by currency.

Traffic is counted from cumulative byte counters reported by the agent, so
agent reboots don't lose usage. The cycle reset boundary and expiry dates are
interpreted in the server's local timezone. Agents from v0.0.x don't report
counters and simply show no traffic data.

Set `PHARUS_ADMIN_PASSWORD` on the server, then open `admin.html` (linked from the
dashboard header) and sign in to manage billing, tasks, alerts, regions and
feature toggles. Without it the admin API stays disabled and the dashboard is
read-only.

> Billing fields are visible on the public dashboard, like everything else.
> If prices are sensitive, put the whole site behind a reverse proxy with auth.

## Protocol

Shared message types live in the `common/` crate, JSON over WebSocket:

- `AgentMsg`: `auth` / `sys_info` / `metrics` / `ping` / `task_result` / `unlock` / `cmd_output` / `mtr_result` / `region`
- `ServerToAgentMsg`: `auth_ok` / `auth_fail` / `run_task` / task & feature sync
- `BrowserMsg`: `snapshot` / `metrics` / `status` / `billing` / `pings` / `unlock` / `diag_result` / `region_update` / `features_update`

## License

MIT
