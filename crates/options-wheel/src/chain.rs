// chain.rs — Options chain fetcher + strike selector (TNA-12)

use anyhow::{bail, Result};
use chrono::{Duration, Local, NaiveDate};
use tracing::info;

use alpaca_client::AlpacaRestClient;

/// BAC earnings dates to avoid (hardcoded for ~2026).
/// Skip contracts whose expiration falls on or after an earnings date
/// and within 7 days after it.
const BAC_EARNINGS_DATES: &[&str] = &[
    "2026-01-14", // Q4 2025 earnings (approx)
    "2026-04-15", // Q1 2026 earnings (approx)
    "2026-07-15", // Q2 2026 earnings (approx)
    "2026-10-14", // Q3 2026 earnings (approx)
];

/// A single option contract from the chain snapshot.
#[derive(Debug, Clone)]
pub struct OptionContract {
    pub symbol: String,
    pub expiration_date: NaiveDate,
    pub strike_price: f64,
    pub contract_type: String, // "put" or "call"
    pub delta: f64,
    pub implied_volatility: f64,
    pub bid: f64,
    pub ask: f64,
    pub mid: f64,
    pub open_interest: u64,
}

/// Parse expiration date and strike price from an OCC option symbol.
///
/// OCC format: `{underlying}{YYMMDD}{C/P}{strike*1000 zero-padded to 8 digits}`
/// Example: `BAC260404P00038000` → 2026-04-04, put, $38.00
fn parse_occ_symbol(symbol: &str) -> Option<(NaiveDate, String, f64)> {
    // Find the contract type character (C or P)
    let cp_pos = symbol.rfind(|c| c == 'C' || c == 'P')?;

    // Everything before cp_pos ending in YYMMDD (6 chars)
    let prefix = &symbol[..cp_pos];
    if prefix.len() < 6 {
        return None;
    }
    let date_str = &prefix[prefix.len() - 6..];
    let year: i32 = format!("20{}", &date_str[..2]).parse().ok()?;
    let month: u32 = date_str[2..4].parse().ok()?;
    let day: u32 = date_str[4..6].parse().ok()?;
    let expiration = NaiveDate::from_ymd_opt(year, month, day)?;

    let contract_type = if &symbol[cp_pos..cp_pos + 1] == "C" {
        "call".to_string()
    } else {
        "put".to_string()
    };

    // Strike: the 8 digits after C/P, representing strike * 1000
    let strike_str = &symbol[cp_pos + 1..];
    if strike_str.len() != 8 {
        return None;
    }
    let strike_raw: u64 = strike_str.parse().ok()?;
    let strike = strike_raw as f64 / 1000.0;

    Some((expiration, contract_type, strike))
}

/// Returns true if the expiration date is within an earnings window.
///
/// We skip any expiration within [-2, +7] days of a known earnings date
/// to avoid IV crush and unexpected assignment risk.
fn within_earnings_window(expiration: NaiveDate) -> bool {
    for &date_str in BAC_EARNINGS_DATES {
        if let Ok(earnings_date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            let days_diff = (expiration - earnings_date).num_days();
            if (-2..=7).contains(&days_diff) {
                return true;
            }
        }
    }
    false
}

/// Fetch options chain for `underlying` within the DTE window (generic over contract type).
async fn fetch_chain(
    underlying: &str,
    client: &AlpacaRestClient,
    contract_type: &str,
    dte_min: u32,
    dte_max: u32,
) -> Result<Vec<OptionContract>> {
    let today = Local::now().date_naive();
    let gte = (today + Duration::days(dte_min as i64))
        .format("%Y-%m-%d")
        .to_string();
    let lte = (today + Duration::days(dte_max as i64))
        .format("%Y-%m-%d")
        .to_string();

    info!(
        underlying,
        contract_type,
        expiration_date_gte = %gte,
        expiration_date_lte = %lte,
        "Fetching options chain"
    );

    let snapshots = client
        .get_options_snapshots(underlying, contract_type, &gte, &lte, "indicative")
        .await?;

    let mut contracts: Vec<OptionContract> = Vec::new();

    for (symbol, snap) in &snapshots.snapshots {
        let (expiration, ct, strike) = match parse_occ_symbol(symbol) {
            Some(v) => v,
            None => {
                tracing::warn!(symbol, "Failed to parse OCC symbol, skipping");
                continue;
            }
        };

        let delta = snap.greeks.as_ref().and_then(|g| g.delta).unwrap_or(0.0);
        let iv = snap.implied_volatility.unwrap_or(0.0);
        let bid = snap.latest_quote.as_ref().and_then(|q| q.bp).unwrap_or(0.0);
        let ask = snap.latest_quote.as_ref().and_then(|q| q.ap).unwrap_or(0.0);
        let mid = (bid + ask) / 2.0;
        let open_interest = snap.open_interest.unwrap_or(0.0) as u64;

        contracts.push(OptionContract {
            symbol: symbol.clone(),
            expiration_date: expiration,
            strike_price: strike,
            contract_type: ct,
            delta,
            implied_volatility: iv,
            bid,
            ask,
            mid,
            open_interest,
        });
    }

    info!(count = contracts.len(), "Raw contracts fetched");
    Ok(contracts)
}

/// Fetch the put chain for `underlying` within the DTE window.
pub async fn fetch_put_chain(
    underlying: &str,
    client: &AlpacaRestClient,
    dte_min: u32,
    dte_max: u32,
) -> Result<Vec<OptionContract>> {
    fetch_chain(underlying, client, "put", dte_min, dte_max).await
}

/// Fetch the call chain for `underlying` within the DTE window.
pub async fn fetch_call_chain(
    underlying: &str,
    client: &AlpacaRestClient,
    dte_min: u32,
    dte_max: u32,
) -> Result<Vec<OptionContract>> {
    fetch_chain(underlying, client, "call", dte_min, dte_max).await
}

/// Select the optimal CC strike from a call chain.
///
/// Same filter logic as CSP: delta 0.20–0.25 (abs), OI >= 200, spread <= 0.10.
/// Selects the call with the highest mid-price (maximizes premium).
pub fn select_cc_strike(chain: &[OptionContract]) -> Result<&OptionContract> {
    let candidates: Vec<&OptionContract> = chain
        .iter()
        .filter(|c| {
            let abs_delta = c.delta.abs();
            let spread = c.ask - c.bid;
            abs_delta >= 0.20
                && abs_delta <= 0.25
                && c.open_interest >= 200
                && spread <= 0.10
                && !within_earnings_window(c.expiration_date)
        })
        .collect();

    if candidates.is_empty() {
        bail!(
            "No CC candidates pass all filters. \
             Consider widening parameters or checking the call chain."
        );
    }

    let best = candidates
        .into_iter()
        .max_by(|a, b| a.mid.partial_cmp(&b.mid).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    info!(
        symbol = %best.symbol,
        delta = best.delta,
        mid = best.mid,
        expiration = %best.expiration_date,
        "Selected CC strike"
    );

    Ok(best)
}

/// Select the optimal CSP strike from the chain.
///
/// Filters applied in order:
/// 1. Delta between -0.25 and -0.20 (abs value 0.20–0.25)
/// 2. open_interest >= 200
/// 3. (ask - bid) <= 0.10 (spread tightness)
/// 4. NOT within earnings window
///
/// Among passing contracts, selects the one with the highest mid-price
/// (maximises premium collected).
pub fn select_csp_strike(chain: &[OptionContract]) -> Result<&OptionContract> {
    let candidates: Vec<&OptionContract> = chain
        .iter()
        .filter(|c| {
            let abs_delta = c.delta.abs();
            let spread = c.ask - c.bid;
            abs_delta >= 0.20
                && abs_delta <= 0.25
                && c.open_interest >= 200
                && spread <= 0.10
                && !within_earnings_window(c.expiration_date)
        })
        .collect();

    if candidates.is_empty() {
        bail!(
            "No CSP candidates pass all filters (delta 0.20-0.25, OI>=200, spread<=0.10, no earnings). \
             Consider widening parameters or checking the options chain."
        );
    }

    let best = candidates
        .into_iter()
        .max_by(|a, b| a.mid.partial_cmp(&b.mid).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap(); // safe: candidates is non-empty

    info!(
        symbol = %best.symbol,
        delta = best.delta,
        mid = best.mid,
        expiration = %best.expiration_date,
        oi = best.open_interest,
        iv = best.implied_volatility,
        "Selected CSP strike"
    );

    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_contract(delta: f64, bid: f64, ask: f64, oi: u64, expiration: &str) -> OptionContract {
        let expiration_date = NaiveDate::parse_from_str(expiration, "%Y-%m-%d").unwrap();
        let mid = (bid + ask) / 2.0;
        OptionContract {
            symbol: format!("BAC{}", &expiration.replace('-', "")[2..]),
            expiration_date,
            strike_price: 38.0,
            contract_type: "put".to_string(),
            delta,
            implied_volatility: 0.25,
            bid,
            ask,
            mid,
            open_interest: oi,
        }
    }

    #[test]
    fn test_select_csp_strike_picks_highest_mid() {
        let chain = vec![
            make_contract(-0.22, 0.60, 0.68, 300, "2026-05-01"), // passes, mid=0.64
            make_contract(-0.23, 0.70, 0.78, 500, "2026-05-01"), // passes, mid=0.74 ← winner
            make_contract(-0.22, 0.50, 0.58, 300, "2026-05-01"), // passes, mid=0.54
        ];
        let selected = select_csp_strike(&chain).unwrap();
        assert_eq!(selected.mid, 0.74);
    }

    #[test]
    fn test_delta_filter() {
        let chain = vec![
            make_contract(-0.10, 0.30, 0.38, 300, "2026-05-01"), // delta too low
            make_contract(-0.30, 0.80, 0.88, 300, "2026-05-01"), // delta too high
            make_contract(-0.22, 0.60, 0.68, 300, "2026-05-01"), // passes
        ];
        let selected = select_csp_strike(&chain).unwrap();
        assert!((selected.delta.abs() - 0.22).abs() < 1e-9);
    }

    #[test]
    fn test_oi_filter() {
        let chain = vec![
            make_contract(-0.22, 0.60, 0.68, 100, "2026-05-01"), // OI too low
            make_contract(-0.22, 0.50, 0.58, 300, "2026-05-01"), // passes
        ];
        let selected = select_csp_strike(&chain).unwrap();
        assert_eq!(selected.open_interest, 300);
    }

    #[test]
    fn test_spread_filter() {
        let chain = vec![
            make_contract(-0.22, 0.40, 0.52, 300, "2026-05-01"), // spread=0.12 → filtered
            make_contract(-0.22, 0.60, 0.68, 300, "2026-05-01"), // spread=0.08 → passes
        ];
        let selected = select_csp_strike(&chain).unwrap();
        assert!((selected.ask - selected.bid - 0.08).abs() < 1e-9);
    }

    #[test]
    fn test_empty_chain_returns_err() {
        let chain: Vec<OptionContract> = vec![];
        assert!(select_csp_strike(&chain).is_err());
    }

    #[test]
    fn test_no_candidates_returns_err() {
        // All have delta out of range
        let chain = vec![make_contract(-0.05, 0.10, 0.18, 300, "2026-05-01")];
        assert!(select_csp_strike(&chain).is_err());
    }

    #[test]
    fn test_parse_occ_symbol() {
        let (exp, contract_type, strike) = parse_occ_symbol("BAC260404P00038000").unwrap();
        assert_eq!(exp, NaiveDate::from_ymd_opt(2026, 4, 4).unwrap());
        assert_eq!(contract_type, "put");
        assert!((strike - 38.0).abs() < 1e-9);
    }

    #[test]
    fn test_parse_occ_symbol_call() {
        let (exp, contract_type, strike) = parse_occ_symbol("BAC260515C00042000").unwrap();
        assert_eq!(exp, NaiveDate::from_ymd_opt(2026, 5, 15).unwrap());
        assert_eq!(contract_type, "call");
        assert!((strike - 42.0).abs() < 1e-9);
    }
}
