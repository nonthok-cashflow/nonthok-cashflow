// iv_history.rs — IV/HV history capture and IVP gating (TNA-20)
//
// Phase 1 (< 252 history rows): HV30 bootstrap proxy gate — HV30 percentile >= 35%.
// Phase 2 (>= 252 history rows): True IVP gate — IVP >= 30%.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{params, Connection};
use tracing::info;

use alpaca_client::AlpacaRestClient;

// ─── DB Setup ────────────────────────────────────────────────────────────────

/// Path to the local SQLite IV history database.
fn db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".wheel_iv_history.db")
}

/// Open (or create) the iv_history SQLite database and initialize schema.
pub fn open_db() -> Result<Connection> {
    let path = db_path();
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open iv_history DB at {:?}", path))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS iv_history (
            symbol TEXT NOT NULL,
            date   TEXT NOT NULL,
            iv_30d REAL,
            hv_30d REAL,
            PRIMARY KEY (symbol, date)
        );",
    )?;
    Ok(conn)
}

// ─── HV30 Computation ────────────────────────────────────────────────────────

/// Compute annualized 30-day Historical Volatility (HV30) from daily closes.
///
/// Formula: stddev of 30 log returns * sqrt(252).
/// Returns `None` if fewer than 31 closes are provided.
pub fn compute_hv30(closes: &[f64]) -> Option<f64> {
    if closes.len() < 31 {
        return None;
    }
    let tail = &closes[closes.len().saturating_sub(31)..];
    let log_returns: Vec<f64> = tail.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
    let n = log_returns.len() as f64;
    let mean = log_returns.iter().sum::<f64>() / n;
    let variance = log_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(variance.sqrt() * 252.0_f64.sqrt())
}

// ─── Data Fetching ───────────────────────────────────────────────────────────

/// Fetch daily close prices for `symbol`, requesting enough bars to compute HV30.
async fn fetch_closes(symbol: &str, client: &AlpacaRestClient, bars_needed: usize) -> Result<Vec<f64>> {
    let end = chrono::Local::now().date_naive();
    // Request 2× calendar days to account for weekends and holidays
    let start = end - chrono::Duration::days((bars_needed as i64) * 2);

    let bars_json = client
        .get_bars(
            symbol,
            "1Day",
            Some(&start.format("%Y-%m-%d").to_string()),
            Some(&end.format("%Y-%m-%d").to_string()),
            Some((bars_needed + 20) as u32),
        )
        .await?;

    let closes: Vec<f64> = bars_json["bars"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|b| b["c"].as_f64())
        .collect();

    Ok(closes)
}

/// Fetch the ATM 30-DTE implied volatility for `symbol`.
///
/// Searches for puts with DTE in [25, 35] and returns the IV of the strike
/// closest to the current mid-price.
async fn fetch_atm_iv(symbol: &str, client: &AlpacaRestClient) -> Result<f64> {
    let today = chrono::Local::now().date_naive();
    let gte = (today + chrono::Duration::days(25)).format("%Y-%m-%d").to_string();
    let lte = (today + chrono::Duration::days(35)).format("%Y-%m-%d").to_string();

    let quote = client.get_stock_quote(symbol).await?;
    let mid_price = quote.mid;

    let snapshots = client
        .get_options_snapshots(symbol, "put", &gte, &lte, "indicative")
        .await?;

    // Find the strike closest to ATM with a valid IV
    let atm_iv = snapshots
        .snapshots
        .iter()
        .filter_map(|(sym, snap)| {
            let cp_pos = sym.rfind(|c| c == 'C' || c == 'P')?;
            let strike_str = &sym[cp_pos + 1..];
            if strike_str.len() != 8 {
                return None;
            }
            let strike_raw: u64 = strike_str.parse().ok()?;
            let strike = strike_raw as f64 / 1000.0;
            let iv = snap.implied_volatility.filter(|&v| v > 0.0)?;
            Some((strike, iv))
        })
        .min_by(|(a, _), (b, _)| {
            (a - mid_price)
                .abs()
                .partial_cmp(&(b - mid_price).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, iv)| iv)
        .ok_or_else(|| anyhow::anyhow!("No ATM IV found for {} in DTE [25,35]", symbol))?;

    info!(symbol, atm_iv = format!("{:.4}", atm_iv), "ATM 30-DTE IV captured");
    Ok(atm_iv)
}

// ─── Capture Functions ───────────────────────────────────────────────────────

/// Capture HV30 for `symbol` on `date` and upsert into iv_history.hv_30d.
///
/// Skips the API call if a value is already recorded for today.
pub async fn capture_hv30(
    conn: &Connection,
    symbol: &str,
    date: NaiveDate,
    client: &AlpacaRestClient,
) -> Result<f64> {
    // Skip if already recorded today
    let existing: Option<f64> = conn
        .query_row(
            "SELECT hv_30d FROM iv_history WHERE symbol = ?1 AND date = ?2 AND hv_30d IS NOT NULL",
            params![symbol, date.to_string()],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    if let Some(hv) = existing {
        info!(symbol, %date, hv30 = format!("{:.4}", hv), "HV30 already captured today, reusing");
        return Ok(hv);
    }

    let closes = fetch_closes(symbol, client, 35).await?;
    let hv30 = compute_hv30(&closes).context("Not enough bar data to compute HV30")?;

    conn.execute(
        "INSERT INTO iv_history (symbol, date, hv_30d)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(symbol, date) DO UPDATE SET hv_30d = excluded.hv_30d",
        params![symbol, date.to_string(), hv30],
    )?;

    info!(symbol, %date, hv30 = format!("{:.4}", hv30), "HV30 captured");
    Ok(hv30)
}

/// Capture ATM 30-DTE IV for `symbol` on `date` and upsert into iv_history.iv_30d.
///
/// Skips the API call if a value is already recorded for today.
pub async fn capture_iv30(
    conn: &Connection,
    symbol: &str,
    date: NaiveDate,
    client: &AlpacaRestClient,
) -> Result<f64> {
    // Skip if already recorded today
    let existing: Option<f64> = conn
        .query_row(
            "SELECT iv_30d FROM iv_history WHERE symbol = ?1 AND date = ?2 AND iv_30d IS NOT NULL",
            params![symbol, date.to_string()],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    if let Some(iv) = existing {
        info!(symbol, %date, iv30 = format!("{:.4}", iv), "IV30 already captured today, reusing");
        return Ok(iv);
    }

    let iv = fetch_atm_iv(symbol, client).await?;

    conn.execute(
        "INSERT INTO iv_history (symbol, date, iv_30d)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(symbol, date) DO UPDATE SET iv_30d = excluded.iv_30d",
        params![symbol, date.to_string(), iv],
    )?;

    info!(symbol, %date, iv30 = format!("{:.4}", iv), "IV30 captured");
    Ok(iv)
}

// ─── Gate Logic ──────────────────────────────────────────────────────────────

/// Count historical rows for `symbol` with dates strictly before `date`.
fn count_history_rows(conn: &Connection, symbol: &str, date: NaiveDate) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iv_history WHERE symbol = ?1 AND date < ?2",
        params![symbol, date.to_string()],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Bootstrap phase gate: HV30 percentile >= 35% passes.
fn hv30_percentile_gate(
    conn: &Connection,
    symbol: &str,
    date: NaiveDate,
    current_hv30: f64,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT hv_30d FROM iv_history
         WHERE symbol = ?1 AND date < ?2 AND hv_30d IS NOT NULL
         ORDER BY date DESC LIMIT 252",
    )?;
    let past_hvs: Vec<f64> = stmt
        .query_map(params![symbol, date.to_string()], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    if past_hvs.is_empty() {
        info!(symbol, "HV30 bootstrap: no history yet, allowing entry");
        return Ok(true);
    }

    let below_count = past_hvs.iter().filter(|&&hv| hv < current_hv30).count();
    let percentile = below_count as f64 / past_hvs.len() as f64;
    let pass = percentile >= 0.35;

    info!(
        symbol,
        hv30 = format!("{:.4}", current_hv30),
        percentile = format!("{:.2}%", percentile * 100.0),
        history_days = past_hvs.len(),
        pass,
        "HV30 bootstrap gate"
    );
    Ok(pass)
}

/// True IVP gate: % of past 252 days where iv_30d < current IV >= 30% passes.
fn ivp_gate(conn: &Connection, symbol: &str, date: NaiveDate, current_iv: f64) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT iv_30d FROM iv_history
         WHERE symbol = ?1 AND date < ?2 AND iv_30d IS NOT NULL
         ORDER BY date DESC LIMIT 252",
    )?;
    let past_ivs: Vec<f64> = stmt
        .query_map(params![symbol, date.to_string()], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    if past_ivs.is_empty() {
        info!(symbol, "True IVP: no IV history yet, allowing entry");
        return Ok(true);
    }

    let below_count = past_ivs.iter().filter(|&&iv| iv < current_iv).count();
    let ivp = below_count as f64 / past_ivs.len() as f64;
    let pass = ivp >= 0.30;

    info!(
        symbol,
        current_iv = format!("{:.4}", current_iv),
        ivp = format!("{:.2}%", ivp * 100.0),
        history_days = past_ivs.len(),
        pass,
        "True IVP gate"
    );
    Ok(pass)
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Capture today's IV/HV data then evaluate whether IV conditions favor a new CSP entry.
///
/// Phase 1 (< 252 history rows): HV30 bootstrap — HV30 percentile >= 35%.
/// Phase 2 (>= 252 history rows): True IVP — IVP >= 30%.
///
/// Also writes today's data to the iv_history table as a side-effect.
pub async fn capture_and_check_iv(
    conn: &Connection,
    symbol: &str,
    date: NaiveDate,
    client: &AlpacaRestClient,
) -> Result<bool> {
    let history_count = count_history_rows(conn, symbol, date)?;

    if history_count < 252 {
        // Phase 1: Bootstrap with HV30
        let hv30 = capture_hv30(conn, symbol, date, client).await?;
        let favorable = hv30_percentile_gate(conn, symbol, date, hv30)?;
        info!(
            symbol,
            history_days = history_count,
            phase = "bootstrap_hv30",
            favorable,
            "IV gate result"
        );
        Ok(favorable)
    } else {
        // Phase 2: True IVP; fall back to HV30 if ATM IV unavailable
        let iv = match capture_iv30(conn, symbol, date, client).await {
            Ok(iv) => iv,
            Err(e) => {
                tracing::warn!(
                    symbol,
                    error = %e,
                    "Failed to capture ATM IV30, falling back to HV30"
                );
                capture_hv30(conn, symbol, date, client).await?
            }
        };
        let favorable = ivp_gate(conn, symbol, date, iv)?;
        info!(
            symbol,
            history_days = history_count,
            phase = "true_ivp",
            favorable,
            "IV gate result"
        );
        Ok(favorable)
    }
}

/// Pure-read variant: check IV favorability from already-captured data.
///
/// Returns `true` (allow entry) when no data has been captured yet for today.
pub fn is_iv_favorable(conn: &Connection, symbol: &str, date: NaiveDate) -> Result<bool> {
    let history_count = count_history_rows(conn, symbol, date)?;

    if history_count < 252 {
        let hv30: Option<f64> = conn
            .query_row(
                "SELECT hv_30d FROM iv_history WHERE symbol = ?1 AND date = ?2",
                params![symbol, date.to_string()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        match hv30 {
            Some(hv) => hv30_percentile_gate(conn, symbol, date, hv),
            None => Ok(true),
        }
    } else {
        let iv: Option<f64> = conn
            .query_row(
                "SELECT iv_30d FROM iv_history WHERE symbol = ?1 AND date = ?2",
                params![symbol, date.to_string()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        match iv {
            Some(iv) => ivp_gate(conn, symbol, date, iv),
            None => Ok(true),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE iv_history (
                symbol TEXT NOT NULL,
                date   TEXT NOT NULL,
                iv_30d REAL,
                hv_30d REAL,
                PRIMARY KEY (symbol, date)
            );",
        )
        .unwrap();
        conn
    }

    // ── HV30 computation ─────────────────────────────────────────────────────

    #[test]
    fn test_compute_hv30_basic() {
        let closes: Vec<f64> = (0..32).map(|i| 100.0 * 1.01_f64.powi(i)).collect();
        let hv = compute_hv30(&closes);
        assert!(hv.is_some());
        assert!(hv.unwrap() > 0.0);
    }

    #[test]
    fn test_compute_hv30_insufficient_data() {
        assert!(compute_hv30(&[100.0, 101.0, 102.0]).is_none());
    }

    #[test]
    fn test_compute_hv30_volatile_gt_stable() {
        let stable: Vec<f64> = (0..32).map(|i| 100.0 + i as f64 * 0.01).collect();
        let volatile: Vec<f64> = (0..32).map(|i| if i % 2 == 0 { 100.0 } else { 110.0 }).collect();
        assert!(compute_hv30(&volatile).unwrap() > compute_hv30(&stable).unwrap());
    }

    // ── Bootstrap gate ───────────────────────────────────────────────────────

    #[test]
    fn test_is_iv_favorable_bootstrap_no_history_allows_entry() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
        assert!(is_iv_favorable(&conn, "BAC", date).unwrap());
    }

    #[test]
    fn test_is_iv_favorable_bootstrap_high_hv_passes() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();

        // Insert 100 history rows with hv_30d in [0.10, 0.30)
        for i in 0..100i64 {
            let past = date - chrono::Duration::days(i + 1);
            let hv = 0.10 + i as f64 * 0.002;
            conn.execute(
                "INSERT INTO iv_history (symbol, date, hv_30d) VALUES (?1, ?2, ?3)",
                params!["BAC", past.to_string(), hv],
            )
            .unwrap();
        }
        // Today's HV at 0.25 — above 75th percentile of [0.10, 0.30)
        conn.execute(
            "INSERT INTO iv_history (symbol, date, hv_30d) VALUES (?1, ?2, ?3)",
            params!["BAC", date.to_string(), 0.25_f64],
        )
        .unwrap();

        assert!(is_iv_favorable(&conn, "BAC", date).unwrap());
    }

    #[test]
    fn test_is_iv_favorable_bootstrap_low_hv_fails() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();

        // Insert 100 history rows with high hv_30d [0.30, 0.50)
        for i in 0..100i64 {
            let past = date - chrono::Duration::days(i + 1);
            let hv = 0.30 + i as f64 * 0.002;
            conn.execute(
                "INSERT INTO iv_history (symbol, date, hv_30d) VALUES (?1, ?2, ?3)",
                params!["BAC", past.to_string(), hv],
            )
            .unwrap();
        }
        // Today's HV very low — at 1st percentile of history
        conn.execute(
            "INSERT INTO iv_history (symbol, date, hv_30d) VALUES (?1, ?2, ?3)",
            params!["BAC", date.to_string(), 0.31_f64],
        )
        .unwrap();

        assert!(!is_iv_favorable(&conn, "BAC", date).unwrap());
    }

    // ── True IVP gate ────────────────────────────────────────────────────────

    #[test]
    fn test_is_iv_favorable_ivp_high_passes() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();

        // 252 history rows with iv_30d in [0.15, 0.40)
        for i in 0..252i64 {
            let past = date - chrono::Duration::days(i + 1);
            let iv = 0.15 + i as f64 * (0.25 / 252.0);
            conn.execute(
                "INSERT INTO iv_history (symbol, date, iv_30d) VALUES (?1, ?2, ?3)",
                params!["BAC", past.to_string(), iv],
            )
            .unwrap();
        }
        // Today's IV at ~80th percentile
        conn.execute(
            "INSERT INTO iv_history (symbol, date, iv_30d) VALUES (?1, ?2, ?3)",
            params!["BAC", date.to_string(), 0.35_f64],
        )
        .unwrap();

        assert!(is_iv_favorable(&conn, "BAC", date).unwrap());
    }

    #[test]
    fn test_is_iv_favorable_ivp_low_fails() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();

        // 252 history rows with high iv_30d [0.35, 0.60)
        for i in 0..252i64 {
            let past = date - chrono::Duration::days(i + 1);
            let iv = 0.35 + i as f64 * (0.25 / 252.0);
            conn.execute(
                "INSERT INTO iv_history (symbol, date, iv_30d) VALUES (?1, ?2, ?3)",
                params!["BAC", past.to_string(), iv],
            )
            .unwrap();
        }
        // Today's IV very low relative to history
        conn.execute(
            "INSERT INTO iv_history (symbol, date, iv_30d) VALUES (?1, ?2, ?3)",
            params!["BAC", date.to_string(), 0.20_f64],
        )
        .unwrap();

        assert!(!is_iv_favorable(&conn, "BAC", date).unwrap());
    }

    #[test]
    fn test_ivp_no_history_allows_entry() {
        let conn = make_test_db();
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();

        // Insert 252 rows so we're in Phase 2, but no iv_30d for today
        for i in 0..252i64 {
            let past = date - chrono::Duration::days(i + 1);
            conn.execute(
                "INSERT INTO iv_history (symbol, date, iv_30d) VALUES (?1, ?2, ?3)",
                params!["BAC", past.to_string(), 0.25_f64],
            )
            .unwrap();
        }

        // No row for today → is_iv_favorable should allow entry
        assert!(is_iv_favorable(&conn, "BAC", date).unwrap());
    }
}
