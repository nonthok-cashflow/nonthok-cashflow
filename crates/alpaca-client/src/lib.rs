pub mod error;
pub mod models;
pub mod rest;
pub mod ws;

pub use error::{AlpacaError, Result};
pub use models::{
    Account, Bar, Order, OrderRequest, OrderSide, OrderStatus, OrderType, Position, Quote,
    StockQuote, StockQuotesResponse, TimeInForce, Trade, UnderlyingQuote, WsAuth, WsMessage,
    WsSubscribe,
};
pub use rest::AlpacaRestClient;
pub use ws::AlpacaWsClient;

/// Convenience struct bundling both REST and WebSocket clients.
pub struct AlpacaClient {
    pub rest: AlpacaRestClient,
    pub ws: AlpacaWsClient,
}

impl AlpacaClient {
    pub fn new(api_key: impl Into<String> + Clone, api_secret: impl Into<String> + Clone, paper: bool) -> Self {
        Self {
            rest: AlpacaRestClient::new(api_key.clone(), api_secret.clone(), paper),
            ws: AlpacaWsClient::new(api_key, api_secret, paper),
        }
    }
}
