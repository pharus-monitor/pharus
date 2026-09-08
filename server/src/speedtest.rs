//! Browser speedtest anti-abuse: per-IP request budget, a global concurrency
//! cap, and a monthly traffic quota. The speedtest endpoints are public (no
//! agent context, no auth), so enforcement lives entirely server-side.

use crate::state::SharedState;
use anyhow::Result;
use chrono::Datelike;
use rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Fallback per-IP hourly request budget (download + upload requests, counted
/// at admission). One single-thread run costs ~2 slots, one multi-thread ~8.
const PER_IP_HOUR_DEFAULT: usize = 30;
/// Data connections in flight globally. Bounds server load and the worst-case
/// quota overshoot (8 connections x 100MB = 800MB above the configured limit).
const MAX_CONCURRENT: usize = 8;
/// Upload bodies beyond this are rejected. Single mode sends 25MB, multi 8MB.
pub const UPLOAD_MAX_BYTES: u64 = 30 * 1024 * 1024;
/// Cap on the sliding-window map so IP-rotating abuse cannot grow it without
/// bound; the older half is evicted once this size is reached.
const WINDOW_MAP_MAX: usize = 10_000;

const MONTH_KEY: &str = "speedtest_traffic_month";
const BYTES_KEY: &str = "speedtest_traffic_bytes";
const LIMIT_KEY: &str = "speedtest_traffic_limit_gb";
const RATE_KEY: &str = "speedtest_per_ip_hour";

/// UTC calendar month, e.g. "2026-09": the quota counter's cycle key.
pub fn current_month() -> String {
    let now = chrono::Utc::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

fn setting_usize(state: &SharedState, key: &str, default: usize) -> usize {
    let conn = state.db.lock().unwrap();
    crate::db::get_setting(&conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Sliding per-IP hourly budget over download + upload requests. Counted at
/// request admission, mirroring `diag::check_ip_budget`. Ping probes are
/// excluded: their 0-byte 204 response costs no bandwidth.
pub fn check_ip_budget(state: &SharedState, ip: &str) -> bool {
    let limit = setting_usize(state, RATE_KEY, PER_IP_HOUR_DEFAULT);
    let mut map = state.speedtest_by_ip.lock().unwrap();
    let cutoff = Instant::now() - Duration::from_secs(3600);
    map.retain(|_, v| {
        v.retain(|t| *t > cutoff);
        !v.is_empty()
    });
    if map.len() >= WINDOW_MAP_MAX {
        // IP-rotating abuse could otherwise grow the map without bound. Evict
        // the stalest half; the global traffic quota is the real backstop.
        let mut by_recency: Vec<(String, Instant)> = map
            .iter()
            .map(|(k, v)| (k.clone(), *v.iter().max().unwrap()))
            .collect();
        by_recency.sort_by_key(|(_, t)| *t);
        for (k, _) in by_recency.into_iter().take(map.len() / 2) {
            map.remove(&k);
        }
    }
    let bucket = map.entry(ip.to_string()).or_default();
    if bucket.len() >= limit {
        return false;
    }
    bucket.push(Instant::now());
    true
}

/// RAII permit for one in-flight data connection; the slot is released on
/// drop however the connection ends (completion, client abort, error).
pub struct InflightPermit {
    counter: Arc<AtomicUsize>,
}

impl Drop for InflightPermit {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn acquire_inflight(state: &SharedState) -> Option<InflightPermit> {
    let counter = &state.speedtest_inflight;
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        if current >= MAX_CONCURRENT {
            return None;
        }
        match counter.compare_exchange_weak(current, current + 1, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => {
                return Some(InflightPermit {
                    counter: Arc::clone(counter),
                })
            }
            Err(actual) => current = actual,
        }
    }
}

/// Configured monthly quota in bytes (GB = 1e9 bytes); `None` = unlimited.
fn quota_limit(conn: &Connection) -> Result<Option<u64>> {
    let raw = crate::db::get_setting(conn, LIMIT_KEY)?;
    Ok(raw
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|g| *g > 0.0)
        .map(|g| (g * 1e9) as u64))
}

/// Bytes used in the current UTC month. A counter stamped with a previous
/// month reads as zero — the reset is applied on the next `add_bytes`.
pub fn month_usage(conn: &Connection) -> Result<u64> {
    let month = crate::db::get_setting(conn, MONTH_KEY)?;
    if month.as_deref() != Some(current_month().as_str()) {
        return Ok(0);
    }
    Ok(crate::db::get_setting(conn, BYTES_KEY)?
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0))
}

/// True when a monthly quota is configured and already reached.
pub fn quota_exceeded(conn: &Connection) -> Result<bool> {
    Ok(match quota_limit(conn)? {
        Some(limit) => month_usage(conn)? >= limit,
        None => false,
    })
}

/// Add actually-transferred bytes to the monthly counter. Month rollover and
/// increment happen in one transaction so the (month, bytes) pair cannot be
/// left mismatched by a crash. Errors are logged, not propagated: a failed
/// counter write must not break an in-flight response.
pub fn add_bytes(state: &SharedState, n: u64) {
    if n == 0 {
        return;
    }
    // Bind the month once: comparing against one call and writing another
    // could straddle midnight UTC and let the new month inherit the old
    // month's counter.
    let month = current_month();
    let result = (|| -> Result<()> {
        let conn = state.db.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let stored_month = crate::db::get_setting(&tx, MONTH_KEY)?;
        let current = if stored_month.as_deref() == Some(month.as_str()) {
            crate::db::get_setting(&tx, BYTES_KEY)?
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0)
        } else {
            0
        };
        crate::db::set_setting(&tx, MONTH_KEY, &month)?;
        crate::db::set_setting(&tx, BYTES_KEY, &(current + n).to_string())?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::error!(error = %e, bytes = n, "speedtest traffic counter write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init(&conn).unwrap();
        conn
    }

    #[test]
    fn unlimited_by_default() {
        let conn = conn();
        assert!(!quota_exceeded(&conn).unwrap());
        assert_eq!(month_usage(&conn).unwrap(), 0);
    }

    #[test]
    fn usage_from_another_month_reads_as_zero() {
        let conn = conn();
        crate::db::set_setting(&conn, MONTH_KEY, "2000-01").unwrap();
        crate::db::set_setting(&conn, BYTES_KEY, "999999999").unwrap();
        assert_eq!(month_usage(&conn).unwrap(), 0);
        crate::db::set_setting(&conn, LIMIT_KEY, "0.5").unwrap();
        assert!(!quota_exceeded(&conn).unwrap());
    }

    #[test]
    fn current_month_usage_counts_against_limit() {
        let conn = conn();
        crate::db::set_setting(&conn, MONTH_KEY, &current_month()).unwrap();
        crate::db::set_setting(&conn, BYTES_KEY, "1500000000").unwrap();
        crate::db::set_setting(&conn, LIMIT_KEY, "1").unwrap();
        assert!(quota_exceeded(&conn).unwrap());
        crate::db::set_setting(&conn, LIMIT_KEY, "2").unwrap();
        assert!(!quota_exceeded(&conn).unwrap());
    }

    #[test]
    fn invalid_or_zero_limit_means_unlimited() {
        let conn = conn();
        for bad in ["", "abc", "-5", "0"] {
            crate::db::set_setting(&conn, LIMIT_KEY, bad).unwrap();
            assert_eq!(quota_limit(&conn).unwrap(), None);
        }
    }
}
