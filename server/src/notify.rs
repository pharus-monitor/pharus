//! Notification channel delivery.

use crate::db::ChannelRow;
use crate::state::SharedState;
use anyhow::{anyhow, bail, Result};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// Deliver a notification to each enabled channel in `channel_ids`.
/// Failures are logged per channel and never propagate to the caller.
pub async fn dispatch(state: &SharedState, channel_ids: &[i64], n: &Notification) {
    let channels = {
        let conn = match state.db.lock() {
            Ok(conn) => conn,
            Err(_) => {
                for &channel_id in channel_ids {
                    tracing::warn!(channel_id, "notification channel could not be loaded");
                }
                return;
            }
        };

        let mut channels = Vec::with_capacity(channel_ids.len());
        for &channel_id in channel_ids {
            match crate::db::get_channel(&conn, channel_id) {
                Ok(Some(mut channel)) if channel.enabled => {
                    channel.config = crate::crypto::reveal(&conn, &channel.config);
                    channels.push(channel);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    tracing::warn!(channel_id, "notification channel could not be loaded");
                }
            }
        }
        channels
    };

    // Deliver each channel on its own task so a slow/blackholed endpoint
    // (retries + backoff can take ~45s) never stalls alert evaluation for the
    // other channels or the next tick.
    for channel in channels {
        let state = state.clone();
        let n = n.clone();
        tokio::spawn(async move {
            let outcome = tokio::time::timeout(Duration::from_secs(75), send(&state, &channel, &n))
                .await
                .unwrap_or(Err(anyhow!("channel send timed out")));
            if outcome.is_err() {
                tracing::warn!(
                    channel_id = channel.id,
                    channel_name = %channel.name,
                    "notification delivery failed"
                );
                set_channel_streak(&state, channel.id, channel.failed_streak.saturating_add(1));
            } else {
                set_channel_streak(&state, channel.id, 0);
            }
        });
    }
}

/// Persist a channel's consecutive-failure streak (0 = healthy).
fn set_channel_streak(state: &SharedState, id: i64, streak: i32) {
    if let Ok(conn) = state.db.lock() {
        let _ = crate::db::set_channel_streak(&conn, id, streak);
    }
}

/// Send a fixed probe message through one channel so operators can verify config.
pub async fn test_channel(state: &SharedState, channel_id: i64) -> Result<()> {
    let channel = {
        let conn = state.db.lock().map_err(|_| anyhow!("数据库暂时不可用"))?;
        let mut channel =
            crate::db::get_channel(&conn, channel_id)?.ok_or_else(|| anyhow!("通知通道不存在"))?;
        channel.config = crate::crypto::reveal(&conn, &channel.config);
        channel
    };

    let notification = Notification {
        title: "Pharus 通知测试".to_string(),
        body: "这是一条测试消息，收到后说明通知通道配置正确。".to_string(),
    };
    send(state, &channel, &notification).await?;
    set_channel_streak(state, channel_id, 0);
    Ok(())
}

async fn send(_state: &SharedState, channel: &ChannelRow, n: &Notification) -> Result<()> {
    match channel.kind.as_str() {
        "telegram" => send_telegram(&channel.config, n).await,
        "webhook" => send_webhook(&channel.config, n).await,
        "email" => send_email(&channel.config, n).await,
        "bark" => send_bark(&channel.config, n).await,
        "feishu" => send_feishu(&channel.config, n).await,
        "dingtalk" => send_dingtalk(&channel.config, n).await,
        "wecom" => send_wecom(&channel.config, n).await,
        "discord" => send_discord(&channel.config, n).await,
        _ => bail!("不支持的通知通道类型"),
    }
    .map_err(|_| anyhow!("通知发送失败"))
}

async fn send_telegram(config: &Value, n: &Notification) -> Result<()> {
    let bot_token = required_str(config, "bot_token")?;
    let chat_id = required_str(config, "chat_id")?;
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    post_json(
        &url,
        &json!({
            "chat_id": chat_id,
            "text": notification_text(n),
        }),
    )
    .await
}

async fn send_webhook(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    post_json(url, &json!({ "title": n.title, "body": n.body })).await
}

async fn send_email(config: &Value, n: &Notification) -> Result<()> {
    let smtp_host = required_str(config, "smtp_host")?;
    let smtp_port = required_port(config, "smtp_port")?;
    let username = required_str(config, "username")?;
    let password = required_str(config, "password")?;
    let from: lettre::message::Mailbox = required_str(config, "from")?
        .parse()
        .map_err(|_| anyhow!("发件地址格式无效"))?;
    let to: lettre::message::Mailbox = required_str(config, "to")?
        .parse()
        .map_err(|_| anyhow!("收件地址格式无效"))?;

    let builder = if smtp_port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
    }
    .map_err(|_| anyhow!("SMTP 配置无效"))?;
    let transport = builder
        .port(smtp_port)
        .credentials(Credentials::new(username.to_owned(), password.to_owned()))
        .build();

    let mut last_err = None;
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }
        let message = Message::builder()
            .from(from.clone())
            .to(to.clone())
            .subject(&n.title)
            .header(ContentType::TEXT_PLAIN)
            .body(n.body.clone())
            .map_err(|_| anyhow!("邮件内容构建失败"))?;
        match transport.send(message).await {
            Ok(_) => return Ok(()),
            Err(err) => last_err = Some(anyhow!("SMTP 发送失败: {err}")),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("SMTP 发送失败")))
}

async fn send_bark(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    let device_key = required_str(config, "device_key")?;
    post_json(
        url,
        &json!({
            "device_key": device_key,
            "title": n.title,
            "body": n.body,
        }),
    )
    .await
}

async fn send_feishu(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    post_json(
        url,
        &json!({
            "msg_type": "text",
            "content": { "text": notification_text(n) },
        }),
    )
    .await
}

async fn send_dingtalk(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    post_json(
        url,
        &json!({
            "msgtype": "text",
            "text": { "content": notification_text(n) },
        }),
    )
    .await
}

async fn send_wecom(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    post_json(
        url,
        &json!({
            "msgtype": "text",
            "text": { "content": notification_text(n) },
        }),
    )
    .await
}

async fn send_discord(config: &Value, n: &Notification) -> Result<()> {
    let url = required_str(config, "url")?;
    post_json(
        url,
        &json!({ "content": format!("**{}**\n{}", n.title, n.body) }),
    )
    .await
}

async fn post_json(url: &str, payload: &Value) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| anyhow!("HTTP 客户端初始化失败"))?;
    let mut last_err = None;
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }
        match client.post(url).json(payload).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) => last_err = Some(anyhow!("HTTP 服务返回失败状态")),
            Err(_) => last_err = Some(anyhow!("HTTP 请求失败")),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("HTTP 请求失败")))
}

fn notification_text(n: &Notification) -> String {
    format!("{}\n{}", n.title, n.body)
}

fn required_str<'a>(config: &'a Value, key: &str) -> Result<&'a str> {
    config
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("通知配置缺少必填字段"))
}

fn required_port(config: &Value, key: &str) -> Result<u16> {
    let port = config
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| anyhow!("SMTP 端口无效"))?;
    Ok(port)
}
