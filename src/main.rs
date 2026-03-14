use anyhow::Result;
use tracing::{error, info};

use alpaca_client::AlpacaClient;
use config_crate::AppConfig;

#[tokio::main]
async fn main() -> Result<()> {
    // ── Load config first so we can configure logging ─────────────────────
    let cfg = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        eprintln!("Hint: set APP__ALPACA__API_KEY and APP__ALPACA__API_SECRET env vars,");
        eprintln!("      or create a config.toml in the working directory.");
        std::process::exit(1);
    });

    // ── Initialise tracing ─────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.logging.level.clone().into()),
        )
        .with_target(true)
        .with_thread_ids(false)
        .compact()
        .init();

    info!(
        paper = cfg.alpaca.paper,
        "nonthok-cashflow starting"
    );

    if cfg.alpaca.paper {
        info!("Running in PAPER TRADING mode — no real money at risk");
    } else {
        // Intentional loud warning for live mode
        tracing::warn!("⚠️  LIVE TRADING MODE — real money at risk!");
    }

    // ── Build Alpaca client ────────────────────────────────────────────────
    let alpaca = AlpacaClient::new(
        &cfg.alpaca.api_key,
        &cfg.alpaca.api_secret,
        cfg.alpaca.paper,
    );

    // ── Connectivity check ─────────────────────────────────────────────────
    match alpaca.rest.get_account().await {
        Ok(account) => {
            info!(
                account_number = %account.account_number,
                equity = %account.equity,
                buying_power = %account.buying_power,
                "Account verified"
            );
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch account — check credentials");
            return Err(e.into());
        }
    }

    // ── Main event loop placeholder ────────────────────────────────────────
    //
    // TODO: Once the strategy team defines the approach (stat-arb, option
    // spreads, or other), wire in:
    //   1. A strategy trait implementation
    //   2. A market-data subscription via `alpaca.ws.subscribe()`
    //   3. The `AlpacaOrderExecutor` for live signals
    //
    info!("Scaffold ready. Awaiting strategy implementation.");

    // Keep the process alive so the WebSocket task (when added) can run.
    tokio::signal::ctrl_c().await?;
    info!("Shutting down gracefully");

    Ok(())
}
