use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Account ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub id: String,
    pub account_number: String,
    pub status: String,
    pub currency: String,
    pub buying_power: String,
    pub cash: String,
    pub portfolio_value: String,
    pub pattern_day_trader: bool,
    pub trading_blocked: bool,
    pub transfers_blocked: bool,
    pub account_blocked: bool,
    pub created_at: DateTime<Utc>,
    pub shorting_enabled: bool,
    pub long_market_value: String,
    pub short_market_value: String,
    pub equity: String,
    pub last_equity: String,
    pub multiplier: String,
    pub initial_margin: String,
    pub maintenance_margin: String,
    pub daytrade_count: i64,
    /// Buying power available for options (paper accounts may omit this)
    pub options_buying_power: Option<String>,
    /// Options trading approval level (0=not approved, 1=covered, 2=naked)
    pub options_approved_level: Option<u8>,
}

// ─── Stock Quotes ─────────────────────────────────────────────────────────────

/// Latest stock quote returned by the data API.
#[derive(Debug, Clone, Deserialize)]
pub struct StockQuote {
    /// Ask price
    pub ap: f64,
    /// Bid price
    pub bp: f64,
    /// Ask size (`as` in JSON — renamed because `as` is a Rust keyword)
    #[serde(rename = "as")]
    pub ask_size: Option<u64>,
    /// Bid size
    pub bs: Option<u64>,
}

/// Response wrapper for `/v1beta1/stocks/quotes/latest`
#[derive(Debug, Clone, Deserialize)]
pub struct StockQuotesResponse {
    pub quotes: std::collections::HashMap<String, StockQuote>,
}

/// Simplified quote with mid-price computed.
#[derive(Debug, Clone)]
pub struct UnderlyingQuote {
    pub bid: f64,
    pub ask: f64,
    pub mid: f64,
}

// ─── Orders ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
    TrailingStop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    Day,
    Gtc,
    Opg,
    Cls,
    Ioc,
    Fok,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    DoneForDay,
    Canceled,
    Expired,
    Replaced,
    PendingCancel,
    PendingReplace,
    PendingNew,
    Accepted,
    PendingNew2,
    AcceptedForBidding,
    Stopped,
    Rejected,
    Suspended,
    Calculated,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Order {
    pub id: String,
    pub client_order_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub filled_at: Option<DateTime<Utc>>,
    pub expired_at: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub replaced_at: Option<DateTime<Utc>>,
    pub replaced_by: Option<String>,
    pub replaces: Option<String>,
    pub asset_id: String,
    pub symbol: String,
    pub asset_class: String,
    pub qty: Option<String>,
    pub notional: Option<String>,
    pub filled_qty: String,
    pub filled_avg_price: Option<String>,
    pub order_class: String,
    pub order_type: OrderType,
    pub side: OrderSide,
    pub time_in_force: TimeInForce,
    pub limit_price: Option<String>,
    pub stop_price: Option<String>,
    pub status: OrderStatus,
    pub extended_hours: bool,
    pub trail_percent: Option<String>,
    pub trail_price: Option<String>,
    pub hwm: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub qty: Option<String>,
    pub notional: Option<String>,
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_hours: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
}

// ─── Positions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Position {
    pub asset_id: String,
    pub symbol: String,
    pub exchange: String,
    pub asset_class: String,
    pub avg_entry_price: String,
    pub qty: String,
    pub qty_available: String,
    pub side: String,
    pub market_value: String,
    pub cost_basis: String,
    pub unrealized_pl: String,
    pub unrealized_plpc: String,
    pub unrealized_intraday_pl: String,
    pub unrealized_intraday_plpc: String,
    pub current_price: String,
    pub lastday_price: String,
    pub change_today: String,
}

// ─── Market Data ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Bar {
    pub t: DateTime<Utc>,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: u64,
    pub n: u64,
    pub vw: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Trade {
    pub t: DateTime<Utc>,
    pub p: f64,
    pub s: u64,
    pub x: String,
    pub i: i64,
    pub c: Vec<String>,
    pub z: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Quote {
    pub t: DateTime<Utc>,
    pub ax: String,
    pub ap: f64,
    pub asz: u64,
    pub bx: String,
    pub bp: f64,
    pub bsz: u64,
    pub c: Vec<String>,
    pub z: String,
}

// ─── WebSocket Messages ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "T", rename_all = "lowercase")]
pub enum WsMessage {
    #[serde(rename = "t")]
    Trade {
        #[serde(rename = "S")]
        symbol: String,
        #[serde(flatten)]
        trade: Trade,
    },
    #[serde(rename = "q")]
    Quote {
        #[serde(rename = "S")]
        symbol: String,
        #[serde(flatten)]
        quote: Quote,
    },
    #[serde(rename = "b")]
    Bar {
        #[serde(rename = "S")]
        symbol: String,
        #[serde(flatten)]
        bar: Bar,
    },
    #[serde(rename = "success")]
    Success { msg: String },
    #[serde(rename = "error")]
    Error { code: i32, msg: String },
    #[serde(rename = "subscription")]
    Subscription {
        trades: Vec<String>,
        quotes: Vec<String>,
        bars: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct WsAuth {
    pub action: String,
    pub key: String,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WsSubscribe {
    pub action: String,
    pub trades: Vec<String>,
    pub quotes: Vec<String>,
    pub bars: Vec<String>,
}
