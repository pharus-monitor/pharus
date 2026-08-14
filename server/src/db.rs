use anyhow::Result;
use pharus_common::{
    BillingCycle, BillingInfo, Currency, CustomTaskSpec, Metrics, PingKind, PingResult,
    PingTaskSpec, Region, UnlockResult,
};
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

        CREATE TABLE IF NOT EXISTS users (
          id            INTEGER PRIMARY KEY AUTOINCREMENT,
          username      TEXT NOT NULL UNIQUE,
          password_hash TEXT NOT NULL,
          created_at    INTEGER NOT NULL
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

        CREATE TABLE IF NOT EXISTS ping_tasks (
          id           INTEGER PRIMARY KEY AUTOINCREMENT,
          agent_id     INTEGER,
          label        TEXT NOT NULL,
          kind         TEXT NOT NULL DEFAULT 'tcp',
          target       TEXT NOT NULL,
          port         INTEGER,
          interval_sec INTEGER NOT NULL DEFAULT 60,
          probe_count  INTEGER NOT NULL DEFAULT 4,
          enabled      INTEGER NOT NULL DEFAULT 1,
          created_at   INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ping_tasks_agent ON ping_tasks (agent_id);

        CREATE TABLE IF NOT EXISTS ping_history (
          id       INTEGER PRIMARY KEY AUTOINCREMENT,
          agent_id INTEGER NOT NULL,
          task_id  INTEGER NOT NULL,
          ts       INTEGER NOT NULL,
          rtt_avg  REAL,
          rtt_min  REAL,
          rtt_max  REAL,
          loss     REAL NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ping_history_lookup
          ON ping_history (agent_id, task_id, ts);

        CREATE TABLE IF NOT EXISTS tasks (
          id           INTEGER PRIMARY KEY AUTOINCREMENT,
          name         TEXT NOT NULL,
          command      TEXT NOT NULL,
          agent_id     INTEGER,
          interval_sec INTEGER NOT NULL DEFAULT 0,
          timeout_sec  INTEGER NOT NULL DEFAULT 30,
          enabled      INTEGER NOT NULL DEFAULT 1,
          created_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS task_results (
          id        INTEGER PRIMARY KEY AUTOINCREMENT,
          task_id   INTEGER NOT NULL,
          agent_id  INTEGER NOT NULL,
          ts        INTEGER NOT NULL,
          exit_code INTEGER NOT NULL,
          output    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_results_lookup ON task_results (task_id, ts);

        CREATE TABLE IF NOT EXISTS agent_settings (
          agent_id INTEGER NOT NULL,
          key      TEXT NOT NULL,
          value    TEXT NOT NULL,
          PRIMARY KEY (agent_id, key)
        );

        CREATE TABLE IF NOT EXISTS alert_rules (
          id         INTEGER PRIMARY KEY AUTOINCREMENT,
          name       TEXT NOT NULL,
          kind       TEXT NOT NULL,
          agent_id   INTEGER,
          metric     TEXT,
          op         TEXT NOT NULL DEFAULT '>',
          threshold  REAL NOT NULL DEFAULT 0,
          duration   INTEGER NOT NULL DEFAULT 300,
          ratio      REAL NOT NULL DEFAULT 1.0,
          channels   TEXT NOT NULL DEFAULT '[]',
          enabled    INTEGER NOT NULL DEFAULT 1,
          cooldown   INTEGER NOT NULL DEFAULT 1800,
          task_id    INTEGER,
          consecutive INTEGER NOT NULL DEFAULT 1,
          created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS alert_state (
          rule_id     INTEGER NOT NULL,
          agent_id    INTEGER NOT NULL,
          firing      INTEGER NOT NULL DEFAULT 0,
          since       INTEGER NOT NULL DEFAULT 0,
          last_notify INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY (rule_id, agent_id)
        );

        CREATE TABLE IF NOT EXISTS notification_channels (
          id         INTEGER PRIMARY KEY AUTOINCREMENT,
          name       TEXT NOT NULL,
          kind       TEXT NOT NULL,
          config     TEXT NOT NULL DEFAULT '{}',
          enabled    INTEGER NOT NULL DEFAULT 1,
          failed_streak INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS streaming_results (
          agent_id INTEGER NOT NULL,
          service  TEXT NOT NULL,
          status   TEXT NOT NULL,
          detail   TEXT,
          ts       INTEGER NOT NULL,
          PRIMARY KEY (agent_id, service)
        );

        CREATE TABLE IF NOT EXISTS themes (
          id           TEXT PRIMARY KEY,
          name         TEXT NOT NULL,
          version      TEXT NOT NULL,
          author       TEXT,
          description  TEXT,
          preview      TEXT,
          source       TEXT NOT NULL,
          dir          TEXT NOT NULL,
          installed_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS streaming_history (
          id       INTEGER PRIMARY KEY AUTOINCREMENT,
          agent_id INTEGER NOT NULL,
          service  TEXT NOT NULL,
          status   TEXT NOT NULL,
          detail   TEXT,
          ts       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_streaming_history_lookup
          ON streaming_history (agent_id, service, ts);

        CREATE TABLE IF NOT EXISTS iperf3_log (
          id        INTEGER PRIMARY KEY AUTOINCREMENT,
          agent_id  INTEGER NOT NULL,
          client_ip TEXT NOT NULL,
          target    TEXT NOT NULL,
          region    TEXT,
          asn       TEXT,
          ts        INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_iperf3_log_ts ON iperf3_log (ts);
        ",
    )?;
    migrate(conn)?;
    Ok(())
}

const SCHEMA_VERSION: i64 = 15;

fn has_column(conn: &Connection, table: &str, col: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(existing.iter().any(|c| c == col))
}

fn add_column(conn: &Connection, table: &str, col: &str, ty: &str) -> Result<()> {
    if !has_column(conn, table, col)? {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"), [])?;
    }
    Ok(())
}

/// Idempotent schema migration for databases created by earlier releases.
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    if version < 2 {
        for (name, ty) in [
            ("reset_day", "INTEGER"),
            ("quota_bytes", "INTEGER"),
            ("expires_at", "INTEGER"),
            ("price", "REAL"),
            ("currency", "TEXT"),
            ("billing_cycle", "TEXT"),
        ] {
            add_column(conn, "agents", name, ty)?;
        }
    }
    if version < 4 {
        // ping_tasks / tasks gain a multi-agent JSON column; the old single
        // agent_id column stays as a compatibility mirror (NULL = all).
        add_column(conn, "ping_tasks", "agent_ids", "TEXT")?;
        add_column(conn, "tasks", "agent_ids", "TEXT")?;
    }
    if version < 5 {
        add_column(conn, "agents", "bandwidth", "REAL")?;
    }
    if version < 6 {
        // Per-host traffic accounting mode and uni-direction pick.
        add_column(conn, "agents", "traffic_mode", "TEXT")?;
        add_column(conn, "agents", "traffic_dir", "TEXT")?;
        // Hostname reported by agents, used to match shared-secret logins.
        add_column(conn, "agents", "hostname", "TEXT")?;
    }
    if version < 7 {
        // Alert notify cooldown and per-task failure rules.
        add_column(conn, "alert_rules", "cooldown", "INTEGER NOT NULL DEFAULT 1800")?;
        add_column(conn, "alert_rules", "task_id", "INTEGER")?;
    }
    if version < 8 {
        // Consecutive-failure threshold for task rules.
        add_column(conn, "alert_rules", "consecutive", "INTEGER NOT NULL DEFAULT 1")?;
    }
    if version < 9 {
        // Notification channel consecutive-failure health counter.
        add_column(conn, "notification_channels", "failed_streak", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if version < 10 {
        // Installed themes registry.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS themes (
              id           TEXT PRIMARY KEY,
              name         TEXT NOT NULL,
              version      TEXT NOT NULL,
              author       TEXT,
              description  TEXT,
              preview      TEXT,
              source       TEXT NOT NULL,
              dir          TEXT NOT NULL,
              installed_at INTEGER NOT NULL
            )",
            [],
        )?;
    }
    if version < 11 {
        // Bounded streaming-check history (kept ~30 days).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS streaming_history (
              id       INTEGER PRIMARY KEY AUTOINCREMENT,
              agent_id INTEGER NOT NULL,
              service  TEXT NOT NULL,
              status   TEXT NOT NULL,
              detail   TEXT,
              ts       INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_streaming_history_lookup
             ON streaming_history (agent_id, service, ts)",
            [],
        )?;
    }
    if version < 12 {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS iperf3_log (
              id        INTEGER PRIMARY KEY AUTOINCREMENT,
              agent_id  INTEGER NOT NULL,
              client_ip TEXT NOT NULL,
              target    TEXT NOT NULL,
              region    TEXT,
              asn       TEXT,
              ts        INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_iperf3_log_ts ON iperf3_log (ts)",
            [],
        )?;
    }
    if version < 13 {
        add_column(conn, "metrics_history", "disk_write_bps", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if version < 14 {
        add_column(conn, "metrics_history", "disk_read_bps", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if version < 15 {
        // Multi-user: role (admin/viewer) and account disable switch.
        add_column(conn, "users", "role", "TEXT NOT NULL DEFAULT 'admin'")?;
        add_column(conn, "users", "enabled", "INTEGER NOT NULL DEFAULT 1")?;
    }
    conn.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION}"), [])?;
    Ok(())
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn add_agent(conn: &Connection, name: &str) -> Result<(i64, String)> {
    add_agent_named(conn, name, None)
}

pub fn add_agent_named(
    conn: &Connection,
    name: &str,
    hostname: Option<&str>,
) -> Result<(i64, String)> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO agents (name, token, hostname, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, token, hostname, now_ts()],
    )?;
    Ok((conn.last_insert_rowid(), token))
}

pub fn find_agent_by_hostname(conn: &Connection, hostname: &str) -> Result<Option<(i64, String)>> {
    let row = conn
        .query_row(
            "SELECT id, name FROM agents WHERE hostname = ?1 ORDER BY id LIMIT 1",
            params![hostname],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row)
}

pub fn rename_agent(conn: &Connection, id: i64, name: &str) -> Result<usize> {
    Ok(conn.execute("UPDATE agents SET name = ?1 WHERE id = ?2", params![name, id])?)
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

/// Admin console only: the per-agent tokens, so an operator can re-issue a
/// config without SSHing to the server for `add-agent` output.
pub fn list_agent_tokens(conn: &Connection) -> Result<HashMap<i64, String>> {
    let mut stmt = conn.prepare("SELECT id, token FROM agents")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn insert_metrics(conn: &Connection, agent_id: i64, m: &Metrics) -> Result<()> {
    conn.execute(
        "INSERT INTO metrics_history
         (agent_id, ts, cpu_usage, mem_used, mem_total, swap_used, swap_total,
          disk_used, disk_total, net_rx_bps, net_tx_bps, load1, uptime, disk_write_bps, disk_read_bps)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
            m.disk_write_bps as i64,
            m.disk_read_bps as i64,
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

pub fn clear_setting(conn: &Connection, key: &str) -> Result<usize> {
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub enabled: bool,
    pub created_at: i64,
}

pub fn count_users(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
}

pub fn find_user(conn: &Connection, username: &str) -> Result<Option<(i64, String, String, bool)>> {
    let row = conn
        .query_row(
            "SELECT id, password_hash, role, enabled FROM users WHERE username = ?1",
            params![username],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)? != 0)),
        )
        .optional()?;
    Ok(row)
}

pub fn find_user_by_id(conn: &Connection, id: i64) -> Result<Option<UserRow>> {
    let row = conn
        .query_row(
            "SELECT id, username, role, enabled, created_at FROM users WHERE id = ?1",
            params![id],
            |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    role: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? != 0,
                    created_at: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn list_users(conn: &Connection) -> Result<Vec<UserRow>> {
    let mut stmt = conn.prepare("SELECT id, username, role, enabled, created_at FROM users ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(UserRow {
                id: r.get(0)?,
                username: r.get(1)?,
                role: r.get(2)?,
                enabled: r.get::<_, i64>(3)? != 0,
                created_at: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn insert_user(conn: &Connection, username: &str, password_hash: &str, role: &str) -> Result<usize> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO users (username, password_hash, role, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![username, password_hash, role, now_ts()],
    )?)
}

pub fn update_user_password(
    conn: &Connection,
    username: &str,
    password_hash: &str,
) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE users SET password_hash = ?2 WHERE username = ?1",
        params![username, password_hash],
    )?)
}

pub fn update_user_role(conn: &Connection, id: i64, role: &str) -> Result<usize> {
    Ok(conn.execute("UPDATE users SET role = ?2 WHERE id = ?1", params![id, role])?)
}

pub fn set_user_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE users SET enabled = ?2 WHERE id = ?1",
        params![id, if enabled { 1 } else { 0 }],
    )?)
}

pub fn delete_user(conn: &Connection, id: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM users WHERE id = ?1", params![id])?)
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
        "SELECT id, reset_day, quota_bytes, expires_at, price, currency, billing_cycle, bandwidth,
                traffic_mode, traffic_dir
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
                    bandwidth: r.get(7)?,
                    traffic_mode: r.get::<_, Option<String>>(8)?.filter(|s| !s.is_empty()),
                    traffic_dir: r.get::<_, Option<String>>(9)?.filter(|s| !s.is_empty()),
                },
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn set_billing(conn: &Connection, agent_id: i64, b: &BillingInfo) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE agents SET reset_day = ?2, quota_bytes = ?3, expires_at = ?4,
         price = ?5, currency = ?6, billing_cycle = ?7, bandwidth = ?8,
         traffic_mode = ?9, traffic_dir = ?10 WHERE id = ?1",
        params![
            agent_id,
            b.reset_day.map(|v| v as i64),
            b.quota_bytes.map(|v| v as i64),
            b.expires_at,
            b.price,
            b.currency.map(currency_to_str),
            b.cycle.map(cycle_to_str),
            b.bandwidth,
            b.traffic_mode.as_deref(),
            b.traffic_dir.as_deref(),
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

// ---------------------------------------------------------------- metrics history

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsPoint {
    pub ts: i64,
    pub cpu_usage: f64,
    pub mem_used: i64,
    pub mem_total: i64,
    pub disk_used: i64,
    pub disk_total: i64,
    pub net_rx_bps: i64,
    pub net_tx_bps: i64,
    pub load1: f64,
    pub disk_write_bps: i64,
    pub disk_read_bps: i64,
}

pub fn metrics_history(
    conn: &Connection,
    agent_id: i64,
    since: i64,
    limit: i64,
) -> Result<Vec<MetricsPoint>> {
    let mut stmt = conn.prepare(
        "SELECT ts, cpu_usage, mem_used, mem_total, disk_used, disk_total,
                net_rx_bps, net_tx_bps, load1, disk_write_bps, disk_read_bps
         FROM metrics_history
         WHERE agent_id = ?1 AND ts >= ?2
         ORDER BY ts DESC LIMIT ?3",
    )?;
    let mut rows = stmt
        .query_map(params![agent_id, since, limit], |r| {
            Ok(MetricsPoint {
                ts: r.get(0)?,
                cpu_usage: r.get(1)?,
                mem_used: r.get(2)?,
                mem_total: r.get(3)?,
                disk_used: r.get(4)?,
                disk_total: r.get(5)?,
                net_rx_bps: r.get(6)?,
                net_tx_bps: r.get(7)?,
                load1: r.get(8)?,
                disk_write_bps: r.get(9)?,
                disk_read_bps: r.get(10)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(rows)
}

// ---------------------------------------------------------------- ping tasks

pub fn ping_kind_from_str(s: &str) -> Option<PingKind> {
    match s {
        "icmp" => Some(PingKind::Icmp),
        "tcp" => Some(PingKind::Tcp),
        "http" => Some(PingKind::Http),
        "ssl" => Some(PingKind::Ssl),
        _ => None,
    }
}

fn parse_agent_ids(s: Option<String>) -> Vec<i64> {
    match s {
        None => Vec::new(),
        Some(t) => serde_json::from_str(&t).unwrap_or_default(),
    }
}

fn agent_ids_to_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// Mirror for the legacy single-agent column: a one-element list is stored in
/// agent_id, anything else (empty = all, multiple) leaves it NULL.
fn compat_agent_id(ids: &[i64]) -> Option<i64> {
    if ids.len() == 1 {
        ids.first().copied()
    } else {
        None
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PingTaskRow {
    pub id: i64,
    /// Legacy mirror of `agent_ids` (single agent). Kept for compatibility.
    pub agent_id: Option<i64>,
    /// Empty = applies to every agent; otherwise the targeted agent ids.
    pub agent_ids: Vec<i64>,
    pub label: String,
    pub kind: String,
    pub target: String,
    pub port: Option<u16>,
    pub interval_sec: u64,
    pub probe_count: u32,
    pub enabled: bool,
}

fn ping_task_from_row(r: &rusqlite::Row) -> rusqlite::Result<PingTaskRow> {
    Ok(PingTaskRow {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        agent_ids: parse_agent_ids(r.get(9)?),
        label: r.get(2)?,
        kind: r.get(3)?,
        target: r.get(4)?,
        port: r.get::<_, Option<i64>>(5)?.map(|v| v as u16),
        interval_sec: r.get::<_, i64>(6)? as u64,
        probe_count: r.get::<_, i64>(7)? as u32,
        enabled: r.get::<_, i64>(8)? != 0,
    })
}

const PING_TASK_COLS: &str =
    "id, agent_id, label, kind, target, port, interval_sec, probe_count, enabled, agent_ids";

pub fn list_ping_tasks(conn: &Connection) -> Result<Vec<PingTaskRow>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {PING_TASK_COLS} FROM ping_tasks ORDER BY id"))?;
    let mut rows = stmt
        .query_map([], ping_task_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Operator-defined display order (admin drag-sort), stored as a JSON array
    // of task ids; unlisted (new) tasks keep their id order at the end.
    if let Ok(Some(text)) = get_setting(conn, "ping_task_order") {
        if let Ok(order) = serde_json::from_str::<Vec<i64>>(&text) {
            let pos: HashMap<i64, usize> = order
                .into_iter()
                .enumerate()
                .map(|(i, id)| (id, i))
                .collect();
            rows.sort_by_key(|r| pos.get(&r.id).copied().unwrap_or(usize::MAX));
        }
    }
    Ok(rows)
}

/// Enabled tasks that apply to `agent_id`, converted to the wire spec.
pub fn ping_tasks_for(conn: &Connection, agent_id: i64) -> Result<Vec<PingTaskSpec>> {
    Ok(list_ping_tasks(conn)?
        .into_iter()
        .filter(|t| t.enabled && (t.agent_ids.is_empty() || t.agent_ids.contains(&agent_id)))
        .map(|t| PingTaskSpec {
            id: t.id,
            label: t.label,
            kind: ping_kind_from_str(&t.kind).unwrap_or(PingKind::Tcp),
            target: t.target,
            port: t.port,
            interval: t.interval_sec,
            count: t.probe_count,
        })
        .collect())
}

pub fn insert_ping_task(conn: &Connection, t: &PingTaskRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO ping_tasks
           (agent_id, agent_ids, label, kind, target, port, interval_sec, probe_count, enabled, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            compat_agent_id(&t.agent_ids),
            agent_ids_to_json(&t.agent_ids),
            t.label,
            t.kind,
            t.target,
            t.port.map(|v| v as i64),
            t.interval_sec as i64,
            t.probe_count as i64,
            t.enabled as i64,
            now_ts(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_ping_task(conn: &Connection, id: i64, t: &PingTaskRow) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE ping_tasks SET agent_id = ?2, agent_ids = ?3, label = ?4, kind = ?5, target = ?6,
           port = ?7, interval_sec = ?8, probe_count = ?9, enabled = ?10 WHERE id = ?1",
        params![
            id,
            compat_agent_id(&t.agent_ids),
            agent_ids_to_json(&t.agent_ids),
            t.label,
            t.kind,
            t.target,
            t.port.map(|v| v as i64),
            t.interval_sec as i64,
            t.probe_count as i64,
            t.enabled as i64,
        ],
    )?;
    Ok(rows)
}

pub fn delete_ping_task(conn: &Connection, id: i64) -> Result<usize> {
    conn.execute("DELETE FROM ping_history WHERE task_id = ?1", params![id])?;
    Ok(conn.execute("DELETE FROM ping_tasks WHERE id = ?1", params![id])?)
}

/// The agent resends its *whole* ping result set whenever any single task
/// re-probes, so a task on a long interval would otherwise get a duplicate row
/// every time a faster task ticks. A task cannot produce two samples closer
/// together than its own interval, so anything arriving sooner is a resend of
/// one already stored. Value comparison would be wrong here: a target that
/// stays down reports identical rows that must all be kept.
pub fn insert_ping_history(conn: &Connection, agent_id: i64, p: &PingResult) -> Result<()> {
    let Some(task_id) = p.task_id else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO ping_history (agent_id, task_id, ts, rtt_avg, rtt_min, rtt_max, loss)
         SELECT ?1,?2,?3,?4,?5,?6,?7
         WHERE NOT EXISTS (
           SELECT 1 FROM ping_history h
           WHERE h.agent_id = ?1 AND h.task_id = ?2
             AND h.ts > ?3 - COALESCE(
               (SELECT MAX(t.interval_sec - 2, 1) FROM ping_tasks t WHERE t.id = ?2), 1)
         )",
        params![agent_id, task_id, now_ts(), p.rtt_ms, p.rtt_min, p.rtt_max, p.loss],
    )?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PingPoint {
    pub task_id: i64,
    pub ts: i64,
    pub rtt_avg: Option<f64>,
    pub rtt_min: Option<f64>,
    pub rtt_max: Option<f64>,
    pub loss: f64,
}

pub fn ping_history(
    conn: &Connection,
    agent_id: i64,
    task_id: Option<i64>,
    since: i64,
    limit: i64,
) -> Result<Vec<PingPoint>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, ts, rtt_avg, rtt_min, rtt_max, loss FROM ping_history
         WHERE agent_id = ?1 AND ts >= ?2 AND (?3 IS NULL OR task_id = ?3)
         ORDER BY ts DESC LIMIT ?4",
    )?;
    let mut rows = stmt
        .query_map(params![agent_id, since, task_id, limit], |r| {
            Ok(PingPoint {
                task_id: r.get(0)?,
                ts: r.get(1)?,
                rtt_avg: r.get(2)?,
                rtt_min: r.get(3)?,
                rtt_max: r.get(4)?,
                loss: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(rows)
}

pub fn prune_ping_history(conn: &Connection, before: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM ping_history WHERE ts < ?1", params![before])?)
}

// ---------------------------------------------------------------- custom tasks

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskRow {
    pub id: i64,
    pub name: String,
    pub command: String,
    /// Legacy mirror of `agent_ids` (single agent). Kept for compatibility.
    pub agent_id: Option<i64>,
    /// Empty = applies to every agent; otherwise the targeted agent ids.
    pub agent_ids: Vec<i64>,
    /// Seconds; 0 = manual trigger only
    pub interval_sec: u64,
    pub timeout_sec: u64,
    pub enabled: bool,
}

fn task_from_row(r: &rusqlite::Row) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        name: r.get(1)?,
        command: r.get(2)?,
        agent_id: r.get(3)?,
        agent_ids: parse_agent_ids(r.get(7)?),
        interval_sec: r.get::<_, i64>(4)? as u64,
        timeout_sec: r.get::<_, i64>(5)? as u64,
        enabled: r.get::<_, i64>(6)? != 0,
    })
}

const TASK_COLS: &str = "id, name, command, agent_id, interval_sec, timeout_sec, enabled, agent_ids";

pub fn list_tasks(conn: &Connection) -> Result<Vec<TaskRow>> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM tasks ORDER BY id"))?;
    let rows = stmt
        .query_map([], task_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_task(conn: &Connection, id: i64) -> Result<Option<TaskRow>> {
    let row = conn
        .query_row(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"),
            params![id],
            task_from_row,
        )
        .optional()?;
    Ok(row)
}

/// Enabled periodic tasks that apply to `agent_id`, converted to the wire spec.
/// Manual-only tasks (interval 0) are excluded — they ship via `RunTask`.
pub fn tasks_for(conn: &Connection, agent_id: i64) -> Result<Vec<CustomTaskSpec>> {
    Ok(list_tasks(conn)?
        .into_iter()
        .filter(|t| {
            t.enabled && t.interval_sec > 0 && (t.agent_ids.is_empty() || t.agent_ids.contains(&agent_id))
        })
        .map(|t| CustomTaskSpec {
            id: t.id,
            command: t.command,
            interval: t.interval_sec,
            timeout: t.timeout_sec,
        })
        .collect())
}

pub fn insert_task(conn: &Connection, t: &TaskRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO tasks (name, command, agent_id, agent_ids, interval_sec, timeout_sec, enabled, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            t.name,
            t.command,
            compat_agent_id(&t.agent_ids),
            agent_ids_to_json(&t.agent_ids),
            t.interval_sec as i64,
            t.timeout_sec as i64,
            t.enabled as i64,
            now_ts(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_task(conn: &Connection, id: i64, t: &TaskRow) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE tasks SET name = ?2, command = ?3, agent_id = ?4, agent_ids = ?5,
           interval_sec = ?6, timeout_sec = ?7, enabled = ?8 WHERE id = ?1",
        params![
            id,
            t.name,
            t.command,
            compat_agent_id(&t.agent_ids),
            agent_ids_to_json(&t.agent_ids),
            t.interval_sec as i64,
            t.timeout_sec as i64,
            t.enabled as i64,
        ],
    )?;
    Ok(rows)
}

pub fn delete_task(conn: &Connection, id: i64) -> Result<usize> {
    conn.execute("DELETE FROM task_results WHERE task_id = ?1", params![id])?;
    Ok(conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?)
}

pub fn insert_task_result(
    conn: &Connection,
    task_id: i64,
    agent_id: i64,
    exit_code: i32,
    output: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO task_results (task_id, agent_id, ts, exit_code, output)
         VALUES (?1,?2,?3,?4,?5)",
        params![task_id, agent_id, now_ts(), exit_code, output],
    )?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskResultRow {
    pub id: i64,
    pub task_id: i64,
    pub agent_id: i64,
    pub ts: i64,
    pub exit_code: i32,
    pub output: String,
}

/// Newest result per (task, agent) as `(ts, exit_code)`. Alert evaluation runs
/// this on every tick while holding the db lock, so it deliberately skips the
/// `output` column — those rows carry up to 32 KiB each.
pub fn latest_task_results(conn: &Connection) -> Result<HashMap<(i64, i64), (i64, i32)>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, agent_id, ts, exit_code FROM task_results
         WHERE id IN (SELECT MAX(id) FROM task_results GROUP BY task_id, agent_id)",
    )?;
    let rows = stmt
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?))))?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn list_task_results(
    conn: &Connection,
    task_id: Option<i64>,
    limit: i64,
) -> Result<Vec<TaskResultRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, agent_id, ts, exit_code, output FROM task_results
         WHERE (?1 IS NULL OR task_id = ?1) ORDER BY ts DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![task_id, limit], |r| {
            Ok(TaskResultRow {
                id: r.get(0)?,
                task_id: r.get(1)?,
                agent_id: r.get(2)?,
                ts: r.get(3)?,
                exit_code: r.get(4)?,
                output: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- per-agent settings

pub fn agent_settings(conn: &Connection, agent_id: i64) -> Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT key, value FROM agent_settings WHERE agent_id = ?1")?;
    let rows = stmt
        .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn all_agent_settings(conn: &Connection) -> Result<HashMap<i64, HashMap<String, String>>> {
    let mut stmt = conn.prepare("SELECT agent_id, key, value FROM agent_settings")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out: HashMap<i64, HashMap<String, String>> = HashMap::new();
    for (agent_id, key, value) in rows {
        out.entry(agent_id).or_default().insert(key, value);
    }
    Ok(out)
}

pub fn set_agent_setting(
    conn: &Connection,
    agent_id: i64,
    key: &str,
    value: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_settings (agent_id, key, value) VALUES (?1,?2,?3)
         ON CONFLICT(agent_id, key) DO UPDATE SET value = excluded.value",
        params![agent_id, key, value],
    )?;
    Ok(())
}

pub fn clear_agent_setting(conn: &Connection, agent_id: i64, key: &str) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM agent_settings WHERE agent_id = ?1 AND key = ?2",
        params![agent_id, key],
    )?)
}

// ---------------------------------------------------------------- region

pub fn set_region(conn: &Connection, agent_id: i64, code: Option<&str>, source: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE agents SET region_code = ?2, region_source = ?3 WHERE id = ?1",
        params![agent_id, code, source],
    )?)
}

/// Returns `(code, source)` per agent for agents that have a region set.
pub fn list_regions(conn: &Connection) -> Result<HashMap<i64, (String, String)>> {
    let mut stmt = conn
        .prepare("SELECT id, region_code, region_source FROM agents WHERE region_code IS NOT NULL")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn region_source(conn: &Connection, agent_id: i64) -> Result<Option<String>> {
    let v = conn
        .query_row(
            "SELECT region_source FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v)
}

pub fn make_region(code: &str, source: &str) -> Region {
    Region {
        code: code.to_string(),
        name: crate::regions::region_name(code).to_string(),
        source: source.to_string(),
    }
}

// ---------------------------------------------------------------- streaming results

pub fn replace_streaming(conn: &Connection, agent_id: i64, results: &[UnlockResult]) -> Result<()> {
    let ts = now_ts();
    for r in results {
        conn.execute(
            "INSERT INTO streaming_results (agent_id, service, status, detail, ts)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(agent_id, service) DO UPDATE SET
               status = excluded.status, detail = excluded.detail, ts = excluded.ts",
            params![agent_id, r.service, r.status, r.detail, ts],
        )?;
    }
    Ok(())
}

pub fn list_streaming(conn: &Connection, agent_id: i64) -> Result<Vec<UnlockResult>> {
    let mut stmt = conn.prepare(
        "SELECT service, status, detail FROM streaming_results
         WHERE agent_id = ?1 ORDER BY service",
    )?;
    let rows = stmt
        .query_map(params![agent_id], |r| {
            Ok(UnlockResult {
                service: r.get(0)?,
                status: r.get(1)?,
                detail: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamingHistoryRow {
    pub service: String,
    pub status: String,
    pub detail: Option<String>,
    pub ts: i64,
}

/// Append a check result to the streaming history, keep at most
/// [`MAX_STREAMING_ROWS`] rows per agent, and prune samples older than the
/// 30-day retention window.
pub fn append_streaming_history(
    conn: &Connection,
    agent_id: i64,
    results: &[UnlockResult],
) -> Result<()> {
    let now = now_ts();
    for r in results {
        conn.execute(
            "INSERT INTO streaming_history (agent_id, service, status, detail, ts)
             VALUES (?1,?2,?3,?4,?5)",
            params![agent_id, r.service, r.status, r.detail, now],
        )?;
    }
    let cutoff = now - 30 * 86_400;
    conn.execute(
        "DELETE FROM streaming_history WHERE agent_id = ?1 AND ts < ?2",
        params![agent_id, cutoff],
    )?;
    conn.execute(
        "DELETE FROM streaming_history WHERE id IN (
           SELECT id FROM streaming_history WHERE agent_id = ?1
           ORDER BY id DESC LIMIT -1 OFFSET ?2
         )",
        params![agent_id, MAX_STREAMING_ROWS],
    )?;
    Ok(())
}

/// Cap on history rows kept per agent (checked after every append).
const MAX_STREAMING_ROWS: i64 = 500;

pub fn list_streaming_history(
    conn: &Connection,
    agent_id: i64,
    since: i64,
    limit: i64,
) -> Result<Vec<StreamingHistoryRow>> {
    let mut stmt = conn.prepare(
        "SELECT service, status, detail, ts FROM streaming_history
         WHERE agent_id = ?1 AND ts >= ?2
         ORDER BY ts DESC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![agent_id, since, limit], |r| {
            Ok(StreamingHistoryRow {
                service: r.get(0)?,
                status: r.get(1)?,
                detail: r.get(2)?,
                ts: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Recent streaming checks across every agent (agent_id paired with the row).
pub fn list_streaming_history_all(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<(i64, StreamingHistoryRow)>> {
    let mut stmt = conn.prepare(
        "SELECT agent_id, service, status, detail, ts FROM streaming_history
         ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                StreamingHistoryRow {
                    service: r.get(1)?,
                    status: r.get(2)?,
                    detail: r.get(3)?,
                    ts: r.get(4)?,
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- iperf3 log

#[derive(Debug, Clone, serde::Serialize)]
pub struct Iperf3LogRow {
    pub id: i64,
    pub agent_id: i64,
    pub client_ip: String,
    pub target: String,
    pub region: Option<String>,
    pub asn: Option<String>,
    pub ts: i64,
}

pub fn insert_iperf3_log(
    conn: &Connection,
    agent_id: i64,
    client_ip: &str,
    target: &str,
    ts: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO iperf3_log (agent_id, client_ip, target, region, asn, ts)
         VALUES (?1,?2,?3,NULL,NULL,?4)",
        params![agent_id, client_ip, target, ts],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_iperf3_log(
    conn: &Connection,
    id: i64,
    region: Option<&str>,
    asn: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE iperf3_log SET region = ?2, asn = ?3 WHERE id = ?1",
        params![id, region, asn],
    )?;
    Ok(())
}

pub fn list_iperf3_log(conn: &Connection, limit: i64) -> Result<Vec<Iperf3LogRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, client_ip, target, region, asn, ts
         FROM iperf3_log ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(Iperf3LogRow {
                id: r.get(0)?,
                agent_id: r.get(1)?,
                client_ip: r.get(2)?,
                target: r.get(3)?,
                region: r.get(4)?,
                asn: r.get(5)?,
                ts: r.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- alert rules

#[derive(Debug, Clone, serde::Serialize)]
pub struct AlertRuleRow {
    pub id: i64,
    pub name: String,
    /// "metric" | "offline" | "task"
    pub kind: String,
    /// None = applies to every agent
    pub agent_id: Option<i64>,
    /// "cpu" | "mem" | "disk" | "load" | "traffic" — only for kind = "metric"
    pub metric: Option<String>,
    /// ">" | "<"
    pub op: String,
    pub threshold: f64,
    /// Observation window in seconds
    pub duration: i64,
    /// Fraction of the window that must breach before firing, 0.0..=1.0
    pub ratio: f64,
    /// Notification channel ids
    pub channels: Vec<i64>,
    pub enabled: bool,
    /// Seconds between notifications for repeated firings (default 1800).
    pub cooldown: i64,
    /// For kind = "task": only fire on failures of this task.
    pub task_id: Option<i64>,
    /// For kind = "task": number of consecutive failures required to fire.
    pub consecutive: i32,
}

fn alert_rule_from_row(r: &rusqlite::Row) -> rusqlite::Result<AlertRuleRow> {
    let channels: String = r.get(8)?;
    Ok(AlertRuleRow {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        agent_id: r.get(3)?,
        metric: r.get(4)?,
        op: r.get(5)?,
        threshold: r.get(6)?,
        duration: r.get(7)?,
        ratio: r.get(9)?,
        channels: serde_json::from_str(&channels).unwrap_or_default(),
        enabled: r.get::<_, i64>(10)? != 0,
        cooldown: r.get::<_, i64>(11)?,
        task_id: r.get(12)?,
        consecutive: r.get::<_, i32>(13)?,
    })
}

const ALERT_RULE_COLS: &str =
    "id, name, kind, agent_id, metric, op, threshold, duration, channels, ratio, enabled, cooldown, task_id, consecutive";

pub fn list_alert_rules(conn: &Connection) -> Result<Vec<AlertRuleRow>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {ALERT_RULE_COLS} FROM alert_rules ORDER BY id"))?;
    let rows = stmt
        .query_map([], alert_rule_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn insert_alert_rule(conn: &Connection, a: &AlertRuleRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO alert_rules
           (name, kind, agent_id, metric, op, threshold, duration, ratio, channels, enabled, cooldown, task_id, consecutive, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            a.name,
            a.kind,
            a.agent_id,
            a.metric,
            a.op,
            a.threshold,
            a.duration,
            a.ratio,
            serde_json::to_string(&a.channels).unwrap_or_else(|_| "[]".into()),
            a.enabled as i64,
            a.cooldown,
            a.task_id,
            a.consecutive,
            now_ts(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_alert_rule(conn: &Connection, id: i64, a: &AlertRuleRow) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE alert_rules SET name = ?2, kind = ?3, agent_id = ?4, metric = ?5, op = ?6,
           threshold = ?7, duration = ?8, ratio = ?9, channels = ?10, enabled = ?11,
           cooldown = ?12, task_id = ?13, consecutive = ?14 WHERE id = ?1",
        params![
            id,
            a.name,
            a.kind,
            a.agent_id,
            a.metric,
            a.op,
            a.threshold,
            a.duration,
            a.ratio,
            serde_json::to_string(&a.channels).unwrap_or_else(|_| "[]".into()),
            a.enabled as i64,
            a.cooldown,
            a.task_id,
            a.consecutive,
        ],
    )?;
    Ok(rows)
}

pub fn delete_alert_rule(conn: &Connection, id: i64) -> Result<usize> {
    conn.execute("DELETE FROM alert_state WHERE rule_id = ?1", params![id])?;
    Ok(conn.execute("DELETE FROM alert_rules WHERE id = ?1", params![id])?)
}

#[derive(Debug, Clone, Default)]
pub struct AlertStateRow {
    pub firing: bool,
    /// When the current firing streak began (unix seconds)
    pub since: i64,
    /// When the last notification was sent (unix seconds)
    pub last_notify: i64,
}

pub fn load_alert_state(conn: &Connection) -> Result<HashMap<(i64, i64), AlertStateRow>> {
    let mut stmt =
        conn.prepare("SELECT rule_id, agent_id, firing, since, last_notify FROM alert_state")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                AlertStateRow {
                    firing: r.get::<_, i64>(2)? != 0,
                    since: r.get(3)?,
                    last_notify: r.get(4)?,
                },
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub fn save_alert_state(
    conn: &Connection,
    rule_id: i64,
    agent_id: i64,
    s: &AlertStateRow,
) -> Result<()> {
    conn.execute(
        "INSERT INTO alert_state (rule_id, agent_id, firing, since, last_notify)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(rule_id, agent_id) DO UPDATE SET
           firing = excluded.firing, since = excluded.since, last_notify = excluded.last_notify",
        params![rule_id, agent_id, s.firing as i64, s.since, s.last_notify],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- notification channels

#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    /// "telegram" | "webhook" | "email" | "bark" | "feishu" | "dingtalk" | "wecom" | "discord"
    pub kind: String,
    /// Channel-specific config; secret-bearing fields are encrypted at rest.
    pub config: serde_json::Value,
    pub enabled: bool,
    /// Consecutive delivery failures; used to flag unhealthy channels.
    pub failed_streak: i32,
}

fn channel_from_row(r: &rusqlite::Row) -> rusqlite::Result<ChannelRow> {
    let config: String = r.get(3)?;
    Ok(ChannelRow {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        config: serde_json::from_str(&config).unwrap_or(serde_json::Value::Null),
        enabled: r.get::<_, i64>(4)? != 0,
        failed_streak: r.get::<_, i64>(5)? as i32,
    })
}

pub fn set_channel_streak(conn: &Connection, id: i64, streak: i32) -> Result<()> {
    conn.execute(
        "UPDATE notification_channels SET failed_streak = ?2 WHERE id = ?1",
        params![id, streak],
    )?;
    Ok(())
}

pub fn list_channels(conn: &Connection) -> Result<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, config, enabled, failed_streak FROM notification_channels ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], channel_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_channel(conn: &Connection, id: i64) -> Result<Option<ChannelRow>> {
    let row = conn
        .query_row(
            "SELECT id, name, kind, config, enabled, failed_streak FROM notification_channels WHERE id = ?1",
            params![id],
            channel_from_row,
        )
        .optional()?;
    Ok(row)
}

pub fn insert_channel(conn: &Connection, c: &ChannelRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO notification_channels (name, kind, config, enabled, created_at)
         VALUES (?1,?2,?3,?4,?5)",
        params![
            c.name,
            c.kind,
            serde_json::to_string(&c.config).unwrap_or_else(|_| "{}".into()),
            c.enabled as i64,
            now_ts(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_channel(conn: &Connection, id: i64, c: &ChannelRow) -> Result<usize> {
    let rows = conn.execute(
        "UPDATE notification_channels SET name = ?2, kind = ?3, config = ?4, enabled = ?5
         WHERE id = ?1",
        params![
            id,
            c.name,
            c.kind,
            serde_json::to_string(&c.config).unwrap_or_else(|_| "{}".into()),
            c.enabled as i64,
        ],
    )?;
    Ok(rows)
}

pub fn delete_channel(conn: &Connection, id: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM notification_channels WHERE id = ?1", params![id])?)
}

// ---------------------------------------------------------------- themes

#[derive(Debug, Clone)]
pub struct ThemeRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub preview: Option<String>,
    pub source: String,
    pub dir: String,
    pub installed_at: i64,
}

fn theme_from_row(r: &rusqlite::Row) -> rusqlite::Result<ThemeRow> {
    Ok(ThemeRow {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        author: r.get(3)?,
        description: r.get(4)?,
        preview: r.get(5)?,
        source: r.get(6)?,
        dir: r.get(7)?,
        installed_at: r.get(8)?,
    })
}

pub fn list_themes(conn: &Connection) -> Result<Vec<ThemeRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, version, author, description, preview, source, dir, installed_at
         FROM themes ORDER BY installed_at DESC",
    )?;
    let rows = stmt
        .query_map([], theme_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_theme(conn: &Connection, id: &str) -> Result<Option<ThemeRow>> {
    let row = conn
        .query_row(
            "SELECT id, name, version, author, description, preview, source, dir, installed_at
             FROM themes WHERE id = ?1",
            params![id],
            theme_from_row,
        )
        .optional()?;
    Ok(row)
}

pub fn upsert_theme(conn: &Connection, t: &ThemeRow) -> Result<()> {
    conn.execute(
        "INSERT INTO themes (id, name, version, author, description, preview, source, dir, installed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET
           name=excluded.name, version=excluded.version, author=excluded.author,
           description=excluded.description, preview=excluded.preview,
           source=excluded.source, dir=excluded.dir, installed_at=excluded.installed_at",
        params![
            t.id,
            t.name,
            t.version,
            t.author,
            t.description,
            t.preview,
            t.source,
            t.dir,
            t.installed_at,
        ],
    )?;
    Ok(())
}

pub fn delete_theme(conn: &Connection, id: &str) -> Result<usize> {
    Ok(conn.execute("DELETE FROM themes WHERE id = ?1", params![id])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(conn: &Connection, interval_sec: u64) -> i64 {
        insert_ping_task(
            conn,
            &PingTaskRow {
                id: 0,
                agent_id: Some(1),
                agent_ids: vec![1],
                label: "cf".into(),
                kind: "icmp".into(),
                target: "1.1.1.1".into(),
                port: None,
                interval_sec,
                probe_count: 3,
                enabled: true,
            },
        )
        .unwrap()
    }

    fn sample(task_id: i64, rtt: f64) -> PingResult {
        PingResult {
            label: "cf".into(),
            rtt_ms: Some(rtt),
            task_id: Some(task_id),
            rtt_min: Some(rtt),
            rtt_max: Some(rtt),
            loss: 0.0,
            status: None,
            cert_days: None,
            cert_name: None,
        }
    }

    fn stored(conn: &Connection, agent_id: i64) -> Vec<PingPoint> {
        ping_history(conn, agent_id, None, 0, 100).unwrap()
    }

    #[test]
    fn resent_snapshot_does_not_duplicate_history() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let id = task(&conn, 60);

        // The agent resends its whole result set whenever any task ticks.
        for _ in 0..5 {
            insert_ping_history(&conn, 1, &sample(id, 12.0)).unwrap();
        }
        assert_eq!(stored(&conn, 1).len(), 1);
    }

    #[test]
    fn a_genuinely_spaced_sample_is_kept() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let id = task(&conn, 60);

        insert_ping_history(&conn, 1, &sample(id, 12.0)).unwrap();
        conn.execute("UPDATE ping_history SET ts = ts - 60", []).unwrap();
        insert_ping_history(&conn, 1, &sample(id, 13.0)).unwrap();

        let rows = stored(&conn, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].rtt_avg, Some(13.0));
    }

    #[test]
    fn an_identical_down_sample_is_still_kept() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let id = task(&conn, 60);
        let down = PingResult {
            label: "cf".into(),
            rtt_ms: None,
            task_id: Some(id),
            rtt_min: None,
            rtt_max: None,
            loss: 1.0,
            status: None,
            cert_days: None,
            cert_name: None,
        };

        insert_ping_history(&conn, 1, &down).unwrap();
        conn.execute("UPDATE ping_history SET ts = ts - 60", []).unwrap();
        insert_ping_history(&conn, 1, &down).unwrap();
        assert_eq!(stored(&conn, 1).len(), 2);
    }

    #[test]
    fn ad_hoc_results_without_a_task_are_not_stored() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let mut legacy = sample(0, 9.0);
        legacy.task_id = None;
        insert_ping_history(&conn, 1, &legacy).unwrap();
        assert!(stored(&conn, 1).is_empty());
    }
}
