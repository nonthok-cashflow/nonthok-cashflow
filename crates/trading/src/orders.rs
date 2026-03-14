use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::{info, instrument, warn};

use alpaca_client::{
    AlpacaRestClient, Order, OrderRequest, OrderSide, OrderType, TimeInForce,
};

/// A signal produced by a strategy that the executor should act on.
#[derive(Debug, Clone)]
pub struct Signal {
    pub symbol: String,
    pub side: OrderSide,
    /// Notional USD value to trade (use this OR qty, not both)
    pub notional_usd: Option<f64>,
    /// Number of shares/contracts (use this OR notional, not both)
    pub qty: Option<f64>,
    pub order_type: OrderType,
    pub limit_price: Option<f64>,
    pub time_in_force: TimeInForce,
    /// Human-readable reason for this signal (logged, not sent to broker)
    pub reason: String,
}

impl Signal {
    /// Convenience constructor for a market buy by notional.
    pub fn market_buy(symbol: impl Into<String>, notional_usd: f64, reason: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            side: OrderSide::Buy,
            notional_usd: Some(notional_usd),
            qty: None,
            order_type: OrderType::Market,
            limit_price: None,
            time_in_force: TimeInForce::Day,
            reason: reason.into(),
        }
    }

    /// Convenience constructor for a market sell by quantity.
    pub fn market_sell(symbol: impl Into<String>, qty: f64, reason: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            side: OrderSide::Sell,
            notional_usd: None,
            qty: Some(qty),
            order_type: OrderType::Market,
            limit_price: None,
            time_in_force: TimeInForce::Day,
            reason: reason.into(),
        }
    }
}

/// Converts a `Signal` into an Alpaca `OrderRequest`.
impl TryFrom<Signal> for OrderRequest {
    type Error = anyhow::Error;

    fn try_from(signal: Signal) -> Result<Self> {
        if signal.notional_usd.is_none() && signal.qty.is_none() {
            anyhow::bail!("Signal must specify either notional_usd or qty");
        }

        Ok(OrderRequest {
            symbol: signal.symbol,
            qty: signal.qty.map(|q| format!("{:.4}", q)),
            notional: signal.notional_usd.map(|n| format!("{:.2}", n)),
            side: signal.side,
            order_type: signal.order_type,
            time_in_force: signal.time_in_force,
            limit_price: signal.limit_price.map(|p| format!("{:.2}", p)),
            stop_price: None,
            extended_hours: None,
            client_order_id: None,
        })
    }
}

/// Abstraction over order execution so strategies can be tested without live API calls.
#[async_trait]
pub trait OrderExecutor: Send + Sync {
    async fn execute(&self, signal: Signal) -> Result<Order>;
    async fn cancel(&self, order_id: &str) -> Result<()>;
}

/// Live executor — sends orders to Alpaca.
pub struct AlpacaOrderExecutor {
    client: AlpacaRestClient,
    max_order_size_usd: f64,
}

impl AlpacaOrderExecutor {
    pub fn new(client: AlpacaRestClient, max_order_size_usd: f64) -> Self {
        Self {
            client,
            max_order_size_usd,
        }
    }

    fn validate_signal(&self, signal: &Signal) -> Result<()> {
        if let Some(notional) = signal.notional_usd {
            if notional > self.max_order_size_usd {
                anyhow::bail!(
                    "Signal notional ${:.2} exceeds max order size ${:.2}",
                    notional,
                    self.max_order_size_usd
                );
            }
        }
        Ok(())
    }
}

#[async_trait]
impl OrderExecutor for AlpacaOrderExecutor {
    #[instrument(skip(self), fields(symbol = %signal.symbol, side = ?signal.side))]
    async fn execute(&self, signal: Signal) -> Result<Order> {
        self.validate_signal(&signal).context("Signal validation failed")?;

        info!(
            symbol = %signal.symbol,
            side = ?signal.side,
            reason = %signal.reason,
            "Executing order signal"
        );

        let order_request = OrderRequest::try_from(signal)?;
        let order = self
            .client
            .place_order(&order_request)
            .await
            .context("Failed to place order with Alpaca")?;

        info!(order_id = %order.id, symbol = %order.symbol, "Order placed");
        Ok(order)
    }

    #[instrument(skip(self))]
    async fn cancel(&self, order_id: &str) -> Result<()> {
        warn!(order_id, "Cancelling order");
        self.client
            .cancel_order(order_id)
            .await
            .context("Failed to cancel order")
    }
}

/// Dry-run executor — logs signals but never hits the API. Useful for back-testing.
pub struct PaperExecutor;

#[async_trait]
impl OrderExecutor for PaperExecutor {
    async fn execute(&self, signal: Signal) -> Result<Order> {
        info!(
            symbol = %signal.symbol,
            side = ?signal.side,
            reason = %signal.reason,
            "[DRY RUN] Would execute order"
        );
        anyhow::bail!("PaperExecutor does not return real orders")
    }

    async fn cancel(&self, order_id: &str) -> Result<()> {
        info!(order_id, "[DRY RUN] Would cancel order");
        Ok(())
    }
}
