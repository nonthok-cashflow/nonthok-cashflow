use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, instrument, warn};

use crate::error::{AlpacaError, Result};
use crate::models::WsMessage;

const PAPER_WS_URL: &str = "wss://stream.data.alpaca.markets/v2/iex";
const LIVE_WS_URL: &str = "wss://stream.data.alpaca.markets/v2/sip";

/// Streaming WebSocket client for Alpaca market data.
pub struct AlpacaWsClient {
    api_key: String,
    api_secret: String,
    url: String,
}

impl AlpacaWsClient {
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>, paper: bool) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            url: if paper {
                PAPER_WS_URL.to_string()
            } else {
                LIVE_WS_URL.to_string()
            },
        }
    }

    /// Connect, authenticate, subscribe to given symbols, and stream messages
    /// to the returned channel receiver. The connection loop runs as a background task.
    #[instrument(skip(self), name = "ws_subscribe")]
    pub async fn subscribe(
        &self,
        trades: Vec<String>,
        quotes: Vec<String>,
        bars: Vec<String>,
    ) -> Result<mpsc::Receiver<WsMessage>> {
        let (tx, rx) = mpsc::channel::<WsMessage>(1024);

        let (ws_stream, _) = connect_async(&self.url).await?;
        info!(url = %self.url, "WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        // Authenticate
        let auth_msg = json!({
            "action": "auth",
            "key": self.api_key,
            "secret": self.api_secret,
        });
        write
            .send(Message::Text(auth_msg.to_string().into()))
            .await?;

        // Wait for auth confirmation
        if let Some(msg) = read.next().await {
            let msg = msg?;
            debug!(raw = %msg, "Auth response");
        }

        // Subscribe to desired symbols
        let sub_msg = json!({
            "action": "subscribe",
            "trades": trades,
            "quotes": quotes,
            "bars": bars,
        });
        write
            .send(Message::Text(sub_msg.to_string().into()))
            .await?;

        // Spawn reader task
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        // Alpaca sends arrays of messages
                        match serde_json::from_str::<Vec<WsMessage>>(&text) {
                            Ok(messages) => {
                                for ws_msg in messages {
                                    if tx_clone.send(ws_msg).await.is_err() {
                                        // Receiver dropped; stop streaming
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, raw = %text, "Failed to parse WS message");
                            }
                        }
                    }
                    Ok(Message::Close(frame)) => {
                        info!(?frame, "WebSocket closed by server");
                        return;
                    }
                    Ok(Message::Ping(data)) => {
                        // tungstenite auto-replies with Pong; log only
                        debug!("Ping received ({} bytes)", data.len());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!(error = %e, "WebSocket error");
                        return;
                    }
                }
            }
        });

        Ok(rx)
    }
}
