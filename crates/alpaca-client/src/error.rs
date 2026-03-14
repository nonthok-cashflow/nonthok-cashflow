use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlpacaError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("API error: status={status}, message={message}")]
    Api { status: u16, message: String },

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Rate limit exceeded")]
    RateLimit,

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Unexpected message type: {0}")]
    UnexpectedMessage(String),
}

pub type Result<T> = std::result::Result<T, AlpacaError>;
