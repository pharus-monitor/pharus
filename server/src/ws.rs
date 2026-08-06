use crate::state::SharedState;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use pharus_common::{AgentMsg, AgentSnapshot, BrowserMsg, ServerToAgentMsg, PROTOCOL_VERSION};
use std::time::Duration;
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

    let token = match serde_json::from_str::<AgentMsg>(&auth_msg) {
        Ok(AgentMsg::Auth { token, version }) if version == PROTOCOL_VERSION => token,
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
        crate::db::find_by_token(&db, &token)
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
    {
        let mut agents = state.agents.write().unwrap();
        let entry = agents.entry(agent_id).or_default();
        entry.online = true;
        entry.conn_epoch = epoch;
    }
    state.broadcast(BrowserMsg::Status {
        agent_id,
        online: true,
    });
    info!(agent_id, name, "agent connected");

    loop {
        let next = tokio::time::timeout(AGENT_OFFLINE_TIMEOUT, read.next()).await;
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
                let mut agents = state.agents.write().unwrap();
                if let Some(a) = agents.get_mut(&agent_id) {
                    a.info = Some(info);
                }
            }
            Ok(AgentMsg::Metrics { data }) => {
                {
                    let mut agents = state.agents.write().unwrap();
                    if let Some(a) = agents.get_mut(&agent_id) {
                        a.data = Some(data.clone());
                    }
                }
                state.broadcast(BrowserMsg::Metrics {
                    agent_id,
                    online: true,
                    data,
                });
            }
            Ok(AgentMsg::Auth { .. }) => {
                warn!(agent_id, "unexpected re-auth, dropping");
                break;
            }
            Err(e) => warn!(agent_id, error = %e, "bad agent message"),
        }
    }

    {
        let mut agents = state.agents.write().unwrap();
        if let Some(a) = agents.get_mut(&agent_id) {
            if a.conn_epoch == epoch {
                a.online = false;
            } else {
                return;
            }
        }
    }
    state.broadcast(BrowserMsg::Status {
        agent_id,
        online: false,
    });
    info!(agent_id, "agent disconnected");
}

pub async fn handle_browser_socket(state: SharedState, socket: WebSocket) {
    let (mut write, mut read) = socket.split();

    let snapshot = {
        let agents = state.agents.read().unwrap();
        let list: Vec<AgentSnapshot> = agents
            .iter()
            .map(|(id, a)| AgentSnapshot {
                agent_id: *id,
                name: a.name.clone(),
                online: a.online,
                info: a.info.clone(),
                data: a.data.clone(),
            })
            .collect();
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
