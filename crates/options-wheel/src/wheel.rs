// wheel.rs — State machine + scheduler (TNA-14)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::info;

use alpaca_client::AlpacaRestClient;

use crate::chain::{fetch_call_chain, fetch_put_chain, select_cc_strike, select_csp_strike};
use crate::config::WheelConfig;
use crate::iv_history;
use crate::orders::{place_buy_to_close, place_cc_order, place_csp_order};
use crate::positions::{detect_wheel_event, get_position_summary, log_all_positions, WheelEvent};

/// Current state of the Options Wheel cycle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "state")]
pub enum WheelState {
    /// No open position — ready to start a new cycle.
    #[default]
    Idle,
    /// Waiting for a CSP to fill, or monitoring the short put position.
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

/// Resolve the state file path (~/.wheel_state.json).
fn state_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".wheel_state.json")
}

/// Load state from disk; falls back to Idle on any error.
pub fn load_state() -> WheelState {
    let path = state_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse state file: {e}, starting from Idle");
            WheelState::Idle
        }),
        Err(_) => {
            info!("No state file found at {:?}, starting from Idle", path);
            WheelState::Idle
        }
    }
}

/// Persist state to disk.
pub fn save_state(state: &WheelState) -> Result<()> {
    let path = state_path();
    let data = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, &data)?;
    info!("State saved to {:?}", path);
    Ok(())
}

/// Advance the wheel state machine by one step.
///
/// Each step performs exactly one action and returns the new state.
/// The scheduler calls this once per run.
pub async fn step(
    state: WheelState,
    client: &AlpacaRestClient,
    cfg: &WheelConfig,
) -> Result<WheelState> {
    // Always log current positions for visibility
    let _ = log_all_positions(client).await;

    match state {
        WheelState::Idle => {
            info!("[WHEEL] State: Idle — checking IV gate");

            // ── IV gate (before delta/OI/spread filters) ──────────────────
            let today = chrono::Local::now().date_naive();
            let iv_conn = match iv_history::open_db() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[IV] Failed to open iv_history DB: {e}, skipping gate");
                    // Fall through to CSP entry if DB unavailable
                    return {
                        let chain = fetch_put_chain(
                            &cfg.underlying,
                            client,
                            cfg.target_dte_min,
                            cfg.target_dte_max,
                        )
                        .await?;
                        let contract = select_csp_strike(&chain)?;
                        info!(
                            "[WHEEL] Idle -> WatchingCSP: {}, delta={:.2}, premium=${:.2}",
                            contract.symbol, contract.delta, contract.mid
                        );
                        let result =
                            place_csp_order(client, &contract.symbol, contract.strike_price, contract.mid)
                                .await?;
                        info!(
                            "[ORDER] SELL PUT {} x1 @ ${:.2} LIMIT -> filled @ ${:.2}",
                            contract.symbol, contract.mid, result.filled_price
                        );
                        Ok(WheelState::WatchingCSP {
                            order_id: result.order_id,
                            symbol: contract.symbol.clone(),
                        })
                    };
                }
            };

            let iv_favorable = iv_history::capture_and_check_iv(
                &iv_conn,
                &cfg.underlying,
                today,
                client,
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[IV] IV gate check failed: {e}, defaulting to favorable");
                true
            });

            if !iv_favorable {
                info!(
                    "[WHEEL] Idle: IV gate FAILED for {} — skipping CSP entry today",
                    cfg.underlying
                );
                return Ok(WheelState::Idle);
            }

            info!("[WHEEL] IV gate PASSED for {} — fetching options chain", cfg.underlying);

            let chain = fetch_put_chain(&cfg.underlying, client, cfg.target_dte_min, cfg.target_dte_max).await?;
            let contract = select_csp_strike(&chain)?;

            info!(
                "[WHEEL] Idle -> WatchingCSP: {}, delta={:.2}, premium=${:.2}",
                contract.symbol, contract.delta, contract.mid
            );

            let result = place_csp_order(client, &contract.symbol, contract.strike_price, contract.mid).await?;

            info!(
                "[ORDER] SELL PUT {} x1 @ ${:.2} LIMIT -> filled @ ${:.2}",
                contract.symbol, contract.mid, result.filled_price
            );

            Ok(WheelState::WatchingCSP {
                order_id: result.order_id,
                symbol: contract.symbol.clone(),
            })
        }

        WheelState::WatchingCSP { order_id, symbol } => {
            info!("[WHEEL] State: WatchingCSP {{ order_id={}, symbol={} }}", order_id, symbol);

            let event = detect_wheel_event(client, &cfg.underlying).await?;

            match event {
                WheelEvent::Assigned => {
                    let pos = client.get_position(&cfg.underlying).await?;
                    let entry_price: f64 = pos.avg_entry_price.parse().unwrap_or(0.0);
                    let qty: i64 = pos.qty.parse().unwrap_or(0);
                    let entry_cost = entry_price * qty as f64;

                    info!(
                        "[WHEEL] WatchingCSP -> AssignedLong: {} shares @ ${:.2}/share, total cost ${:.2}",
                        qty, entry_price, entry_cost
                    );

                    Ok(WheelState::AssignedLong { entry_cost })
                }

                WheelEvent::Expired => {
                    info!("[WHEEL] WatchingCSP -> Idle: {} expired worthless, collecting premium", symbol);
                    Ok(WheelState::Idle)
                }

                WheelEvent::Active => {
                    // Log position P&L if the short put is still open
                    if let Ok(Some(summary)) = get_position_summary(client, &symbol).await {
                        info!(
                            "[PNL] Position P&L: {:+.0} ({:.0}% of initial premium), DTE={}",
                            summary.unrealized_pl,
                            summary.unrealized_plpc * 100.0,
                            summary.dte_remaining
                        );

                        // 50% profit trigger — buy to close early
                        if summary.unrealized_plpc >= 0.50 {
                            info!(
                                "[WHEEL] WatchingCSP: 50%+ profit captured ({:.0}%), triggering buy-to-close",
                                summary.unrealized_plpc * 100.0
                            );
                            let close_price = summary.current_price;
                            let result = place_buy_to_close(client, &symbol, close_price).await?;
                            info!(
                                "[ORDER] BUY {} x1 @ ${:.2} LIMIT -> filled @ ${:.2} (50% profit close)",
                                symbol, close_price, result.filled_price
                            );
                            return Ok(WheelState::Idle);
                        }
                    }

                    info!("[WHEEL] WatchingCSP: position still active, monitoring");
                    Ok(WheelState::WatchingCSP { order_id, symbol })
                }
            }
        }

        WheelState::AssignedLong { entry_cost } => {
            info!(
                "[WHEEL] State: AssignedLong {{ entry_cost=${:.2} }} — fetching call chain",
                entry_cost
            );

            let chain = match fetch_call_chain(
                &cfg.underlying,
                client,
                cfg.target_dte_min,
                cfg.target_dte_max,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[WHEEL] AssignedLong: failed to fetch call chain: {e}, staying");
                    return Ok(WheelState::AssignedLong { entry_cost });
                }
            };

            let contract = match select_cc_strike(&chain) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[WHEEL] AssignedLong: no CC candidate found: {e}, staying");
                    return Ok(WheelState::AssignedLong { entry_cost });
                }
            };

            info!(
                "[WHEEL] AssignedLong -> WatchingCC: {}, delta={:.2}, premium=${:.2}",
                contract.symbol, contract.delta.abs(), contract.mid
            );

            let result = place_cc_order(client, &cfg.underlying, &contract.symbol, contract.mid).await?;

            info!(
                "[ORDER] SELL CALL {} x1 @ ${:.2} LIMIT -> filled @ ${:.2}",
                contract.symbol, contract.mid, result.filled_price
            );

            Ok(WheelState::WatchingCC {
                order_id: result.order_id,
                symbol: contract.symbol.clone(),
            })
        }

        WheelState::WatchingCC { order_id, symbol } => {
            info!("[WHEEL] State: WatchingCC {{ order_id={}, symbol={} }}", order_id, symbol);

            let event = detect_wheel_event(client, &cfg.underlying).await?;

            match event {
                WheelEvent::Assigned => {
                    // CC was assigned — shares sold at strike price
                    // Get realized P&L from the order (simplified: use position data)
                    let realized_pnl = 0.0; // Would be computed from execution reports

                    info!(
                        "[WHEEL] WatchingCC -> Called: CC assigned, shares sold. Realized P&L: ${:.2}",
                        realized_pnl
                    );

                    Ok(WheelState::Called { realized_pnl })
                }

                WheelEvent::Expired => {
                    // CC expired worthless — keep shares, sell another CC
                    info!("[WHEEL] WatchingCC -> AssignedLong: {} expired worthless, selling another CC", symbol);

                    // Get current entry cost from position
                    let pos = client.get_position(&cfg.underlying).await?;
                    let entry_price: f64 = pos.avg_entry_price.parse().unwrap_or(0.0);
                    let qty: i64 = pos.qty.parse().unwrap_or(0);
                    let entry_cost = entry_price * qty as f64;

                    Ok(WheelState::AssignedLong { entry_cost })
                }

                WheelEvent::Active => {
                    // Check for 50% profit trigger on the CC
                    if let Ok(Some(summary)) = get_position_summary(client, &symbol).await {
                        info!(
                            "[PNL] CC P&L: {:+.0} ({:.0}% of premium), DTE={}",
                            summary.unrealized_pl,
                            summary.unrealized_plpc * 100.0,
                            summary.dte_remaining
                        );

                        if summary.unrealized_plpc >= 0.50 {
                            info!(
                                "[WHEEL] WatchingCC: 50%+ profit ({:.0}%), buy-to-close",
                                summary.unrealized_plpc * 100.0
                            );
                            let close_price = summary.current_price;
                            let result = place_buy_to_close(client, &symbol, close_price).await?;
                            info!(
                                "[ORDER] BUY {} x1 @ ${:.2} -> filled @ ${:.2} (50% profit close)",
                                symbol, close_price, result.filled_price
                            );

                            // After BTC, we still hold shares — go back to AssignedLong
                            let pos = client.get_position(&cfg.underlying).await?;
                            let entry_price: f64 = pos.avg_entry_price.parse().unwrap_or(0.0);
                            let qty: i64 = pos.qty.parse().unwrap_or(0);
                            return Ok(WheelState::AssignedLong {
                                entry_cost: entry_price * qty as f64,
                            });
                        }
                    }

                    info!("[WHEEL] WatchingCC: active, monitoring");
                    Ok(WheelState::WatchingCC { order_id, symbol })
                }
            }
        }

        WheelState::Called { realized_pnl } => {
            info!(
                "[WHEEL] State: Called — cycle complete. Realized P&L: ${:.2}. Restarting.",
                realized_pnl
            );
            Ok(WheelState::Idle)
        }
    }
}
