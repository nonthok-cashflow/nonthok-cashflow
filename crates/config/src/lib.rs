use serde::Deserialize;
use anyhow::Result;

/// Top-level application configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub alpaca: AlpacaConfig,
    pub trading: TradingConfig,
    pub logging: LoggingConfig,
}

/// Alpaca API credentials and endpoint selection.
#[derive(Debug, Clone, Deserialize)]
pub struct AlpacaConfig {
    /// Alpaca API key ID
    pub api_key: String,
    /// Alpaca API secret key
    pub api_secret: String,
    /// Use paper trading endpoints (strongly recommended to start with `true`)
    #[serde(default = "default_true")]
    pub paper: bool,
}

/// Trading behaviour parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TradingConfig {
    /// Maximum single-order notional value in USD
    #[serde(default = "default_max_order_size")]
    pub max_order_size_usd: f64,
    /// Maximum number of concurrent open positions
    #[serde(default = "default_max_positions")]
    pub max_open_positions: usize,
    /// Symbols to watch (e.g. ["AAPL", "SPY"])
    #[serde(default)]
    pub watchlist: Vec<String>,
    /// Risk percentage of portfolio per trade (e.g. 0.01 = 1%)
    #[serde(default = "default_risk_pct")]
    pub risk_per_trade_pct: f64,
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// `tracing` filter directive, e.g. "info" or "nonthok_cashflow=debug,info"
    #[serde(default = "default_log_level")]
    pub level: String,
}

// ─── Defaults ────────────────────────────────────────────────────────────────

fn default_true() -> bool { true }
fn default_max_order_size() -> f64 { 1000.0 }
fn default_max_positions() -> usize { 5 }
fn default_risk_pct() -> f64 { 0.01 }
fn default_log_level() -> String { "info".to_string() }

// ─── Loader ──────────────────────────────────────────────────────────────────

impl AppConfig {
    /// Load configuration from environment variables and optional `config.toml`.
    ///
    /// Environment variable names follow the pattern `APP__<SECTION>__<KEY>`,
    /// e.g. `APP__ALPACA__API_KEY`.  A `.env` file is loaded first if present.
    pub fn load() -> Result<Self> {
        // Load .env if present (silently ignore missing file)
        let _ = dotenvy::dotenv();

        let cfg = config::Config::builder()
            // 1. Defaults from optional config file
            .add_source(
                config::File::with_name("config")
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            // 2. Environment variables override file values
            .add_source(
                config::Environment::with_prefix("APP")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let app: AppConfig = cfg.try_deserialize()?;
        Ok(app)
    }
}
