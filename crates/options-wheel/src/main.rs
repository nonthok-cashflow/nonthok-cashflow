mod account;
mod chain;
mod config;
mod orders;
mod positions;
mod wheel;

use anyhow::Result;
use tracing::info;

use alpaca_client::AlpacaRestClient;

#[tokio::main]
async fn main() -> Result<()> {
    // ── Load config ──────────────────────────────────────────────────────────
    let cfg = config::WheelConfig::load().unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        eprintln!("Required env vars: APCA_API_KEY_ID, APCA_API_SECRET_KEY");
        eprintln!("Optional: APCA_API_BASE_URL, APCA_DATA_URL, MAX_BUYING_POWER, UNDERLYING, TARGET_DTE_MIN, TARGET_DTE_MAX");
        std::process::exit(1);
    });

    // ── Init logging ─────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .compact()
        .init();

    info!(
        underlying = %cfg.underlying,
        max_buying_power = cfg.max_buying_power,
        dte_window = format!("{}-{}", cfg.target_dte_min, cfg.target_dte_max),
        "Options Wheel starting (paper mode)"
    );

    // ── Build REST client (always paper for now) ─────────────────────────────
    let client = AlpacaRestClient::new(&cfg.api_key, &cfg.api_secret, true);

    // ── Account health check ─────────────────────────────────────────────────
    let account_state = account::check_account(&client).await?;

    info!(
        options_buying_power = account_state.options_buying_power,
        options_approved_level = account_state.options_approved_level,
        "Account verified"
    );

    // ── Effective buying power (capped by config) ────────────────────────────
    let effective_bp = account_state.options_buying_power.min(cfg.max_buying_power);
    info!(effective_buying_power = effective_bp, "Effective buying power");

    // ── Underlying mid-price ─────────────────────────────────────────────────
    let quote = account::fetch_underlying_quote(&client, &cfg.underlying).await?;

    info!(
        symbol = %cfg.underlying,
        bid = quote.bid,
        ask = quote.ask,
        mid = quote.mid,
        "Underlying price fetched"
    );

    println!("\n=== Options Wheel — Phase 1 ===");
    println!("Underlying:         {} @ ${:.2} mid", cfg.underlying, quote.mid);
    println!("Options BP:         ${:.2}", account_state.options_buying_power);
    println!("Effective BP cap:   ${:.2}", effective_bp);
    println!("Approval level:     {}", account_state.options_approved_level);
    println!("DTE window:         {}-{} days", cfg.target_dte_min, cfg.target_dte_max);
    println!("================================");

    // ── Load persisted state ─────────────────────────────────────────────────
    let state = wheel::load_state();
    info!("[WHEEL] Loaded state: {:?}", state);
    println!("Current state:      {:?}", state);

    // ── Advance state machine ────────────────────────────────────────────────
    let new_state = wheel::step(state, &client, &cfg).await?;
    info!("[WHEEL] New state: {:?}", new_state);
    println!("New state:          {:?}", new_state);

    // ── Persist new state ────────────────────────────────────────────────────
    wheel::save_state(&new_state)?;

    println!("================================");
    println!("Step complete. Run again to advance the wheel.");

    Ok(())
}
