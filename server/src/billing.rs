use crate::state::TrafficState;
use chrono::{DateTime, Datelike, TimeZone};
use pharus_common::{BillingInfo, Metrics};

/// Current billing-cycle traffic usage as a percentage of the quota
/// (0..=100+), honoring the per-host uni/bidirectional pick. `None` when no
/// quota is configured.
pub fn usage_percent(billing: &BillingInfo, traffic: &TrafficState) -> Option<f64> {
    let quota = billing.quota_bytes?;
    if quota == 0 {
        return None;
    }
    let used = match billing.traffic_mode.as_deref() {
        Some("uni") => match billing.traffic_dir.as_deref() {
            Some("up") => traffic.tx_bytes,
            Some("max") => traffic.rx_bytes.max(traffic.tx_bytes),
            _ => traffic.rx_bytes,
        },
        _ => traffic.rx_bytes.saturating_add(traffic.tx_bytes),
    };
    Some(used as f64 / quota as f64 * 100.0)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// Start of the current billing cycle: the most recent local midnight on the
/// reset day (clamped to the month's length, so 31 becomes 28/29 in February).
pub fn cycle_start_for<Tz: TimeZone>(reset_day: u8, now: DateTime<Tz>) -> i64 {
    let rd = (reset_day as u32).clamp(1, 31);
    let day = rd.min(days_in_month(now.year(), now.month()));
    if let Some(d) = now
        .timezone()
        .with_ymd_and_hms(now.year(), now.month(), day, 0, 0, 0)
        .single()
    {
        if d <= now {
            return d.timestamp();
        }
    }
    let (py, pm) = if now.month() == 1 {
        (now.year() - 1, 12)
    } else {
        (now.year(), now.month() - 1)
    };
    let pday = rd.min(days_in_month(py, pm));
    now.timezone()
        .with_ymd_and_hms(py, pm, pday, 0, 0, 0)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

/// Accumulate agent counters into period traffic. Old agents that don't
/// report cumulative counters (both zero) are skipped.
pub fn apply_metrics<Tz: TimeZone>(
    t: &mut TrafficState,
    billing: &BillingInfo,
    m: &Metrics,
    now: DateTime<Tz>,
) {
    if let Some(rd) = billing.reset_day {
        let start = cycle_start_for(rd, now.clone());
        if t.cycle_start < start {
            t.cycle_start = start;
            t.rx_bytes = 0;
            t.tx_bytes = 0;
            t.last_rx_total = None;
            t.last_tx_total = None;
        }
    }
    if m.net_rx_total == 0 && m.net_tx_total == 0 {
        return;
    }
    match t.last_rx_total {
        Some(prev) if m.net_rx_total >= prev => t.rx_bytes += m.net_rx_total - prev,
        // counter went backwards: agent rebooted, count from its new zero
        Some(_) => t.rx_bytes += m.net_rx_total,
        None => {}
    }
    match t.last_tx_total {
        Some(prev) if m.net_tx_total >= prev => t.tx_bytes += m.net_tx_total - prev,
        Some(_) => t.tx_bytes += m.net_tx_total,
        None => {}
    }
    t.last_rx_total = Some(m.net_rx_total);
    t.last_tx_total = Some(m.net_tx_total);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn at(y: i32, mo: u32, d: u32, h: u32) -> DateTime<FixedOffset> {
        tz().with_ymd_and_hms(y, mo, d, h, 0, 0).single().unwrap()
    }

    fn metrics(rx_total: u64, tx_total: u64) -> Metrics {
        Metrics {
            cpu_usage: 0.0,
            mem_used: 0,
            mem_total: 0,
            swap_used: 0,
            swap_total: 0,
            disk_used: 0,
            disk_total: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            load1: 0.0,
            uptime: 0,
            net_rx_total: rx_total,
            net_tx_total: tx_total,
            disk_write_bps: 0,
            disk_read_bps: 0,
        }
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn cycle_start_same_month() {
        // Aug 6, reset on the 1st -> Aug 1 00:00 local
        assert_eq!(cycle_start_for(1, at(2026, 8, 6, 12)), at(2026, 8, 1, 0).timestamp());
    }

    #[test]
    fn cycle_start_previous_month() {
        // Aug 6, reset on the 10th -> Jul 10 00:00 local
        assert_eq!(cycle_start_for(10, at(2026, 8, 6, 12)), at(2026, 7, 10, 0).timestamp());
    }

    #[test]
    fn cycle_start_exact_boundary() {
        assert_eq!(cycle_start_for(1, at(2026, 8, 1, 0)), at(2026, 8, 1, 0).timestamp());
    }

    #[test]
    fn cycle_start_clamps_short_month() {
        // reset on 31st, in March -> cycle started Feb 28 (2026 not a leap year)
        assert_eq!(cycle_start_for(31, at(2026, 3, 15, 12)), at(2026, 2, 28, 0).timestamp());
        // and on Mar 31 it rolls to Mar 31 itself
        assert_eq!(cycle_start_for(31, at(2026, 3, 31, 12)), at(2026, 3, 31, 0).timestamp());
    }

    #[test]
    fn cycle_start_year_rollover() {
        assert_eq!(cycle_start_for(10, at(2026, 1, 5, 12)), at(2025, 12, 10, 0).timestamp());
    }

    #[test]
    fn skips_old_agents() {
        let mut t = TrafficState::default();
        apply_metrics(&mut t, &BillingInfo::default(), &metrics(0, 0), at(2026, 8, 6, 12));
        assert_eq!(t.rx_bytes, 0);
        assert_eq!(t.last_rx_total, None);
    }

    #[test]
    fn baseline_then_delta() {
        let mut t = TrafficState::default();
        let b = BillingInfo::default();
        apply_metrics(&mut t, &b, &metrics(1000, 500), at(2026, 8, 6, 12));
        assert_eq!(t.rx_bytes, 0); // first sample only sets the baseline
        apply_metrics(&mut t, &b, &metrics(1300, 900), at(2026, 8, 6, 12));
        assert_eq!(t.rx_bytes, 300);
        assert_eq!(t.tx_bytes, 400);
    }

    #[test]
    fn agent_reboot_counts_from_zero() {
        let mut t = TrafficState::default();
        let b = BillingInfo::default();
        apply_metrics(&mut t, &b, &metrics(5000, 0), at(2026, 8, 6, 12));
        apply_metrics(&mut t, &b, &metrics(700, 0), at(2026, 8, 6, 12));
        assert_eq!(t.rx_bytes, 700); // counter dropped -> agent rebooted
    }

    #[test]
    fn cycle_rollover_resets() {
        let mut t = TrafficState::default();
        let b = BillingInfo { reset_day: Some(1), ..Default::default() };
        apply_metrics(&mut t, &b, &metrics(1000, 0), at(2026, 8, 31, 12));
        apply_metrics(&mut t, &b, &metrics(1600, 0), at(2026, 8, 31, 12));
        assert_eq!(t.rx_bytes, 600);
        apply_metrics(&mut t, &b, &metrics(1800, 0), at(2026, 9, 1, 1));
        assert_eq!(t.rx_bytes, 0); // rolled into September, sample only re-baselines
        assert_eq!(t.cycle_start, at(2026, 9, 1, 0).timestamp());
        apply_metrics(&mut t, &b, &metrics(2100, 0), at(2026, 9, 1, 2));
        assert_eq!(t.rx_bytes, 300);
    }
}
