#!/usr/bin/env bash
# Pharus Agent 一键安装脚本
# 用法:
#   curl -fsSL .../install-agent.sh | sudo bash -s -- --server wss://mon.example.com/ws/agent --token <token>
set -euo pipefail

REPO="pharus-monitor/pharus"
INSTALL_BIN="/usr/local/bin/pharus-agent"
CONFIG_DIR="/etc/pharus"
SERVICE="pharus-agent"

SERVER=""
TOKEN=""
INTERVAL="3"

usage() {
  echo "用法: $0 --server <ws(s)://.../ws/agent> --token <token> [--interval 3]"
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --server)   SERVER="$2"; shift 2 ;;
    --token)    TOKEN="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[[ -z "$SERVER" || -z "$TOKEN" ]] && usage

if [[ $EUID -ne 0 ]]; then
  echo "请用 root 或 sudo 运行" >&2
  exit 1
fi

# 识别架构
case "$(uname -m)" in
  x86_64|amd64)   ARCH="x86_64" ;;
  aarch64|arm64)  ARCH="aarch64" ;;
  *) echo "不支持的架构: $(uname -m)" >&2; exit 1 ;;
esac
ASSET="pharus-agent-linux-${ARCH}.tar.gz"

echo ">> 下载 pharus-agent (linux/${ARCH})"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if command -v curl >/dev/null; then
  curl -fsSL "https://github.com/${REPO}/releases/latest/download/${ASSET}" -o "${TMP}/agent.tar.gz"
elif command -v wget >/dev/null; then
  wget -q "https://github.com/${REPO}/releases/latest/download/${ASSET}" -O "${TMP}/agent.tar.gz"
else
  echo "需要 curl 或 wget" >&2; exit 1
fi

tar -xzf "${TMP}/agent.tar.gz" -C "$TMP"
install -m 0755 "${TMP}/pharus-agent" "$INSTALL_BIN"

echo ">> 写入配置 ${CONFIG_DIR}/agent.toml"
mkdir -p "$CONFIG_DIR"
cat > "${CONFIG_DIR}/agent.toml" <<EOF
server = "${SERVER}"
token = "${TOKEN}"
interval = ${INTERVAL}
EOF
chmod 600 "${CONFIG_DIR}/agent.toml"

echo ">> 安装 systemd 服务"
cat > "/etc/systemd/system/${SERVICE}.service" <<EOF
[Unit]
Description=Pharus monitoring agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=RUST_LOG=info
ExecStart=${INSTALL_BIN} --config ${CONFIG_DIR}/agent.toml
Restart=always
RestartSec=3
NoNewPrivileges=true
ProtectSystem=full
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now "$SERVICE"

echo ""
echo "安装完成。查看状态:  systemctl status ${SERVICE}"
echo "查看日志:            journalctl -u ${SERVICE} -f"
