// wheel.rs — State machine + scheduler
// Implemented in TNA-14.

use serde::{Deserialize, Serialize};

/// Current state of the Options Wheel cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum WheelState {
    /// No open position — ready to start a new cycle.
    Idle,
    /// Waiting for a CSP order to fill or expire.
    WatchingCSP {
        order_id: String,
        symbol: String,
    },
    /// Assigned 100 shares after CSP assignment.
    AssignedLong {
        entry_cost: f64,
    },
    /// Watching a covered call for fill or expiry.
    WatchingCC {
        order_id: String,
        symbol: String,
    },
    /// Shares called away — cycle complete.
    Called {
        realized_pnl: f64,
    },
}

impl Default for WheelState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Advance the wheel state machine by one step.
/// TODO: implement in TNA-14
#[allow(dead_code)]
pub fn step(_state: WheelState) -> WheelState {
    unimplemented!("wheel::step: implement in TNA-14")
}
