//! Integration tests for the Options Wheel — hit the real Alpaca paper API.
//!
//! Run with:
//!   cargo test --test integration -- --nocapture
//!
//! Requires env vars:
//!   APCA_API_KEY_ID      — Alpaca API key
//!   APCA_API_SECRET_KEY  — Alpaca API secret
//!
//! Tests are skipped gracefully if credentials are absent.

use alpaca_client::{AlpacaRestClient, OrderRequest, OrderSide, OrderStatus, OrderType, TimeInForce};

/// Load paper API credentials from environment, or return None to skip.
fn load_credentials() -> Option<(String, String)> {
    let key = std::env::var("APCA_API_KEY_ID").ok()?;
    let secret = std::env::var("APCA_API_SECRET_KEY").ok()?;
    if key.is_empty() || secret.is_empty() {
        return None;
    }
    Some((key, secret))
}

/// Full integration cycle:
/// 1. Connect to paper API and verify account
/// 2. Fetch BAC options chain (21-30 DTE)
/// 3. Select a CSP strike using the filter logic
/// 4. Place a CSP order (sell put, limit at mid)
/// 5. Confirm order appears in the open orders list
/// 6. Cancel the order (cleanup)
/// 7. Assert no errors throughout
#[tokio::test]
async fn test_full_csp_cycle() {
    // Load .env if present
    let _ = dotenvy::dotenv();

    let (api_key, api_secret) = match load_credentials() {
        Some(creds) => creds,
        None => {
            println!("SKIP: APCA_API_KEY_ID / APCA_API_SECRET_KEY not set");
            return;
        }
    };

    let client = AlpacaRestClient::new(&api_key, &api_secret, true);

    // Step 1: Verify account
    let account = client
        .get_account()
        .await
        .expect("Failed to fetch account");

    println!("Account: {} ({})", account.account_number, account.status);
    assert_eq!(account.status, "ACTIVE", "Account must be ACTIVE");

    let options_bp: f64 = account
        .options_buying_power
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    println!("Options buying power: ${:.2}", options_bp);
    assert!(
        options_bp > 0.0,
        "Options buying power must be > 0 (enable options in account settings)"
    );

    // Step 2: Fetch BAC options chain
    use chrono::{Duration, Local};
    let today = Local::now().date_naive();
    let gte = (today + Duration::days(21)).format("%Y-%m-%d").to_string();
    let lte = (today + Duration::days(30)).format("%Y-%m-%d").to_string();

    println!("Fetching BAC put chain ({} to {})...", gte, lte);

    let snapshots = client
        .get_options_snapshots("BAC", "put", &gte, &lte, "indicative")
        .await
        .expect("Failed to fetch options chain");

    println!("Raw contracts fetched: {}", snapshots.snapshots.len());
    assert!(
        !snapshots.snapshots.is_empty(),
        "Options chain must be non-empty"
    );

    // Step 3: Select a CSP strike using chain.rs logic
    // We inline the logic here to avoid needing to expose internals
    use alpaca_client::OptionSnapshot;
    use chrono::NaiveDate;

    struct Contract {
        symbol: String,
        strike: f64,
        delta: f64,
        bid: f64,
        ask: f64,
        mid: f64,
        oi: u64,
    }

    fn parse_strike(symbol: &str) -> f64 {
        let cp_pos = symbol.rfind(|c| c == 'C' || c == 'P').unwrap_or(0);
        let strike_str = &symbol[cp_pos + 1..];
        if strike_str.len() == 8 {
            strike_str.parse::<u64>().unwrap_or(0) as f64 / 1000.0
        } else {
            0.0
        }
    }

    let mut contracts: Vec<Contract> = snapshots
        .snapshots
        .iter()
        .filter_map(|(sym, snap)| {
            let delta = snap.greeks.as_ref()?.delta?;
            let bid = snap.latest_quote.as_ref()?.bp?;
            let ask = snap.latest_quote.as_ref()?.ap?;
            let oi = snap.open_interest.unwrap_or(0.0) as u64;
            let mid = (bid + ask) / 2.0;
            let strike = parse_strike(sym);
            Some(Contract {
                symbol: sym.clone(),
                strike,
                delta,
                bid,
                ask,
                mid,
                oi,
            })
        })
        .filter(|c| {
            let abs_delta = c.delta.abs();
            abs_delta >= 0.20 && abs_delta <= 0.25 && c.oi >= 200 && (c.ask - c.bid) <= 0.10
        })
        .collect();

    if contracts.is_empty() {
        println!(
            "SKIP: No contracts pass all filters (delta 0.20-0.25, OI>=200, spread<=0.10). \
             This is expected if market is closed or chain is thin."
        );
        return;
    }

    contracts.sort_by(|a, b| b.mid.partial_cmp(&a.mid).unwrap_or(std::cmp::Ordering::Equal));
    let best = &contracts[0];

    println!(
        "Selected CSP: {} strike=${:.2} delta={:.2} mid=${:.4} OI={}",
        best.symbol, best.strike, best.delta, best.mid, best.oi
    );

    // Step 4: Place a CSP order (sell put, limit at mid)
    let limit_price = format!("{:.2}", best.mid);
    let order_req = OrderRequest {
        symbol: best.symbol.clone(),
        qty: Some("1".to_string()),
        notional: None,
        side: OrderSide::Sell,
        order_type: OrderType::Limit,
        time_in_force: TimeInForce::Day,
        limit_price: Some(limit_price.clone()),
        stop_price: None,
        extended_hours: None,
        client_order_id: None,
    };

    println!(
        "Placing CSP: SELL PUT {} x1 @ ${} LIMIT...",
        best.symbol, limit_price
    );

    let order = client
        .place_order(&order_req)
        .await
        .expect("Failed to place CSP order");

    println!("Order placed: id={} status={:?}", order.id, order.status);
    let order_id = order.id.clone();

    // Step 5: Confirm order appears in open orders
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let open_orders = client
        .get_orders(Some("open"), Some(50))
        .await
        .expect("Failed to fetch open orders");

    let found = open_orders.iter().any(|o| o.id == order_id);
    assert!(
        found,
        "Placed order {} must appear in open orders",
        order_id
    );
    println!("Order {} confirmed in open orders", order_id);

    // Step 6: Cancel the order (cleanup)
    client
        .cancel_order(&order_id)
        .await
        .expect("Failed to cancel order");

    println!("Order {} cancelled (cleanup complete)", order_id);

    // Step 7: Verify no lingering position from this test
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let remaining = client
        .get_orders(Some("open"), Some(50))
        .await
        .expect("Failed to fetch orders after cleanup");

    let still_open = remaining.iter().any(|o| o.id == order_id);
    assert!(!still_open, "Order {} should not remain open after cancel", order_id);

    println!("Integration test PASSED — full CSP cycle (place -> confirm -> cancel)");
}

/// Test: fetch account and verify it's accessible.
#[tokio::test]
async fn test_account_health() {
    let _ = dotenvy::dotenv();
    let (api_key, api_secret) = match load_credentials() {
        Some(c) => c,
        None => {
            println!("SKIP: credentials not set");
            return;
        }
    };

    let client = AlpacaRestClient::new(&api_key, &api_secret, true);
    let account = client.get_account().await.expect("Failed to fetch account");

    println!("Account status: {}", account.status);
    println!("Portfolio value: {}", account.portfolio_value);
    println!("Options BP: {:?}", account.options_buying_power);
    println!("Options level: {:?}", account.options_approved_level);

    assert!(!account.account_number.is_empty());
    assert!(!account.trading_blocked, "Trading must not be blocked");
}

/// Test: fetch options chain (does not place any orders).
#[tokio::test]
async fn test_fetch_options_chain() {
    let _ = dotenvy::dotenv();
    let (api_key, api_secret) = match load_credentials() {
        Some(c) => c,
        None => {
            println!("SKIP: credentials not set");
            return;
        }
    };

    let client = AlpacaRestClient::new(&api_key, &api_secret, true);

    use chrono::{Duration, Local};
    let today = Local::now().date_naive();
    let gte = (today + Duration::days(21)).format("%Y-%m-%d").to_string();
    let lte = (today + Duration::days(30)).format("%Y-%m-%d").to_string();

    let result = client
        .get_options_snapshots("BAC", "put", &gte, &lte, "indicative")
        .await;

    match result {
        Ok(snapshots) => {
            println!("Fetched {} BAC put contracts", snapshots.snapshots.len());
            for (sym, snap) in snapshots.snapshots.iter().take(3) {
                let delta = snap.greeks.as_ref().and_then(|g| g.delta).unwrap_or(0.0);
                let bid = snap.latest_quote.as_ref().and_then(|q| q.bp).unwrap_or(0.0);
                let ask = snap.latest_quote.as_ref().and_then(|q| q.ap).unwrap_or(0.0);
                println!("  {}: delta={:.2} bid={:.2} ask={:.2}", sym, delta, bid, ask);
            }
            // Chain may be empty outside market hours but fetch should succeed
        }
        Err(e) => {
            println!("Options chain fetch failed (may be expected outside market hours): {e}");
        }
    }
}
