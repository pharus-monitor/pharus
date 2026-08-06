# Pharus 部署教程

本文档覆盖生产环境的完整部署流程：

- [一、部署 Server](#一部署-server)
  - [方式 A：单二进制 + systemd（推荐）](#方式-a单二进制--systemd推荐)
  - [方式 B：Docker / docker-compose](#方式-bdocker--docker-compose)
  - [配置 HTTPS 反向代理（Caddy / Nginx）](#配置-https-反向代理)
- [二、部署 Agent（被监控机）](#二部署-agent被监控机)
  - [方式 A：一键脚本](#方式-a一键脚本)
  - [方式 B：手动安装 + systemd](#方式-b手动安装--systemd)
  - [方式 C：Docker](#方式-cdocker)
  - [Windows 被监控机](#windows-被监控机)
- [三、验证与排错](#三验证与排错)
- [四、升级与备份](#四升级与备份)
- [五、安全建议](#五安全建议)

---

## 一、部署 Server

### 方式 A：单二进制 + systemd（推荐）

**1. 获取二进制**

从 [Releases](https://github.com/pharus-monitor/pharus/releases) 下载对应架构，或自行交叉编译：

```bash
# 在开发机上交叉编译 Linux x86_64 静态包（musl）
rustup target add x86_64-unknown-linux-musl
cargo build --release -p pharus --target x86_64-unknown-linux-musl
# 产物：target/x86_64-unknown-linux-musl/release/pharus
```

**2. 安装到服务器**

```bash
# 创建用户与目录
sudo useradd -r -s /usr/sbin/nologin pharus || true
sudo mkdir -p /etc/pharus /var/lib/pharus
sudo cp pharus /usr/local/bin/pharus
sudo chmod +x /usr/local/bin/pharus

# 拷贝主题目录（前端静态文件）
sudo cp -r server/themes /var/lib/pharus/themes

# 授权
sudo chown -R pharus:pharus /var/lib/pharus
```

**3. 创建第一个 Agent（拿到 token，后面装 Agent 要用）**

```bash
sudo -u pharus pharus add-agent --name my-first-vps --db /var/lib/pharus/pharus.db
# 输出 token = xxxxxxxx...  ← 记下来
```

**4. 配置 systemd**

```bash
sudo cp deploy/pharus-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now pharus-server
systemctl status pharus-server
```

服务默认监听 `0.0.0.0:8080`。改端口可编辑 unit 文件中的 `PHARUS_ADDR`。

**5. 防火墙放行**

```bash
# UFW
sudo ufw allow 8080/tcp
# firewalld
sudo firewall-cmd --permanent --add-port=8080/tcp && sudo firewall-cmd --reload
```

> 若使用 HTTPS 反向代理（见下），则**无需**对外放行 8080，让 unit 只监听
> `127.0.0.1:8080` 即可。

### 方式 B：Docker / docker-compose

```bash
git clone https://github.com/pharus-monitor/pharus.git
cd pharus

# 启动 Server
docker compose up -d

# 注册 Agent（在容器内执行，token 会打印出来）
docker compose exec server pharus add-agent --name my-first-vps --db /app/data/pharus.db
```

数据与主题通过卷持久化：

| 卷 | 内容 |
|---|---|
| `pharus-data` → `/app/data` | SQLite 数据库 |
| `./server/themes` → `/app/themes` | 主题目录（新增主题放进去即可） |

升级：

```bash
git pull
docker compose up -d --build
```

### 配置 HTTPS 反向代理

生产环境强烈建议套一层 TLS。WebSocket 在 HTTPS 下自动升级为 wss。

**Caddy（最省事，自动签发证书）**

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
        # WebSocket 必需的升级头
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 3600s;   # 长连接保活
    }
}
```

使用反代后，Agent 连接地址写 wss：

```
wss://mon.example.com/ws/agent
```

---

## 二、部署 Agent（被监控机）

前置条件：已经在 Server 上 `add-agent` 拿到 **token**。

### 方式 A：一键脚本

在**被监控机**上执行（Linux x86_64 / ARM64）：

```bash
curl -fsSL https://raw.githubusercontent.com/pharus-monitor/pharus/main/scripts/install-agent.sh | sudo bash -s -- \
  --server wss://mon.example.com/ws/agent \
  --token <你的token>
```

脚本会自动：下载对应架构的 `pharus-agent` → 写入 `/etc/pharus/agent.toml` →
安装并启动 systemd 服务。

### 方式 B：手动安装 + systemd

```bash
# 1. 放置二进制
sudo cp pharus-agent /usr/local/bin/pharus-agent
sudo chmod +x /usr/local/bin/pharus-agent

# 2. 写配置
sudo mkdir -p /etc/pharus
sudo tee /etc/pharus/agent.toml > /dev/null <<EOF
server = "wss://mon.example.com/ws/agent"
token = "<你的token>"
interval = 3
EOF
sudo chmod 600 /etc/pharus/agent.toml   # token 仅 root 可读

# 3. systemd
sudo cp deploy/pharus-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now pharus-agent

# 4. 查看日志确认已连上
journalctl -u pharus-agent -f
# 应看到：authenticated  agent_id=1
```

> Agent 无需任何入站端口，只要能**出站**访问 Server 即可（穿透 NAT / 家宽友好）。

### 方式 C：Docker

```bash
docker run -d --name pharus-agent --restart unless-stopped \
  -e PHARUS_SERVER=wss://mon.example.com/ws/agent \
  -e PHARUS_TOKEN=<你的token> \
  ghcr.io/pharus-monitor/pharus-agent:latest
```

> 容器内采集的是容器的指标。要监控**宿主机**，请用二进制方式部署；
> 或给容器挂载宿主机 `/proc`、`/sys`（只读）——后续版本会提供示例。

### Windows 被监控机

1. 下载 `pharus-agent-windows-x86_64.exe`
2. 直接用参数运行：

```powershell
.\pharus-agent.exe --server wss://mon.example.com/ws/agent --token <token>
```

3. 注册为开机自启服务（任选其一）：

```powershell
# 用 NSSM（推荐）
nssm install pharus-agent "C:\pharus\pharus-agent.exe" "--server wss://mon.example.com/ws/agent --token <token>"
nssm start pharus-agent

# 或任务计划程序（开机触发）
schtasks /create /tn "PharusAgent" /sc onstart /ru SYSTEM ^
  /tr "C:\pharus\pharus-agent.exe --server wss://mon.example.com/ws/agent --token <token>"
```

---

## 三、验证与排错

**验证闭环**

```bash
# Server 上：应返回 JSON，online: true
curl http://127.0.0.1:8080/api/status

# Agent 上：日志应有 authenticated
journalctl -u pharus-agent -n 20
```

浏览器打开面板地址，主机卡片应在 3 秒内亮起。

**常见问题**

| 现象 | 排查 |
|---|---|
| Agent 报 `auth failed: invalid token` | token 复制错 / 在错的 Server 上注册的 |
| Agent 一直 reconnecting | Server 地址是否可达：`curl -i <addr>/api/status`；反代是否配了 WS 升级头 |
| 面板显示在线但无数据 | 浏览器开 DevTools 看 `/api/stream` 是否 101 升级成功 |
| 反代后面板白屏/断开 | Nginx 缺 `Upgrade`/`Connection` 头，或 `proxy_read_timeout` 太短 |
| 数据不写入历史 | 历史每 60s 写一行，等一分钟再查 `pharus.db` 的 `metrics_history` 表 |

日志级别：两端都读 `RUST_LOG` 环境变量，如 `RUST_LOG=debug`。

---

## 四、升级与备份

**升级**

- 二进制方式：替换 `/usr/local/bin/pharus(-agent)` 后 `systemctl restart`
- Docker：`git pull && docker compose up -d --build`
- 协议字段向后兼容；两端版本不必强一致，但建议同步升级

**备份**

只需备份一个文件（WAL 模式下建议先停机或用 `.backup`）：

```bash
# 在线备份（安全）
sqlite3 /var/lib/pharus/pharus.db ".backup /backup/pharus-$(date +%F).db"
```

主题目录 `themes/` 如有自定义主题也一并备份。

---

## 五、安全建议

1. **永远走 wss**：公网部署务必套 TLS，token 明文走 ws 会被嗅探
2. **token 即凭据**：每台机器一个独立 token，泄露后删掉该行重建即可
3. **最小权限**：两端均以非 root 用户运行（unit 文件已配置 `User=pharus`）
4. **收窄监听**：有反代时 Server 只监听 `127.0.0.1:8080`
5. **数据库文件权限**：`chmod 600 pharus.db`，内含所有 agent token
