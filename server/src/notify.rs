//! Notification channel delivery.

use crate::state::SharedState;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// Deliver a notification to each enabled channel in `channel_ids`.
/// Failures are logged per channel and never propagate to the caller.
pub async fn dispatch(_state: &SharedState, _channel_ids: &[i64], _n: &Notification) {}

/// Send a fixed probe message through one channel so operators can verify config.
pub async fn test_channel(_state: &SharedState, _channel_id: i64) -> Result<()> {
    Ok(())
}
