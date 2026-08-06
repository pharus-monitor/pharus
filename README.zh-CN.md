# Pharus · 灯塔

> **Pharus**（古希腊语 *pharos*，灯塔）—— 用 Rust 编写的轻量级服务器监控系统。
> 每台被监控机如海上一个点，Pharus 是那座持续守望的灯塔。

一个用 Rust 编写的轻量级服务器监控系统。

[English](README.md)

## 特性（MVP）

- **Rust 全栈**：Agent 常驻内存 1–3MB 级别，无 GC 停顿
- **单二进制**：无运行时依赖，musl 静态链接，适配各种 VPS
- **实时推送**：WebSocket 双端推送，前端免轮询
- **SQLite 存储**：单文件零运维，60s 降采样历史
- **主题系统**：前端整模板可替换，`themes/` 加载 + `current_theme` 切换
- **多语言界面**：English / 中文 / 日本語 / Русский，自动匹配浏览器语言
- **断线重连**：指数退避 1s 到 30s 封顶；15s 无上报判定离线

路线图（二期/进阶）：Ping/TCPing 动态监控、自定义任务、Looking Glass / MTR / iperf3、
告警通知、流媒体解锁检测、主题商店等，详见设计大纲。

## 架构

Agent 主动出站连 Server（NAT 友好），单连接双向复用；Server 内存保存实时状态，
每 60s 降采样写入 SQLite，并经 `/api/stream` 把增量推给浏览器。

## 快速开始

预编译静态二进制（x86_64 / i686 / aarch64，musl，任何发行版可直接运行）见
[Releases](https://github.com/pharus-monitor/pharus/releases)：

```bash
# 0. 下载并解压 Server（内含 pharus + themes/ + deploy/）
curl -fLO https://github.com/pharus-monitor/pharus/releases/latest/download/pharus-linux-x86_64.tar.gz
tar -xzf pharus-linux-x86_64.tar.gz

# 1. 注册一台被监控机，得到 token
./pharus add-agent --name my-vps

# 2. 启动 Server（默认 0.0.0.0:8080）
./pharus serve --themes themes

# 3. 在被监控机上启动 Agent
./pharus-agent --server ws://<server>:8080/ws/agent --token <token>
```

也可以用一键脚本安装 Agent（自动识别架构并配置 systemd）：

```bash
curl -fsSL https://raw.githubusercontent.com/pharus-monitor/pharus/main/scripts/install-agent.sh | sudo bash -s -- \
  --server ws://<server>:8080/ws/agent --token <token>
```

打开 `http://<server>:8080` 即可看到实时面板。

如需从源码构建：`cargo build --release`。

> 生产环境完整部署（systemd、Docker、HTTPS 反代、Windows 被控端、排错）：
> **[docs/deployment.zh-CN.md](docs/deployment.zh-CN.md)**

### 配置

Server 与 Agent 均支持命令行参数与环境变量：

| 环境变量 | 说明 | 默认 |
|---|---|---|
| `PHARUS_ADDR` | Server 监听地址 | `0.0.0.0:8080` |
| `PHARUS_DB` | SQLite 路径 | `pharus.db` |
| `PHARUS_THEMES` | 主题根目录 | `themes` |
| `PHARUS_SERVER` | Agent 连接地址 | — |
| `PHARUS_TOKEN` | Agent 令牌 | — |
| `PHARUS_INTERVAL` | 上报间隔（秒） | `3` |
| `PHARUS_ADMIN_TOKEN` | 账单管理 API 的 Token（不设置则关闭） | — |

Agent 也支持 TOML 配置文件（`--config agent.toml`）：

```toml
server = "wss://example.com/ws/agent"
token = "..."
interval = 3
```

## Docker

```bash
docker compose up -d                    # server
docker compose --profile agent up -d    # 同机跑 agent（可选）
```

`themes/` 与 `data/`（SQLite）通过卷持久化。

## 主题开发

主题是纯静态文件目录（HTML/CSS/JS），无构建步骤、无 CDN 依赖。Server 托管
`themes/<current_theme>/` 作为站点根。数据通过 WebSocket `/api/stream` 获取
（JSON 增量协议），兜底 REST `/api/status`。翻译文件在主题内的 `i18n/*.json`。
参考内置 `server/themes/default/`。

运行时切换主题，无需重启：

```bash
./pharus set-theme --name <theme>
```

## 账单与流量

面板支持按机器记录账单信息：计费周期流量（可设每月重置日）、流量配额
（用量进度条，>80% 变黄、>95% 变红）、到期时间（倒计时，≤7 天变红）、
续费价格（人民币/美元/欧元，月付/季付/年付）。头部概览按币种汇总月成本。

流量由 agent 上报的开机累计字节数差值统计，agent 重启不丢用量。
周期重置边界与到期日均按 **server 本地时区** 解释。v0.0.x 旧版 agent
不上报计数器，仅无流量数据，其余功能不受影响。

在 server 上设置 `PHARUS_ADMIN_TOKEN` 后，点击面板右上角的齿轮图标即可
进入管理模式编辑每台机器。未设置时管理 API 返回 404，面板保持只读。

> 账单信息在公开面板上可见（与其他数据一致）。价格敏感时请将整站置于
> 带鉴权的反向代理之后。

## 协议

两端共享的消息结构定义在 `common/` crate，JSON over WebSocket：

- `AgentMsg`：`auth` / `sys_info` / `metrics`
- `ServerToAgentMsg`：`auth_ok` / `auth_fail`
- `BrowserMsg`：`snapshot` / `metrics` / `status`

## License

MIT
