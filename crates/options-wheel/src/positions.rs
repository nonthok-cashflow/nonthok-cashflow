// positions.rs — Position tracking + assignment detection
// Implemented in TNA-13.

use anyhow::Result;

/// Summary of the active wheel position.
#[allow(dead_code)]
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
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum WheelEvent {
    Active,
    Assigned,
    Expired,
}

/// Fetch and summarise the active wheel position.
/// TODO: implement in TNA-13
#[allow(dead_code)]
pub async fn get_position_summary(
    _symbol: &str,
    _api_key: &str,
    _api_secret: &str,
    _api_base_url: &str,
) -> Result<Option<PositionSummary>> {
    unimplemented!("get_position_summary: implement in TNA-13")
}

/// Poll account activities to detect assignment or expiration.
/// TODO: implement in TNA-13
#[allow(dead_code)]
pub async fn detect_wheel_event(
    _underlying: &str,
    _api_key: &str,
    _api_secret: &str,
    _api_base_url: &str,
) -> Result<WheelEvent> {
    unimplemented!("detect_wheel_event: implement in TNA-13")
}
