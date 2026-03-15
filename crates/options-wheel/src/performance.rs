// performance.rs — Wheel cycle DB queries for --report (TNA-30)

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Path to the wheel performance database (~/.wheel_performance.db).
pub fn db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".wheel_performance.db")
}

/// Open (or create) the performance DB and initialize the wheel_cycles schema.
pub fn open_db() -> Result<Connection> {
    let path = db_path();
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open performance DB at {:?}", path))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wheel_cycles (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            underlying       TEXT    NOT NULL,
            cycle_started    TEXT,
            cycle_ended      TEXT,
            total_premium    REAL    NOT NULL DEFAULT 0,
            realized_pnl     REAL    NOT NULL DEFAULT 0,
            csp_outcome      TEXT,
            cycle_days       REAL    NOT NULL DEFAULT 0,
            capital_at_risk  REAL    NOT NULL DEFAULT 0
        );",
    )?;
    Ok(conn)
}

/// Per-symbol summary row.
#[derive(Debug)]
pub struct SymbolSummaryRow {
    pub underlying: String,
    pub cycles: i64,
    pub total_premium: f64,
    pub avg_pnl_per_cycle: f64,
    pub win_rate_pct: f64,
    pub avg_cycle_days: f64,
    pub annualized_yield_pct: f64,
}

/// Individual cycle row (for the "last 10 cycles" table).
#[derive(Debug)]
pub struct CycleRow {
    pub underlying: String,
    pub cycle_ended: String,
    pub total_premium: f64,
    pub realized_pnl: f64,
    pub csp_outcome: String,
    pub cycle_days: f64,
}

/// Query per-symbol summary for all completed cycles.
pub fn query_symbol_summary(conn: &Connection) -> Result<Vec<SymbolSummaryRow>> {
    let mut stmt = conn.prepare(
        "SELECT underlying,
                COUNT(*) AS cycles,
                ROUND(SUM(total_premium), 2) AS total_premium_collected,
                ROUND(AVG(realized_pnl), 2) AS avg_pnl_per_cycle,
                ROUND(100.0 * SUM(CASE WHEN csp_outcome != 'assigned' THEN 1 ELSE 0 END)
                      / COUNT(*), 1) AS win_rate_pct,
                ROUND(AVG(cycle_days), 1) AS avg_cycle_days,
                ROUND(SUM(realized_pnl)
                      / NULLIF(SUM(capital_at_risk * cycle_days / 365.0), 0) * 100.0,
                      1) AS annualized_yield_pct
         FROM wheel_cycles
         WHERE cycle_ended IS NOT NULL
         GROUP BY underlying
         ORDER BY underlying",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SymbolSummaryRow {
                underlying: row.get(0)?,
                cycles: row.get(1)?,
                total_premium: row.get(2)?,
                avg_pnl_per_cycle: row.get(3)?,
                win_rate_pct: row.get(4)?,
                avg_cycle_days: row.get(5)?,
                annualized_yield_pct: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Query the most recent N completed cycles.
pub fn query_recent_cycles(conn: &Connection, limit: usize) -> Result<Vec<CycleRow>> {
    let mut stmt = conn.prepare(
        "SELECT underlying, cycle_ended, total_premium, realized_pnl,
                COALESCE(csp_outcome, ''), cycle_days
         FROM wheel_cycles
         WHERE cycle_ended IS NOT NULL
         ORDER BY cycle_ended DESC
         LIMIT ?1",
    )?;

    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(CycleRow {
                underlying: row.get(0)?,
                cycle_ended: row.get(1)?,
                total_premium: row.get(2)?,
                realized_pnl: row.get(3)?,
                csp_outcome: row.get(4)?,
                cycle_days: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}
