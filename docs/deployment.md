# Pharus Deployment Guide

This document covers the full production deployment workflow.
[中文版本](deployment.zh-CN.md)

- [1. Deploy the Server](#1-deploy-the-server)
  - [Option A: single binary + systemd (recommended)](#option-a-single-binary--systemd-recommended)
  - [Option B: Docker / docker-compose](#option-b-docker--docker-compose)
  - [HTTPS reverse proxy (Caddy / Nginx)](#https-reverse-proxy-caddy--nginx)
- [2. Deploy the Agent (monitored machines)](#2-deploy-the-agent-monitored-machines)
  - [Option A: one-line installer](#option-a-one-line-installer)
  - [Option B: manual install + systemd](#option-b-manual-install--systemd)
  - [Option C: Docker](#option-c-docker)
  - [Windows machines](#windows-machines)
- [3. Verification & Troubleshooting](#3-verification--troubleshooting)
- [4. Upgrade & Backup](#4-upgrade--backup)
- [5. Security Hardening](#5-security-hardening)

---

## 1. Deploy the Server

### Option A: single binary + systemd (recommended)

**1. Get the binary**

Download the matching architecture from [Releases](https://github.com/pharus-monitor/pharus/releases) (`linux-x86_64`, `linux-i686`, `linux-aarch64`), or cross-compile it yourself:

```bash
# Cross-compile a static Linux x86_64 build (musl)
rustup target add x86_64-unknown-linux-musl
cargo build --release -p pharus --target x86_64-unknown-linux-musl
# Output: target/x86_64-unknown-linux-musl/release/pharus
```

**2. Install on the server**

```bash
# Create user and directories
sudo useradd -r -s /usr/sbin/nologin pharus || true
sudo mkdir -p /etc/pharus /var/lib/pharus
sudo cp pharus /usr/local/bin/pharus
sudo chmod +x /usr/local/bin/pharus

# Copy the theme directory (frontend static files)
sudo cp -r themes /var/lib/pharus/themes

# Permissions
sudo chown -R pharus:pharus /var/lib/pharus
```

**3. Create the first agent (you need the token when installing agents)**

```bash
sudo -u pharus pharus add-agent --name my-first-vps --db /var/lib/pharus/pharus.db
# prints: token = xxxxxxxx...   <- save this
```

**4. Configure systemd**

```bash
sudo cp deploy/pharus-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now pharus-server
systemctl status pharus-server
```

The service listens on `0.0.0.0:8080` by default. Edit `PHARUS_ADDR` in the unit file to change the port.

**5. Open the firewall**

```bash
# UFW
sudo ufw allow 8080/tcp
# firewalld
sudo firewall-cmd --permanent --add-port=8080/tcp && sudo firewall-cmd --reload
```

> If you use an HTTPS reverse proxy (see below), you do **not** need to expose 8080
> publicly — bind the unit to `127.0.0.1:8080` instead.

### Option B: Docker / docker-compose

```bash
git clone https://github.com/pharus-monitor/pharus.git
cd pharus

# Start the server
docker compose up -d

# Register an agent (runs inside the container, token is printed)
docker compose exec server pharus add-agent --name my-first-vps --db /app/data/pharus.db
```

Data and themes are persisted via volumes:

| Volume | Contents |
|---|---|
| `pharus-data` → `/app/data` | SQLite database |
| `./server/themes` → `/app/themes` | Theme directory (drop new themes here) |

Upgrading:

```bash
git pull
docker compose up -d --build
```

### HTTPS reverse proxy (Caddy / Nginx)

A TLS layer is strongly recommended in production. WebSocket automatically upgrades to `wss` under HTTPS.

**Caddy (easiest, automatic certificates)**

```caddyfile
mon.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

**Nginx**

```nginx
server {
    listen 443 ssl;
    server_name mon.example.com;

    ssl_certificate     /etc/letsencrypt/live/mon.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/mon.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        # Headers required for WebSocket
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 3600s;   # keep long-lived connections alive
    }
}
```

With a reverse proxy in place, agents connect via `wss`:

```
wss://mon.example.com/ws/agent
```

---

## 2. Deploy the Agent (monitored machines)

Prerequisite: you have already run `add-agent` on the server and have a **token**.

### Option A: one-line installer

Run on the **monitored machine** (Linux x86_64 / i686 / ARM64):

```bash
curl -fsSL https://raw.githubusercontent.com/pharus-monitor/pharus/main/scripts/install-agent.sh | sudo bash -s -- \
  --server wss://mon.example.com/ws/agent \
  --token <your-token>
```

The script automatically: downloads the matching `pharus-agent` build → writes
`/etc/pharus/agent.toml` → installs and starts the systemd service.

### Option B: manual install + systemd

```bash
# 1. Install the binary
sudo cp pharus-agent /usr/local/bin/pharus-agent
sudo chmod +x /usr/local/bin/pharus-agent

# 2. Configuration
sudo mkdir -p /etc/pharus
sudo tee /etc/pharus/agent.toml > /dev/null <<EOF
server = "wss://mon.example.com/ws/agent"
token = "<your-token>"
interval = 3
EOF
sudo chmod 600 /etc/pharus/agent.toml   # token readable by root only

# 3. systemd
sudo cp deploy/pharus-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now pharus-agent

# 4. Check the logs to confirm it connected
journalctl -u pharus-agent -f
# You should see: authenticated  agent_id=1
```

> The agent needs **no inbound ports** — it only makes outbound connections to the
> server (NAT / home-connection friendly).

### Option C: Docker

```bash
docker run -d --name pharus-agent --restart unless-stopped \
  -e PHARUS_SERVER=wss://mon.example.com/ws/agent \
  -e PHARUS_TOKEN=<your-token> \
  ghcr.io/pharus-monitor/pharus-agent:latest
```

> A containerized agent reports the container's metrics. To monitor the **host**,
> install the binary instead, or mount the host's `/proc` and `/sys` read-only —
> a future release will ship a ready-made example.

### Windows machines

1. Download `pharus-agent-windows-x86_64.exe`
2. Run it directly with arguments:

```powershell
.\pharus-agent.exe --server wss://mon.example.com/ws/agent --token <token>
```

3. Register it as an auto-start service (either option):

```powershell
# Using NSSM (recommended)
nssm install pharus-agent "C:\pharus\pharus-agent.exe" "--server wss://mon.example.com/ws/agent --token <token>"
nssm start pharus-agent

# Or Task Scheduler (runs at boot)
schtasks /create /tn "PharusAgent" /sc onstart /ru SYSTEM ^
  /tr "C:\pharus\pharus-agent.exe --server wss://mon.example.com/ws/agent --token <token>"
```

---

## 3. Verification & Troubleshooting

**End-to-end check**

```bash
# On the server: should return JSON with online: true
curl http://127.0.0.1:8080/api/status

# On the agent: logs should show "authenticated"
journalctl -u pharus-agent -n 20
```

Open the dashboard in a browser — the host card should light up within 3 seconds.

**Common issues**

| Symptom | What to check |
|---|---|
| Agent logs `auth failed: invalid token` | Token copied incorrectly, or registered on the wrong server |
| Agent stuck in `reconnecting` | Is the server reachable: `curl -i <addr>/api/status`; did the reverse proxy forward the WS upgrade headers |
| Dashboard shows online but no data | Open DevTools and check whether `/api/stream` upgrades with a 101 |
| Blank/disconnecting dashboard behind a proxy | Nginx missing `Upgrade`/`Connection` headers, or `proxy_read_timeout` too short |
| No history being written | History is written once per 60s — wait a minute, then check the `metrics_history` table in `pharus.db` |

Log level: both binaries honor the `RUST_LOG` environment variable, e.g. `RUST_LOG=debug`.

---

## 4. Upgrade & Backup

**Upgrade**

- Binary installs: replace `/usr/local/bin/pharus(-agent)` and `systemctl restart`
- Docker: `git pull && docker compose up -d --build`
- Protocol fields are backward-compatible; versions do not need to match exactly, but upgrading both sides together is recommended

**Backup**

Only one file needs backing up (in WAL mode, stop the service first or use `.backup`):

```bash
# Safe online backup
sqlite3 /var/lib/pharus/pharus.db ".backup /backup/pharus-$(date +%F).db"
```

Also back up the `themes/` directory if you have custom themes.

---

## 5. Security Hardening

1. **Always use wss**: put TLS in front of any public deployment — tokens sent over plain `ws` can be sniffed
2. **Tokens are credentials**: use one token per machine; if one leaks, delete that row and re-register
3. **Least privilege**: run both sides as a non-root user (the unit files already set `User=pharus`)
4. **Tighten the bind address**: with a reverse proxy, the server should only listen on `127.0.0.1:8080`
5. **Database file permissions**: `chmod 600 pharus.db` — it contains every agent token
