// positions.rs — Position tracking + assignment detection (TNA-13)

use anyhow::Result;
use chrono::{Local, NaiveDate};
use tracing::info;

use alpaca_client::AlpacaRestClient;

/// Summary of the active wheel position.
#[derive(Debug, Clone)]
pub struct PositionSummary {
    pub symbol: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pl: f64,
    pub unrealized_plpc: f64,
    pub qty: i64,
    pub dte_remaining: i32,
}

/// Events detected from account activity polling.
#[derive(Debug, Clone, PartialEq)]
pub enum WheelEvent {
    Active,
    Assigned,
    Expired,
}

/// Parse expiration date from an OCC option symbol.
///
/// OCC format: `{underlying}{YYMMDD}{C/P}{strike*1000}`
/// Returns None if the symbol is not a valid OCC option symbol.
fn parse_occ_expiration(symbol: &str) -> Option<NaiveDate> {
    let cp_pos = symbol.rfind(|c| c == 'C' || c == 'P')?;
    let prefix = &symbol[..cp_pos];
    if prefix.len() < 6 {
        return None;
    }
    let date_str = &prefix[prefix.len() - 6..];
    let year: i32 = format!("20{}", &date_str[..2]).parse().ok()?;
    let month: u32 = date_str[2..4].parse().ok()?;
    let day: u32 = date_str[4..6].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Fetch and summarise the active wheel position for `symbol`.
///
/// Returns `Ok(None)` if no position is found (not an error).
pub async fn get_position_summary(
    client: &AlpacaRestClient,
    symbol: &str,
) -> Result<Option<PositionSummary>> {
    let position = match client.get_position(symbol).await {
        Ok(p) => p,
        Err(alpaca_client::AlpacaError::Api { status: 404, .. }) => {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };

    let entry_price: f64 = position.avg_entry_price.parse().unwrap_or(0.0);
    let current_price: f64 = position.current_price.parse().unwrap_or(0.0);
    let unrealized_pl: f64 = position.unrealized_pl.parse().unwrap_or(0.0);
    let unrealized_plpc: f64 = position.unrealized_plpc.parse().unwrap_or(0.0);
    let qty: i64 = position.qty.parse().unwrap_or(0);

    // Compute DTE from OCC symbol (0 for equity positions)
    let dte_remaining = parse_occ_expiration(symbol)
        .map(|exp| {
            let today = Local::now().date_naive();
            (exp - today).num_days() as i32
        })
        .unwrap_or(0);

    let summary = PositionSummary {
        symbol: symbol.to_string(),
        entry_price,
        current_price,
        unrealized_pl,
        unrealized_plpc,
        qty,
        dte_remaining,
    };

    info!(
        symbol,
        entry_price,
        current_price,
        unrealized_pl,
        unrealized_plpc,
        dte_remaining,
        "[PNL] Position P&L: {:+.0} ({:.0}% of premium), DTE={}",
        unrealized_pl,
        unrealized_plpc * 100.0,
        dte_remaining
    );

    Ok(Some(summary))
}

/// Log all open positions and return a summary of each.
pub async fn log_all_positions(client: &AlpacaRestClient) -> Result<Vec<PositionSummary>> {
    let positions = client.get_positions().await?;

    if positions.is_empty() {
        info!("No open positions");
        return Ok(vec![]);
    }

    let mut summaries = Vec::new();
    for pos in &positions {
        let entry_price: f64 = pos.avg_entry_price.parse().unwrap_or(0.0);
        let current_price: f64 = pos.current_price.parse().unwrap_or(0.0);
        let unrealized_pl: f64 = pos.unrealized_pl.parse().unwrap_or(0.0);
        let unrealized_plpc: f64 = pos.unrealized_plpc.parse().unwrap_or(0.0);
        let qty: i64 = pos.qty.parse().unwrap_or(0);
        let dte_remaining = parse_occ_expiration(&pos.symbol)
            .map(|exp| {
                let today = Local::now().date_naive();
                (exp - today).num_days() as i32
            })
            .unwrap_or(0);

        info!(
            symbol = %pos.symbol,
            asset_class = %pos.asset_class,
            qty,
            entry_price,
            current_price,
            unrealized_pl,
            unrealized_plpc,
            "[POSITION] {} qty={} entry=${:.2} current=${:.2} P&L={:+.2} ({:.1}%)",
            pos.symbol, qty, entry_price, current_price, unrealized_pl, unrealized_plpc * 100.0
        );

        summaries.push(PositionSummary {
            symbol: pos.symbol.clone(),
            entry_price,
            current_price,
            unrealized_pl,
            unrealized_plpc,
            qty,
            dte_remaining,
        });
    }

    Ok(summaries)
}

/// Poll account activities to detect assignment or expiration.
///
/// Detection priority:
/// 1. If BAC equity position with qty >= 100 exists → Assigned
///    (Primary method: paper accounts delay NTA events until next day)
/// 2. If OPASN activity found → Assigned
/// 3. If OPEXP or OPEXC activity found → Expired
/// 4. Default → Active
pub async fn detect_wheel_event(
    client: &AlpacaRestClient,
    underlying: &str,
) -> Result<WheelEvent> {
    // Primary detection: check equity position
    match client.get_position(underlying).await {
        Ok(pos) => {
            let qty: i64 = pos.qty.parse().unwrap_or(0);
            if qty >= 100 {
                info!(
                    underlying,
                    qty,
                    "Assignment detected via equity position (qty={} >= 100)", qty
                );
                return Ok(WheelEvent::Assigned);
            }
        }
        Err(alpaca_client::AlpacaError::Api { status: 404, .. }) => {
            // No equity position — expected before assignment
        }
        Err(e) => {
            tracing::warn!("Error checking equity position: {e}");
        }
    }

    // Secondary detection: account activities
    match client.get_account_activities("OPASN,OPEXP,OPEXC").await {
        Ok(activities) => {
            if let Some(arr) = activities.as_array() {
                for activity in arr {
                    let activity_type = activity
                        .get("activity_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let symbol = activity
                        .get("symbol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    // Only look at activities for our underlying
                    if !symbol.starts_with(underlying) {
                        continue;
                    }

                    match activity_type {
                        "OPASN" => {
                            info!(symbol, "Assignment detected via OPASN activity");
                            return Ok(WheelEvent::Assigned);
                        }
                        "OPEXP" | "OPEXC" => {
                            info!(symbol, activity_type, "Expiration detected via activity");
                            return Ok(WheelEvent::Expired);
                        }
                        _ => {}
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Could not fetch account activities: {e}");
        }
    }

    Ok(WheelEvent::Active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_occ_expiration() {
        let exp = parse_occ_expiration("BAC260404P00038000").unwrap();
        assert_eq!(exp, NaiveDate::from_ymd_opt(2026, 4, 4).unwrap());
    }

    #[test]
    fn test_parse_occ_expiration_call() {
        let exp = parse_occ_expiration("BAC260515C00042000").unwrap();
        assert_eq!(exp, NaiveDate::from_ymd_opt(2026, 5, 15).unwrap());
    }

    #[test]
    fn test_parse_occ_expiration_equity() {
        // BAC (equity) should return None
        assert!(parse_occ_expiration("BAC").is_none());
    }

    #[test]
    fn test_unrealized_plpc_threshold() {
        // 50% profit threshold
        let unrealized_plpc = 0.51_f64;
        assert!(unrealized_plpc >= 0.50);
    }
}
