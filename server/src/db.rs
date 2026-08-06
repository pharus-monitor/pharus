use anyhow::Result;
use pharus_common::{BillingCycle, BillingInfo, Currency, Metrics};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;

        CREATE TABLE IF NOT EXISTS agents (
          id            INTEGER PRIMARY KEY AUTOINCREMENT,
          name          TEXT NOT NULL,
          token         TEXT NOT NULL UNIQUE,
          region_code   TEXT,
          region_source TEXT NOT NULL DEFAULT 'auto',
          created_at    INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS metrics_history (
          id         INTEGER PRIMARY KEY AUTOINCREMENT,
          agent_id   INTEGER NOT NULL,
          ts         INTEGER NOT NULL,
          cpu_usage  REAL NOT NULL,
          mem_used   INTEGER NOT NULL,
          mem_total  INTEGER NOT NULL,
          swap_used  INTEGER NOT NULL,
          swap_total INTEGER NOT NULL,
          disk_used  INTEGER NOT NULL,
          disk_total INTEGER NOT NULL,
          net_rx_bps INTEGER NOT NULL,
          net_tx_bps INTEGER NOT NULL,
          load1      REAL NOT NULL,
          uptime     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_metrics_history_agent_ts
          ON metrics_history (agent_id, ts);

        CREATE TABLE IF NOT EXISTS settings (
          key   TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS traffic_usage (
          agent_id      INTEGER PRIMARY KEY,
          cycle_start   INTEGER NOT NULL DEFAULT 0,
          rx_bytes      INTEGER NOT NULL DEFAULT 0,
          tx_bytes      INTEGER NOT NULL DEFAULT 0,
          last_rx_total INTEGER,
          last_tx_total INTEGER,
          updated_at    INTEGER NOT NULL DEFAULT 0
        );
        ",
    )?;
    migrate(conn)?;
    Ok(())
}

/// Idempotent schema migration for databases created before v0.1.
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= 2 {
        return Ok(());
    }
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(agents)")?;
        let mapped = stmt.query_map([], |r| r.get::<_, String>(1))?;
        mapped.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let columns = [
        ("reset_day", "INTEGER"),
        ("quota_bytes", "INTEGER"),
        ("expires_at", "INTEGER"),
        ("price", "REAL"),
        ("currency", "TEXT"),
        ("billing_cycle", "TEXT"),
    ];
    for (name, ty) in columns {
        if !existing.iter().any(|c| c == name) {
            conn.execute(&format!("ALTER TABLE agents ADD COLUMN {name} {ty}"), [])?;
        }
    }
    conn.execute("PRAGMA user_version = 2", [])?;
    Ok(())
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn add_agent(conn: &Connection, name: &str) -> Result<(i64, String)> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO agents (name, token, created_at) VALUES (?1, ?2, ?3)",
        params![name, token, now_ts()],
    )?;
    Ok((conn.last_insert_rowid(), token))
}

pub fn find_by_token(conn: &Connection, token: &str) -> Result<Option<(i64, String)>> {
    let row = conn
        .query_row(
            "SELECT id, name FROM agents WHERE token = ?1",
            params![token],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row)
}

pub fn list_agents(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, name FROM agents ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn insert_metrics(conn: &Connection, agent_id: i64, m: &Metrics) -> Result<()> {
    conn.execute(
        "INSERT INTO metrics_history
         (agent_id, ts, cpu_usage, mem_used, mem_total, swap_used, swap_total,
          disk_used, disk_total, net_rx_bps, net_tx_bps, load1, uptime)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            agent_id,
            now_ts(),
            m.cpu_usage,
            m.mem_used as i64,
            m.mem_total as i64,
            m.swap_used as i64,
            m.swap_total as i64,
            m.disk_used as i64,
            m.disk_total as i64,
            m.net_rx_bps as i64,
            m.net_tx_bps as i64,
            m.load1,
            m.uptime as i64,
        ],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let v = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct TrafficRow {
    pub cycle_start: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub last_rx_total: Option<u64>,
    pub last_tx_total: Option<u64>,
}

fn currency_to_str(c: Currency) -> &'static str {
    match c {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
        Currency::EUR => "EUR",
    }
}

fn currency_from_str(s: &str) -> Option<Currency> {
    match s {
        "CNY" => Some(Currency::CNY),
        "USD" => Some(Currency::USD),
        "EUR" => Some(Currency::EUR),
        _ => None,
    }
}

fn cycle_to_str(c: BillingCycle) -> &'static str {
    match c {
        BillingCycle::Monthly => "monthly",
        BillingCycle::Quarterly => "quarterly",
        BillingCycle::Yearly => "yearly",
    }
}

fn cycle_from_str(s: &str) -> Option<BillingCycle> {
    match s {
        "monthly" => Some(BillingCycle::Monthly),
        "quarterly" => Some(BillingCycle::Quarterly),
        "yearly" => Some(BillingCycle::Yearly),
        _ => None,
    }
}

pub fn list_billing(conn: &Connection) -> Result<HashMap<i64, BillingInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, reset_day, quota_bytes, expires_at, price, currency, billing_cycle
         FROM agents",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let currency: Option<String> = r.get(5)?;
            let cycle: Option<String> = r.get(6)?;
            let quota: Option<i64> = r.get(2)?;
            Ok((
                r.get::<_, i64>(0)?,
                BillingInfo {
                    reset_day: r.get::<_, Option<i64>>(1)?.map(|v| v as u8),
                    quota_bytes: quota.map(|v| v as u64),
                    expires_at: r.get(3)?,
                    price: r.get(4)?,
                    currency: currency.and_then(|s| currency_from_str(&s)),
                    cycle: cycle.and_then(|s| cycle_from_str(&s)),
                },
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn set_billing(conn: &Connection, agent_id: i64, b: &BillingInfo) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE agents SET reset_day = ?2, quota_bytes = ?3, expires_at = ?4,
         price = ?5, currency = ?6, billing_cycle = ?7 WHERE id = ?1",
        params![
            agent_id,
            b.reset_day.map(|v| v as i64),
            b.quota_bytes.map(|v| v as i64),
            b.expires_at,
            b.price,
            b.currency.map(currency_to_str),
            b.cycle.map(cycle_to_str),
        ],
    )?;
    Ok(rows)
}

pub fn load_traffic(conn: &Connection) -> Result<HashMap<i64, TrafficRow>> {
    let mut stmt = conn.prepare(
        "SELECT agent_id, cycle_start, rx_bytes, tx_bytes, last_rx_total, last_tx_total
         FROM traffic_usage",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                TrafficRow {
                    cycle_start: r.get(1)?,
                    rx_bytes: r.get::<_, i64>(2)? as u64,
                    tx_bytes: r.get::<_, i64>(3)? as u64,
                    last_rx_total: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                    last_tx_total: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                },
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn upsert_traffic(conn: &Connection, agent_id: i64, t: &TrafficRow) -> Result<()> {
    conn.execute(
        "INSERT INTO traffic_usage
           (agent_id, cycle_start, rx_bytes, tx_bytes, last_rx_total, last_tx_total, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(agent_id) DO UPDATE SET
           cycle_start = excluded.cycle_start,
           rx_bytes = excluded.rx_bytes,
           tx_bytes = excluded.tx_bytes,
           last_rx_total = excluded.last_rx_total,
           last_tx_total = excluded.last_tx_total,
           updated_at = excluded.updated_at",
        params![
            agent_id,
            t.cycle_start,
            t.rx_bytes as i64,
            t.tx_bytes as i64,
            t.last_rx_total.map(|v| v as i64),
            t.last_tx_total.map(|v| v as i64),
            now_ts(),
        ],
    )?;
    Ok(())
}
