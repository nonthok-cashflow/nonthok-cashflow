use anyhow::{bail, Result};
use tracing::info;

use alpaca_client::{AlpacaRestClient, UnderlyingQuote};

/// Distilled account state relevant to the Options Wheel.
#[derive(Debug, Clone)]
pub struct AccountState {
    pub options_buying_power: f64,
    pub options_approved_level: u8,
}

/// Verify account is options-enabled and return buying power + approval level.
///
/// Returns `Err` if options trading is not approved (level 0 or field absent).
pub async fn check_account(client: &AlpacaRestClient) -> Result<AccountState> {
    let account = client.get_account().await?;

    let level: u8 = account.options_approved_level.unwrap_or(0);
    if level < 1 {
        bail!(
            "Options trading not approved on this account (level={}). \
             Enable options in Alpaca account settings.",
            level
        );
    }

    let options_buying_power: f64 = account
        .options_buying_power
        .as_deref()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0.0);

    info!(
        options_buying_power,
        options_approved_level = level,
        cash = %account.cash,
        equity = %account.equity,
        "Account health check passed"
    );

    Ok(AccountState {
        options_buying_power,
        options_approved_level: level,
    })
}

/// Fetch the latest bid/ask/mid for the underlying symbol.
pub async fn fetch_underlying_quote(
    client: &AlpacaRestClient,
    symbol: &str,
) -> Result<UnderlyingQuote> {
    let quote = client.get_stock_quote(symbol).await?;
    info!(
        symbol,
        bid = quote.bid,
        ask = quote.ask,
        mid = quote.mid,
        "Underlying quote fetched"
    );
    Ok(quote)
}
