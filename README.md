# Pharus · Lighthouse

> **Pharus** (from Greek *pharos*, lighthouse) — a lightweight server monitoring
> system written in Rust. Every monitored machine is a point at sea; Pharus is
> the lighthouse that keeps watch and tells you who is online and how they are doing.

A lightweight server monitoring system written in Rust.

[简体中文](README.zh-CN.md)

## Features (MVP)

- **Full Rust stack**: agent idles at 1–3 MB RSS, no GC pauses
- **Single binary**: zero runtime dependencies, statically linked (musl) for any VPS
- **Real-time push**: WebSocket both ways, no browser polling
- **SQLite storage**: single-file, zero-ops, 60s downsampled history
- **Theme system**: the whole frontend is a swappable template (`themes/` + `current_theme`)
- **Multilingual UI**: English / 中文 / 日本語 / Русский, auto-matched to browser language
- **Reconnect & offline detection**: exponential backoff 1s→30s; offline after 15s silence

Roadmap (phase 2+): Ping/TCPing monitoring, custom script tasks, Looking Glass /
MTR / iperf3, alert notifications, streaming-unlock checks, theme store.

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
| `PHARUS_ADMIN_TOKEN` | Admin API token for billing management (disabled if unset) | — |

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

Set `PHARUS_ADMIN_TOKEN` on the server to enable the admin API, then click the
gear icon in the dashboard header to edit each agent. Without it the admin API
returns 404 and the dashboard stays read-only.

> Billing fields are visible on the public dashboard, like everything else.
> If prices are sensitive, put the whole site behind a reverse proxy with auth.

## Protocol

Shared message types live in the `common/` crate, JSON over WebSocket:

- `AgentMsg`: `auth` / `sys_info` / `metrics`
- `ServerToAgentMsg`: `auth_ok` / `auth_fail`
- `BrowserMsg`: `snapshot` / `metrics` / `status`

## License

MIT
