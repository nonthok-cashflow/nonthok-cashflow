// chain.rs — Options chain fetcher + strike selector
// Implemented in TNA-12.

use anyhow::Result;
use chrono::NaiveDate;

/// A single option contract from the chain snapshot.
#[allow(dead_code)]
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

/// Fetch the put chain for `underlying` within the DTE window.
/// TODO: implement in TNA-12
#[allow(dead_code)]
pub async fn fetch_put_chain(
    _underlying: &str,
    _api_key: &str,
    _api_secret: &str,
    _data_url: &str,
    _dte_min: u32,
    _dte_max: u32,
) -> Result<Vec<OptionContract>> {
    unimplemented!("fetch_put_chain: implement in TNA-12")
}

/// Select the optimal CSP strike from the chain.
/// TODO: implement in TNA-12
#[allow(dead_code)]
pub fn select_csp_strike(chain: &[OptionContract]) -> Result<&OptionContract> {
    chain
        .iter()
        .max_by(|a, b| a.mid.partial_cmp(&b.mid).unwrap())
        .ok_or_else(|| anyhow::anyhow!("Empty options chain"))
}
