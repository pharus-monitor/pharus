//! Browser terminal over SSH: the server connects straight to the target
//! host's sshd (no agent involvement), attaches a pty + shell, and relays
//! I/O with the browser WebSocket. Credentials are stored per host,
//! encrypted at rest with the site's AES-GCM master key.

use anyhow::{anyhow, Result};
use russh::client::{self};
use russh::keys::PublicKeyOrCertificate;
use russh::ChannelMsg;
use std::sync::Arc;
use tokio::sync::mpsc;

struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Accept-any: hosts are added by the operator, the panel is an
        // internal tool, and a strict pin would break on host key rotation.
        Ok(true)
    }
}

/// Stored credentials for a host, decrypted for use.
#[derive(Debug, Clone, Default)]
pub struct SshCreds {
    /// Explicit host override; None = use the agent-reported address.
    pub host: Option<String>,
    pub port: u16,
    pub user: String,
    pub password: String,
}

/// Load + decrypt a host's SSH credentials. Ok(None) = never configured.
pub fn load_creds(state: &crate::state::SharedState, agent_id: i64) -> Result<Option<SshCreds>> {
    let stored = {
        let conn = state.db.lock().unwrap();
        crate::db::get_setting(&conn, &format!("ssh:{agent_id}"))?
    };
    let Some(stored) = stored.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let plain = {
        let conn = state.db.lock().unwrap();
        crate::crypto::decrypt(&conn, &stored).map_err(|e| anyhow!("decrypt ssh creds: {e}"))?
    };
    let v: serde_json::Value = serde_json::from_str(&plain)?;
    let password = {
        let conn = state.db.lock().unwrap();
        v.get("password")
            .and_then(|x| x.as_str())
            .map(|p| crate::crypto::decrypt(&conn, p).unwrap_or_default())
            .unwrap_or_default()
    };
    Ok(Some(SshCreds {
        host: v.get("host").and_then(|x| x.as_str()).map(String::from),
        port: v.get("port").and_then(|x| x.as_u64()).unwrap_or(22) as u16,
        user: v
            .get("user")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        password,
    }))
}

/// Encrypt + persist credentials for a host.
pub fn save_creds(state: &crate::state::SharedState, agent_id: i64, creds: &SshCreds) -> Result<()> {
    let enc_password = {
        let conn = state.db.lock().unwrap();
        crate::crypto::encrypt(&conn, &creds.password)?
    };
    let json = serde_json::json!({
        "host": creds.host,
        "port": creds.port,
        "user": creds.user,
        "password": enc_password,
    });
    let sealed = {
        let conn = state.db.lock().unwrap();
        crate::crypto::encrypt(&conn, &json.to_string())?
    };
    {
        let conn = state.db.lock().unwrap();
        crate::db::set_setting(&conn, &format!("ssh:{agent_id}"), &sealed)?;
    }
    if let Some(a) = state.agents.write().unwrap().get_mut(&agent_id) {
        a.ssh_host = Some(format!(
            "{}:{}",
            creds.host.clone().unwrap_or_default(),
            creds.port
        ));
    }
    Ok(())
}

pub fn clear_creds(state: &crate::state::SharedState, agent_id: i64) -> Result<()> {
    {
        let conn = state.db.lock().unwrap();
        crate::db::set_setting(&conn, &format!("ssh:{agent_id}"), "")?;
    }
    if let Some(a) = state.agents.write().unwrap().get_mut(&agent_id) {
        a.ssh_host = None;
    }
    Ok(())
}

/// Resolve the sshd address for a host: explicit override first, then the
/// agent-reported addresses (public before private in collection order).
pub fn ssh_target(
    state: &crate::state::SharedState,
    agent_id: i64,
    creds: &SshCreds,
) -> Option<String> {
    if let Some(host) = creds.host.as_deref().filter(|s| !s.is_empty()) {
        return Some(host.to_string());
    }
    if let Some(cached) = state
        .agents
        .read()
        .unwrap()
        .get(&agent_id)
        .and_then(|a| a.ssh_host.clone())
    {
        // Cached "host:port" from save_creds; strip the port, the caller
        // reads the port from creds.
        return cached.rsplit_once(':').map(|(h, _)| h.to_string());
    }
    let agents = state.agents.read().unwrap();
    agents
        .get(&agent_id)
        .and_then(|a| a.info.as_ref())
        .and_then(|i| i.ips.first())
        .cloned()
}

/// A live SSH terminal: write half for input/resize, plus the session handle
/// kept alive for the connection's lifetime.
pub struct SshTerm {
    pub write: russh::ChannelWriteHalf<russh::client::Msg>,
    pub output: mpsc::UnboundedReceiver<String>,
    _handle: client::Handle<ClientHandler>,
}

/// Connect, authenticate, request pty + shell. Bounded by a 15s timeout so a
/// blackholed address can't pin the terminal WebSocket forever.
pub async fn connect(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    cols: u16,
    rows: u16,
) -> Result<SshTerm> {
    tokio::time::timeout(std::time::Duration::from_secs(15), connect_inner(host, port, user, password, cols, rows))
        .await
        .map_err(|_| anyhow!("连接 {host}:{port} 超时"))?
}

async fn connect_inner(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    cols: u16,
    rows: u16,
) -> Result<SshTerm> {
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, (host, port), ClientHandler)
        .await
        .map_err(|e| anyhow!("连接 {host}:{port} 失败: {e}"))?;

    let auth = handle
        .authenticate_password(user, password)
        .await
        .map_err(|e| anyhow!("SSH 认证错误: {e}"))?;
    if !auth.success() {
        return Err(anyhow!("SSH 认证失败（用户名或密码错误）"));
    }

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| anyhow!("打开会话失败: {e}"))?;
    channel
        .request_pty(true, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await
        .map_err(|e| anyhow!("请求终端失败: {e}"))?;
    channel
        .request_shell(true)
        .await
        .map_err(|e| anyhow!("请求 shell 失败: {e}"))?;

    let (read, write) = channel.split();
    let (tx, output) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut read = read;
        loop {
            match read.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    if tx.send(String::from_utf8_lossy(&data).to_string()).is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if tx.send(String::from_utf8_lossy(&data).to_string()).is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    let _ = tx.send("\r\n[连接已关闭]\r\n".to_string());
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(SshTerm { write, output, _handle: handle })
}

impl SshTerm {
    pub async fn input(&mut self, data: &str) -> Result<()> {
        self.write
            .data_bytes(data.as_bytes().to_vec())
            .await
            .map_err(|e| anyhow!("send input: {e}"))
    }

    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.write
            .window_change(cols as u32, rows as u32, 0, 0)
            .await
            .map_err(|e| anyhow!("resize: {e}"))
    }
}
