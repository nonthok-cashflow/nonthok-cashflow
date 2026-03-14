use anyhow::{bail, Result};

/// Options Wheel strategy configuration, loaded from environment variables.
///
/// Required env vars:
///   APCA_API_KEY_ID     — Alpaca API key
///   APCA_API_SECRET_KEY — Alpaca API secret
///
/// Optional env vars (with defaults):
///   APCA_API_BASE_URL — default: https://paper-api.alpaca.markets
///   APCA_DATA_URL     — default: https://data.alpaca.markets
///   MAX_BUYING_POWER  — default: 5000.0
///   UNDERLYING        — default: BAC
///   TARGET_DTE_MIN    — default: 21
///   TARGET_DTE_MAX    — default: 30
#[derive(Debug, Clone)]
pub struct WheelConfig {
    pub api_key: String,
    pub api_secret: String,
    pub api_base_url: String,
    pub data_url: String,
    /// Maximum buying power to deploy (simulates $5k constraint on paper account)
    pub max_buying_power: f64,
    /// Underlying symbol to trade
    pub underlying: String,
    /// Minimum days to expiration for option selection
    pub target_dte_min: u32,
    /// Maximum days to expiration for option selection
    pub target_dte_max: u32,
}

impl WheelConfig {
    pub fn load() -> Result<Self> {
        // Load .env if present
        let _ = dotenvy::dotenv();

        let api_key = std::env::var("APCA_API_KEY_ID")
            .map_err(|_| anyhow::anyhow!("APCA_API_KEY_ID not set"))?;
        if api_key.is_empty() {
            bail!("APCA_API_KEY_ID is empty");
        }

        let api_secret = std::env::var("APCA_API_SECRET_KEY")
            .map_err(|_| anyhow::anyhow!("APCA_API_SECRET_KEY not set"))?;
        if api_secret.is_empty() {
            bail!("APCA_API_SECRET_KEY is empty");
        }

        let api_base_url = std::env::var("APCA_API_BASE_URL")
            .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string());

        let data_url = std::env::var("APCA_DATA_URL")
            .unwrap_or_else(|_| "https://data.alpaca.markets".to_string());

        let max_buying_power = std::env::var("MAX_BUYING_POWER")
            .unwrap_or_else(|_| "5000.0".to_string())
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("MAX_BUYING_POWER must be a valid number"))?;

        let underlying = std::env::var("UNDERLYING").unwrap_or_else(|_| "BAC".to_string());

        let target_dte_min = std::env::var("TARGET_DTE_MIN")
            .unwrap_or_else(|_| "21".to_string())
            .parse::<u32>()
            .map_err(|_| anyhow::anyhow!("TARGET_DTE_MIN must be a positive integer"))?;

        let target_dte_max = std::env::var("TARGET_DTE_MAX")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u32>()
            .map_err(|_| anyhow::anyhow!("TARGET_DTE_MAX must be a positive integer"))?;

        if target_dte_min >= target_dte_max {
            bail!("TARGET_DTE_MIN ({}) must be less than TARGET_DTE_MAX ({})", target_dte_min, target_dte_max);
        }

        Ok(Self {
            api_key,
            api_secret,
            api_base_url,
            data_url,
            max_buying_power,
            underlying,
            target_dte_min,
            target_dte_max,
        })
    }
}
