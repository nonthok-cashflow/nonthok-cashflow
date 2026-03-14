// orders.rs — CSP/CC order placement + order status monitor (TNA-13)

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use alpaca_client::{AlpacaRestClient, OrderRequest, OrderSide, OrderStatus, OrderType, TimeInForce};

/// Result of a placed and filled option order.
#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: String,
    pub filled_price: f64,
    pub filled_at: DateTime<Utc>,
}

/// Poll an order until filled, cancelled, or timeout.
///
/// Polls every 10 seconds for up to `timeout_secs` total.
async fn wait_for_fill(
    client: &AlpacaRestClient,
    order_id: &str,
    timeout_secs: u64,
) -> Result<OrderResult> {
    let poll_secs = 10u64;
    let attempts = (timeout_secs / poll_secs).max(1);

    for attempt in 0..attempts {
        tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
        let order = client.get_order(order_id).await?;

        info!(
            order_id,
            attempt,
            status = ?order.status,
            "Polling order status"
        );

        match order.status {
            OrderStatus::Filled => {
                let filled_price = order
                    .filled_avg_price
                    .as_deref()
                    .and_then(|p| p.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let filled_at = order.filled_at.unwrap_or_else(Utc::now);
                info!(
                    order_id,
                    filled_price,
                    "[ORDER] filled @ ${:.2}", filled_price
                );
                return Ok(OrderResult {
                    order_id: order.id,
                    filled_price,
                    filled_at,
                });
            }
            OrderStatus::Canceled
            | OrderStatus::Expired
            | OrderStatus::Rejected => {
                bail!("Order {} ended with terminal status {:?}", order_id, order.status);
            }
            _ => {
                // Still open — keep polling
            }
        }
    }

    Err(anyhow::anyhow!(
        "Order {} not filled after {}s",
        order_id,
        timeout_secs
    ))
}

/// Place a single-contract option order (limit, then market fallback).
///
/// Tries the limit price first; if not filled within 60s, cancels and
/// retries with a market order.
async fn place_option_order_with_retry(
    client: &AlpacaRestClient,
    symbol: &str,
    side: OrderSide,
    limit_price: f64,
) -> Result<OrderResult> {
    let limit_req = OrderRequest {
        symbol: symbol.to_string(),
        qty: Some("1".to_string()),
        notional: None,
        side: side.clone(),
        order_type: OrderType::Limit,
        time_in_force: TimeInForce::Day,
        limit_price: Some(format!("{:.2}", limit_price)),
        stop_price: None,
        extended_hours: None,
        client_order_id: None,
    };

    let order = client.place_order(&limit_req).await?;
    info!(
        order_id = %order.id,
        symbol,
        side = ?side,
        limit_price,
        "[ORDER] {} {:?} {} x1 @ ${:.2} LIMIT placed",
        if matches!(side, OrderSide::Sell) { "SELL" } else { "BUY" },
        side, symbol, limit_price
    );

    match wait_for_fill(client, &order.id, 60).await {
        Ok(result) => return Ok(result),
        Err(e) => {
            warn!("Limit order timed out ({e}), cancelling and retrying at market");
            let _ = client.cancel_order(&order.id).await;
        }
    }

    // Retry at market
    let market_req = OrderRequest {
        symbol: symbol.to_string(),
        qty: Some("1".to_string()),
        notional: None,
        side: side.clone(),
        order_type: OrderType::Market,
        time_in_force: TimeInForce::Day,
        limit_price: None,
        stop_price: None,
        extended_hours: None,
        client_order_id: None,
    };

    let market_order = client.place_order(&market_req).await?;
    info!(
        order_id = %market_order.id,
        symbol,
        "[ORDER] {} {} x1 MARKET placed (retry after limit timeout)",
        if matches!(side, OrderSide::Sell) { "SELL" } else { "BUY" },
        symbol
    );

    wait_for_fill(client, &market_order.id, 60).await
}

/// Place a cash-secured put (CSP) order and wait for fill.
///
/// Pre-flight: verifies `options_buying_power >= strike * 100`.
pub async fn place_csp_order(
    client: &AlpacaRestClient,
    symbol: &str,
    strike: f64,
    limit_price: f64,
) -> Result<OrderResult> {
    // Pre-flight: check options buying power
    let account = client.get_account().await?;
    let options_bp: f64 = account
        .options_buying_power
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let required_bp = strike * 100.0;

    if options_bp < required_bp {
        bail!(
            "Insufficient options buying power: ${:.2} available, ${:.2} required for {} CSP",
            options_bp,
            required_bp,
            symbol
        );
    }

    info!(
        symbol,
        strike,
        limit_price,
        options_bp,
        "[ORDER] SELL PUT {} x1 @ ${:.2} LIMIT (pre-flight passed)",
        symbol, limit_price
    );

    place_option_order_with_retry(client, symbol, OrderSide::Sell, limit_price).await
}

/// Place a covered call (CC) order after assignment.
///
/// Pre-flight: verifies BAC equity position has qty >= 100.
pub async fn place_cc_order(
    client: &AlpacaRestClient,
    underlying: &str,
    symbol: &str,
    limit_price: f64,
) -> Result<OrderResult> {
    // Pre-flight: check equity position
    let position = client.get_position(underlying).await.map_err(|e| {
        anyhow::anyhow!(
            "Pre-flight failed: cannot find {} equity position ({}). \
             Ensure assignment has been processed.",
            underlying, e
        )
    })?;

    let qty: i64 = position.qty.parse().unwrap_or(0);
    if qty < 100 {
        bail!(
            "Pre-flight failed: only {} shares of {} held, need >= 100 for covered call",
            qty, underlying
        );
    }

    info!(
        underlying,
        symbol,
        limit_price,
        qty,
        "[ORDER] SELL CALL {} x1 @ ${:.2} LIMIT (pre-flight passed, {} shares held)",
        symbol, limit_price, qty
    );

    place_option_order_with_retry(client, symbol, OrderSide::Sell, limit_price).await
}

/// Place a buy-to-close order for profit taking.
pub async fn place_buy_to_close(
    client: &AlpacaRestClient,
    symbol: &str,
    close_price: f64,
) -> Result<OrderResult> {
    info!(
        symbol,
        close_price,
        "[ORDER] BUY {} x1 @ ${:.2} LIMIT (buy-to-close)",
        symbol, close_price
    );
    place_option_order_with_retry(client, symbol, OrderSide::Buy, close_price).await
}

#[cfg(test)]
mod tests {
    /// Unit tests for pre-flight logic are integration-level (need API).
    /// We test helper logic separately.
    #[test]
    fn test_required_buying_power() {
        let strike = 38.0_f64;
        let required = strike * 100.0;
        assert!((required - 3800.0).abs() < 1e-9);
    }

    #[test]
    fn test_limit_price_format() {
        let price = 0.725_f64;
        let formatted = format!("{:.2}", price);
        assert_eq!(formatted, "0.73");
    }
}
