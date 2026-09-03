use crate::state::SharedState;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use pharus_common::{AgentMsg, AgentSnapshot, BrowserMsg, ServerToAgentMsg, PROTOCOL_VERSION};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

const AGENT_OFFLINE_TIMEOUT: Duration = Duration::from_secs(15);
const AGENT_AUTH_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn handle_agent_socket(state: SharedState, socket: WebSocket) {
    let (mut write, mut read) = socket.split();

    let auth_msg = match tokio::time::timeout(AGENT_AUTH_TIMEOUT, read.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => t,
        _ => {
            let _ = write
                .send(Message::Text(
                    serde_json::to_string(&ServerToAgentMsg::AuthFail {
                        reason: "auth timeout or bad frame".into(),
                    })
                    .unwrap(),
                ))
                .await;
            return;
        }
    };

    let (token, auth_name, app_version, platform) = match serde_json::from_str::<AgentMsg>(&auth_msg) {
        Ok(AgentMsg::Auth {
            token,
            version,
            name,
            app_version,
            platform,
        }) if version == PROTOCOL_VERSION => (token, name, app_version, platform),
        Ok(AgentMsg::Auth { .. }) => {
            let _ = write
                .send(Message::Text(
                    serde_json::to_string(&ServerToAgentMsg::AuthFail {
                        reason: "protocol version mismatch".into(),
                    })
                    .unwrap(),
                ))
                .await;
            return;
        }
        _ => {
            let _ = write
                .send(Message::Text(
                    serde_json::to_string(&ServerToAgentMsg::AuthFail {
                        reason: "first message must be auth".into(),
                    })
                    .unwrap(),
                ))
                .await;
            return;
        }
    };

    let found = {
        let db = state.db.lock().unwrap();
        match crate::db::find_by_token(&db, &token) {
            Ok(Some(pair)) => Ok(Some(pair)),
            Ok(None) => {
                // Fall back to one of the shared agent secrets configured in
                // site settings; the agent is then matched (or auto-registered)
                // by its reported hostname.
                let secret_ok = {
                    let single = crate::db::get_setting(&db, "agent_secret").ok().flatten();
                    let list = crate::admin::load_agent_secrets(&db);
                    single.as_deref() == Some(token.as_str())
                        || list.iter().any(|e| e.secret == token)
                };
                if !secret_ok {
                    Ok(None)
                } else {
                    let host = auth_name.as_deref().unwrap_or("").trim();
                    if host.is_empty() {
                        Ok(None)
                    } else {
                        match crate::db::find_agent_by_hostname(&db, host) {
                            Ok(Some(pair)) => Ok(Some(pair)),
                            Ok(None) => crate::db::add_agent_named(&db, host, Some(host))
                                .map(|(id, _t)| Some((id, host.to_string()))),
                            Err(e) => Err(e),
                        }
                    }
                }
            }
            Err(e) => Err(e),
        }
    };
    let (agent_id, name) = match found {
        Ok(Some(pair)) => pair,
        _ => {
            let _ = write
                .send(Message::Text(
                    serde_json::to_string(&ServerToAgentMsg::AuthFail {
                        reason: "invalid token".into(),
                    })
                    .unwrap(),
                ))
                .await;
            return;
        }
    };

    if write
        .send(Message::Text(
            serde_json::to_string(&ServerToAgentMsg::AuthOk {
                agent_id,
                name: name.clone(),
            })
            .unwrap(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let epoch = state
        .next_epoch
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (agent_tx, mut agent_rx) =
        tokio::sync::mpsc::unbounded_channel::<pharus_common::ServerToAgentMsg>();
    // Load persisted per-agent state before touching the agents map; the lock
    // order is db → agents everywhere.
    let (region, features, unlock) = {
        let db = state.db.lock().unwrap();
        let region = crate::db::list_regions(&db)
            .ok()
            .and_then(|m| m.get(&agent_id).map(|(c, s)| crate::db::make_region(c, s)));
        let features = crate::features::effective_for(&db, agent_id).unwrap_or_default();
        let unlock = crate::db::list_streaming(&db, agent_id).unwrap_or_default();
        (region, features, unlock)
    };

    {
        let mut agents = state.agents.write().unwrap();
        let entry = agents.entry(agent_id).or_default();
        entry.name = name.clone();
        entry.online = true;
        entry.conn_epoch = epoch;
        entry.agent_tx = Some(agent_tx);
        entry.region = region.clone();
        entry.features = features.clone();
        entry.app_version = app_version;
        entry.platform = platform;
        if entry.unlock.is_empty() {
            entry.unlock = unlock;
        }
    }
    state.broadcast(BrowserMsg::Status {
        agent_id,
        online: true,
    });
    state.broadcast(BrowserMsg::RegionUpdate { agent_id, region });
    state.broadcast(BrowserMsg::FeaturesUpdate { agent_id, features });
    state.push_tasks(agent_id);
    info!(agent_id, name, "agent connected");

    loop {
        tokio::select! {
            next = tokio::time::timeout(AGENT_OFFLINE_TIMEOUT, read.next()) => {
                let msg = match next {
                    Ok(Some(Ok(Message::Text(t)))) => t,
                    Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
                    Ok(Some(Ok(_))) => continue,
                    Ok(Some(Err(e))) => {
                        warn!(agent_id, error = %e, "agent ws error");
                        break;
                    }
                    Ok(None) => break,
                    Err(_) => {
                        warn!(agent_id, "agent report timeout, marking offline");
                        break;
                    }
                };
                match serde_json::from_str::<AgentMsg>(&msg) {
                    Ok(AgentMsg::SysInfo { info }) => {
                        {
                            let mut agents = state.agents.write().unwrap();
                            if let Some(a) = agents.get_mut(&agent_id) {
                                a.info = Some(info);
                            }
                        }
                        // Push the updated snapshot so the UI picks up host
                        // info and reported IPs even when they arrive after
                        // the initial connection snapshot.
                        let agents_list = {
                            let ids: Vec<i64> =
                                state.agents.read().unwrap().keys().copied().collect();
                            ids.iter().map(|id| state.snapshot_with_gates(*id)).collect::<Vec<_>>()
                        };
                        state.broadcast(BrowserMsg::Snapshot { agents: agents_list });
                    }
                    Ok(AgentMsg::Metrics { data }) => {
                        {
                            let mut agents = state.agents.write().unwrap();
                            if let Some(a) = agents.get_mut(&agent_id) {
                                let default_billing = pharus_common::BillingInfo::default();
                                let b = a.billing.as_ref().unwrap_or(&default_billing);
                                crate::billing::apply_metrics(&mut a.traffic, b, &data, chrono::Local::now());
                                a.data = Some(data.clone());
                            }
                        }
                        state.broadcast(BrowserMsg::Metrics {
                            agent_id,
                            online: true,
                            data,
                        });
                    }
                    Ok(AgentMsg::Ping { results }) => {
                        {
                            // Only server-managed tasks carry a task_id; ad-hoc
                            // legacy tcping results are live-only.
                            let db = state.db.lock().unwrap();
                            for r in results.iter().filter(|r| r.task_id.is_some()) {
                                if let Err(e) = crate::db::insert_ping_history(&db, agent_id, r) {
                                    warn!(agent_id, error = %e, "ping history insert failed");
                                }
                            }
                        }
                        {
                            let mut agents = state.agents.write().unwrap();
                            if let Some(a) = agents.get_mut(&agent_id) {
                                a.pings = results.clone();
                            }
                        }
                        state.broadcast(BrowserMsg::Pings { agent_id, results });
                    }
                    Ok(AgentMsg::Unlock { results }) => {
                        {
                            let db = state.db.lock().unwrap();
                            if let Err(e) = crate::db::replace_streaming(&db, agent_id, &results) {
                                warn!(agent_id, error = %e, "streaming result save failed");
                            }
                            if let Err(e) = crate::db::append_streaming_history(&db, agent_id, &results) {
                                warn!(agent_id, error = %e, "streaming history save failed");
                            }
                        }
                        {
                            let mut agents = state.agents.write().unwrap();
                            if let Some(a) = agents.get_mut(&agent_id) {
                                a.unlock = results.clone();
                            }
                        }
                        state.broadcast(BrowserMsg::Unlock { agent_id, results });
                    }
                    Ok(AgentMsg::Containers { containers, available }) => {
                        {
                            let mut agents = state.agents.write().unwrap();
                            if let Some(a) = agents.get_mut(&agent_id) {
                                a.containers = Some(containers.clone());
                                if available {
                                    a.docker_available = true;
                                }
                            }
                        }
                        state.broadcast(BrowserMsg::Containers {
                            agent_id,
                            containers,
                            available,
                        });
                    }
                    Ok(AgentMsg::TaskResult { task_id, exit_code, output, scheduled_id }) => {
                        match scheduled_id {
                            // periodic run of a stored task: persist it
                            Some(id) => {
                                let db = state.db.lock().unwrap();
                                if let Err(e) =
                                    crate::db::insert_task_result(&db, id, agent_id, exit_code, &output)
                                {
                                    warn!(agent_id, task_id = id, error = %e, "task result insert failed");
                                }
                            }
                            // one-shot run: hand off to the admin handler waiting on it
                            None => {
                                let waiter =
                                    state.task_waiters.lock().unwrap().remove(&task_id);
                                match waiter {
                                    Some(tx) => {
                                        let _ = tx.send(AgentMsg::TaskResult {
                                            task_id,
                                            exit_code,
                                            output,
                                            scheduled_id: None,
                                        });
                                    }
                                    None => {
                                        crate::diag::relay_legacy_result(
                                            &state, agent_id, task_id, exit_code, output,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Ok(AgentMsg::Region { code }) => {
                        // a manual override always wins over agent-side detection
                        let manual = {
                            let db = state.db.lock().unwrap();
                            crate::db::region_source(&db, agent_id)
                                .ok()
                                .flatten()
                                .is_some_and(|s| s == "manual")
                        };
                        if !manual {
                            if let Some(code) = crate::regions::normalize(&code) {
                                let region = crate::db::make_region(&code, "auto");
                                {
                                    let db = state.db.lock().unwrap();
                                    if let Err(e) =
                                        crate::db::set_region(&db, agent_id, Some(&code), "auto")
                                    {
                                        warn!(agent_id, error = %e, "region update failed");
                                    }
                                }
                                {
                                    let mut agents = state.agents.write().unwrap();
                                    if let Some(a) = agents.get_mut(&agent_id) {
                                        a.region = Some(region.clone());
                                    }
                                }
                                state.broadcast(BrowserMsg::RegionUpdate {
                                    agent_id,
                                    region: Some(region),
                                });
                            }
                        }
                    }
                    Ok(AgentMsg::CmdOutput { request_id, stream, data, done, exit_code }) => {
                        info!(
                            agent_id,
                            request_id = %request_id,
                            stream = %stream,
                            done,
                            exit_code = ?exit_code,
                            preview = %data.chars().take(80).collect::<String>(),
                            "agent cmd output"
                        );
                        crate::diag::relay_output(
                            &state, agent_id, request_id, stream, data, done, exit_code,
                        );
                    }
                    Ok(AgentMsg::MtrResult { request_id, hubs, done }) => {
                        info!(
                            agent_id,
                            request_id = %request_id,
                            hubs = hubs.len(),
                            done,
                            "agent mtr result"
                        );
                        crate::diag::relay_mtr(&state, agent_id, request_id, hubs, done);
                    }
                    Ok(AgentMsg::Iperf3Result {
                        request_id,
                        direction,
                        throughput_bps,
                        retransmits,
                        duration_s,
                    }) => {
                        info!(
                            agent_id,
                            request_id = %request_id,
                            direction = %direction,
                            throughput_bps = ?throughput_bps,
                            retransmits = ?retransmits,
                            duration_s = ?duration_s,
                            "agent iperf3 result"
                        );
                        crate::diag::relay_iperf3(
                            &state,
                            agent_id,
                            request_id,
                            direction,
                            throughput_bps,
                            retransmits,
                            duration_s,
                        );
                    }
                    Ok(AgentMsg::Auth { .. }) => {
                        warn!(agent_id, "unexpected re-auth, dropping");
                        break;
                    }
                    Ok(AgentMsg::UpdateStatus {
                        request_id,
                        phase,
                        done,
                        error,
                    }) => {
                        info!(
                            agent_id,
                            request_id,
                            phase,
                            done,
                            error = error.as_deref().unwrap_or(""),
                            "agent update status"
                        );
                        state.broadcast(pharus_common::BrowserMsg::UpdateStatus {
                            agent_id: Some(agent_id),
                            kind: "agent".into(),
                            phase,
                            done,
                            error,
                        });
                    }
                    Ok(AgentMsg::TermOutput { session_id, data }) => {
                        if let Some(tx) = state.term_sessions.lock().unwrap().get(&session_id) {
                            let _ = tx.send(data);
                        }
                    }
                    Ok(AgentMsg::TermExit { session_id, exit_code }) => {
                        let mut sessions = state.term_sessions.lock().unwrap();
                        if let Some(tx) = sessions.get(&session_id) {
                            let _ = tx.send(
                                format!("\r\n[进程已退出{}{}]\r\n", if exit_code.is_some() { "，代码 " } else { "" }, exit_code.map(|c| c.to_string()).unwrap_or_default())
                            );
                            sessions.remove(&session_id);
                        }
                    }
                    Err(e) => warn!(agent_id, error = %e, "bad agent message"),
                }
            }
            down = agent_rx.recv() => {
                match down {
                    Some(m) => {
                        let Ok(s) = serde_json::to_string(&m) else { continue };
                        if write.send(Message::Text(s)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    {
        let mut agents = state.agents.write().unwrap();
        if let Some(a) = agents.get_mut(&agent_id) {
            if a.conn_epoch == epoch {
                a.online = false;
                a.agent_tx = None;
            } else {
                return;
            }
        }
    }
    crate::diag::cancel_for_agent(&state, agent_id);
    state.broadcast(BrowserMsg::Status {
        agent_id,
        online: false,
    });
    info!(agent_id, "agent disconnected");
}

pub async fn handle_browser_socket(state: SharedState, socket: WebSocket) {
    let (mut write, mut read) = socket.split();

    let snapshot = {
        let ids: Vec<i64> = state.agents.read().unwrap().keys().copied().collect();
        let list: Vec<AgentSnapshot> = ids.iter().map(|id| state.snapshot_with_gates(*id)).collect();
        BrowserMsg::Snapshot { agents: list }
    };
    if write
        .send(Message::Text(serde_json::to_string(&snapshot).unwrap()))
        .await
        .is_err()
    {
        return;
    }

    let mut rx = state.browser_tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(m) => {
                        if write
                            .send(Message::Text(serde_json::to_string(&m).unwrap()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

/// Browser terminal: one WebSocket per session. The first frame must be
/// `{"type":"open","agent_id":N}`, then `input`/`resize` frames stream down.
/// The server SSHes straight into the target host; credentials come from the
/// per-host store or from `auth` frames typed in the terminal dialog.
pub async fn handle_term_socket(state: SharedState, socket: WebSocket) {
    let (mut write, mut read) = socket.split();

    let open = match read.next().await {
        Some(Ok(Message::Text(t))) => serde_json::from_str::<serde_json::Value>(&t).ok(),
        _ => None,
    };
    let Some(open) = open else {
        let _ = write.send(Message::Close(None)).await;
        return;
    };
    if open.get("type").and_then(|v| v.as_str()) != Some("open") {
        let _ = write.send(Message::Close(None)).await;
        return;
    }
    let agent_id = open.get("agent_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if agent_id <= 0 {
        let _ = write.send(Message::Close(None)).await;
        return;
    }
    let cols = open.get("cols").and_then(|v| v.as_u64()).map(|v| v as u16).unwrap_or(80);
    let rows = open.get("rows").and_then(|v| v.as_u64()).map(|v| v as u16).unwrap_or(24);
    let frame_user = open.get("user").and_then(|v| v.as_str()).map(String::from);
    let frame_password = open.get("password").and_then(|v| v.as_str()).map(String::from);
    let save_creds = open.get("save").and_then(|v| v.as_bool()).unwrap_or(false);

    let stored = crate::ssh_term::load_creds(&state, agent_id).ok().flatten();

    // Credentials: the open frame wins (operator just typed them), stored
    // creds serve repeat visits.
    let (user, password) = match (frame_user.filter(|u| !u.is_empty()), frame_password.filter(|p| !p.is_empty())) {
        (Some(u), Some(p)) => (u, p),
        _ => match &stored {
            Some(c) if !c.user.is_empty() && !c.password.is_empty() => (c.user.clone(), c.password.clone()),
            _ => {
                let _ = write.send(Message::Text("\r\n[未配置 SSH 凭据：请通过主机页的 SSH 按钮登录]\r\n".into())).await;
                let _ = write.send(Message::Close(None)).await;
                return;
            }
        },
    };
    let port = stored.as_ref().map(|c| c.port).unwrap_or(22);
    let Some(host) = crate::ssh_term::ssh_target(&state, agent_id, stored.as_ref().unwrap_or(&Default::default())) else {
        let _ = write
            .send(Message::Text("\r\n[无法确定 SSH 地址：该主机没有上报 IP，请先配置主机地址]\r\n".into()))
            .await;
        let _ = write.send(Message::Close(None)).await;
        return;
    };

    if save_creds {
        let to_save = crate::ssh_term::SshCreds {
            host: stored.as_ref().and_then(|c| c.host.clone()),
            port,
            user: user.clone(),
            password: password.clone(),
        };
        if let Err(e) = crate::ssh_term::save_creds(&state, agent_id, &to_save) {
            tracing::warn!(agent_id, error = %e, "save ssh creds failed");
        }
    }

    let mut term = match crate::ssh_term::connect(&host, port, &user, &password, cols, rows).await {
        Ok(t) => t,
        Err(e) => {
            let _ = write.send(Message::Text(format!("\r\n[SSH 连接失败: {e}]\r\n"))).await;
            let _ = write.send(Message::Close(None)).await;
            return;
        }
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let mut output = std::mem::replace(&mut term.output, mpsc::unbounded_channel().1);
    let pump = tokio::spawn(async move {
        while let Some(chunk) = output.recv().await {
            if out_tx.send(chunk).is_err() {
                break;
            }
        }
    });

    let fwd = tokio::spawn(async move {
        while let Some(data) = out_rx.recv().await {
            if write.send(Message::Text(data)).await.is_err() {
                break;
            }
        }
        let _ = write.send(Message::Close(None)).await;
    });

    loop {
        match read.next().await {
            Some(Ok(Message::Text(t))) => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                match v.get("type").and_then(|x| x.as_str()) {
                    Some("input") => {
                        if let Some(d) = v.get("data").and_then(|x| x.as_str()) {
                            let _ = term.input(d).await;
                        }
                    }
                    Some("resize") => {
                        let c = v.get("cols").and_then(|x| x.as_u64()).map(|x| x as u16).unwrap_or(80);
                        let r = v.get("rows").and_then(|x| x.as_u64()).map(|x| x as u16).unwrap_or(24);
                        let _ = term.resize(c, r).await;
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            _ => break,
        }
    }

    pump.abort();
    fwd.abort();
}
