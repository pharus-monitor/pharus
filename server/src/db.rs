use anyhow::Result;
use pharus_common::Metrics;
use rusqlite::{params, Connection, OptionalExtension};

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
        ",
    )?;
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
