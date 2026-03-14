use reqwest::{Client, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{debug, instrument};

use crate::error::{AlpacaError, Result};
use crate::models::{Account, Order, OrderRequest, OptionsSnapshotsResponse, Position, StockQuotesResponse, UnderlyingQuote};

/// Alpaca paper-trading REST API base URL (v2)
const PAPER_BASE_URL: &str = "https://paper-api.alpaca.markets/v2";
/// Alpaca live trading REST API base URL (v2)
const LIVE_BASE_URL: &str = "https://api.alpaca.markets/v2";
/// Alpaca market data API base URL (v2)
const DATA_BASE_URL: &str = "https://data.alpaca.markets/v2";
/// Alpaca market data API base URL (v1beta1, used for options and latest quotes)
const DATA_BASE_URL_BETA: &str = "https://data.alpaca.markets/v1beta1";

/// REST client for the Alpaca v2 API.
#[derive(Clone)]
pub struct AlpacaRestClient {
    http: Client,
    api_key: String,
    api_secret: String,
    trading_base_url: String,
    data_base_url: String,
    data_base_url_beta: String,
}

impl AlpacaRestClient {
    /// Create a new client. Set `paper = true` to use paper-trading endpoints.
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>, paper: bool) -> Self {
        let http = Client::builder()
            .user_agent("nonthok-cashflow/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            trading_base_url: if paper {
                PAPER_BASE_URL.to_string()
            } else {
                LIVE_BASE_URL.to_string()
            },
            data_base_url: DATA_BASE_URL.to_string(),
            data_base_url_beta: DATA_BASE_URL_BETA.to_string(),
        }
    }

    fn auth_headers(&self, builder: RequestBuilder) -> RequestBuilder {
        builder
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.api_secret)
    }

    async fn handle_response<T: DeserializeOwned>(&self, resp: Response) -> Result<T> {
        let status = resp.status().as_u16();
        if status == 429 {
            return Err(AlpacaError::RateLimit);
        }
        let text = resp.text().await.map_err(AlpacaError::Http)?;
        debug!(status, body = %text, "API response");

        if status < 200 || status >= 300 {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["message"].as_str().map(String::from))
                .unwrap_or_else(|| text.clone());
            return Err(AlpacaError::Api { status, message });
        }

        serde_json::from_str(&text).map_err(AlpacaError::Json)
    }

    // ─── Account ─────────────────────────────────────────────────────────────

    #[instrument(skip(self), name = "get_account")]
    pub async fn get_account(&self) -> Result<Account> {
        let req = self
            .http
            .get(format!("{}/account", self.trading_base_url));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    // ─── Orders ──────────────────────────────────────────────────────────────

    #[instrument(skip(self), name = "place_order")]
    pub async fn place_order(&self, order: &OrderRequest) -> Result<Order> {
        let req = self
            .http
            .post(format!("{}/orders", self.trading_base_url))
            .json(order);
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "get_orders")]
    pub async fn get_orders(&self, status: Option<&str>, limit: Option<u32>) -> Result<Vec<Order>> {
        let mut req = self.http.get(format!("{}/orders", self.trading_base_url));
        if let Some(s) = status {
            req = req.query(&[("status", s)]);
        }
        if let Some(l) = limit {
            req = req.query(&[("limit", l.to_string())]);
        }
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "get_order")]
    pub async fn get_order(&self, order_id: &str) -> Result<Order> {
        let req = self
            .http
            .get(format!("{}/orders/{}", self.trading_base_url, order_id));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "cancel_order")]
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let req = self
            .http
            .delete(format!("{}/orders/{}", self.trading_base_url, order_id));
        let resp = self.auth_headers(req).send().await?;
        let status = resp.status().as_u16();
        if status == 204 {
            return Ok(());
        }
        let text = resp.text().await.map_err(AlpacaError::Http)?;
        Err(AlpacaError::Api {
            status,
            message: text,
        })
    }

    #[instrument(skip(self), name = "cancel_all_orders")]
    pub async fn cancel_all_orders(&self) -> Result<()> {
        let req = self
            .http
            .delete(format!("{}/orders", self.trading_base_url));
        let resp = self.auth_headers(req).send().await?;
        let status = resp.status().as_u16();
        if status == 207 || status == 200 {
            return Ok(());
        }
        let text = resp.text().await.map_err(AlpacaError::Http)?;
        Err(AlpacaError::Api {
            status,
            message: text,
        })
    }

    // ─── Positions ───────────────────────────────────────────────────────────

    #[instrument(skip(self), name = "get_positions")]
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let req = self
            .http
            .get(format!("{}/positions", self.trading_base_url));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "get_position")]
    pub async fn get_position(&self, symbol: &str) -> Result<Position> {
        let req = self
            .http
            .get(format!("{}/positions/{}", self.trading_base_url, symbol));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "close_position")]
    pub async fn close_position(&self, symbol: &str) -> Result<Order> {
        let req = self
            .http
            .delete(format!("{}/positions/{}", self.trading_base_url, symbol));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    // ─── Market Data ─────────────────────────────────────────────────────────

    #[instrument(skip(self), name = "get_latest_trade")]
    pub async fn get_latest_trade(&self, symbol: &str) -> Result<serde_json::Value> {
        let req = self
            .http
            .get(format!("{}/stocks/{}/trades/latest", self.data_base_url, symbol));
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    #[instrument(skip(self), name = "get_bars")]
    pub async fn get_bars(
        &self,
        symbol: &str,
        timeframe: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
    ) -> Result<serde_json::Value> {
        let mut req = self
            .http
            .get(format!("{}/stocks/{}/bars", self.data_base_url, symbol))
            .query(&[("timeframe", timeframe)]);
        if let Some(s) = start {
            req = req.query(&[("start", s)]);
        }
        if let Some(e) = end {
            req = req.query(&[("end", e)]);
        }
        if let Some(l) = limit {
            req = req.query(&[("limit", l.to_string())]);
        }
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    // ─── Options Snapshots (v1beta1) ──────────────────────────────────────────

    /// Fetch option contract snapshots for an underlying symbol.
    ///
    /// Paginates automatically until all contracts are returned.
    #[instrument(skip(self), name = "get_options_snapshots")]
    pub async fn get_options_snapshots(
        &self,
        underlying: &str,
        contract_type: &str,
        expiration_date_gte: &str,
        expiration_date_lte: &str,
        feed: &str,
    ) -> Result<OptionsSnapshotsResponse> {
        let url = format!(
            "{}/options/snapshots/{}",
            self.data_base_url_beta, underlying
        );

        let mut all_snapshots = std::collections::HashMap::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut req = self
                .http
                .get(&url)
                .query(&[
                    ("type", contract_type),
                    ("expiration_date_gte", expiration_date_gte),
                    ("expiration_date_lte", expiration_date_lte),
                    ("feed", feed),
                    ("limit", "1000"),
                ]);

            if let Some(ref token) = page_token {
                req = req.query(&[("page_token", token.as_str())]);
            }

            let resp = self.auth_headers(req).send().await?;
            let page: OptionsSnapshotsResponse = self.handle_response(resp).await?;

            all_snapshots.extend(page.snapshots);
            page_token = page.next_page_token;

            if page_token.is_none() {
                break;
            }
        }

        Ok(OptionsSnapshotsResponse {
            snapshots: all_snapshots,
            next_page_token: None,
        })
    }

    // ─── Account Activities ───────────────────────────────────────────────────

    /// Fetch account activities filtered by activity type(s).
    #[instrument(skip(self), name = "get_account_activities")]
    pub async fn get_account_activities(&self, activity_types: &str) -> Result<serde_json::Value> {
        let req = self
            .http
            .get(format!("{}/account/activities", self.trading_base_url))
            .query(&[("activity_types", activity_types)]);
        let resp = self.auth_headers(req).send().await?;
        self.handle_response(resp).await
    }

    // ─── Stock Quotes (v1beta1) ───────────────────────────────────────────────

    /// Fetch the latest NBBO quote for one or more symbols.
    ///
    /// Uses the `v1beta1` data endpoint with `feed=iex`.
    #[instrument(skip(self), name = "get_stock_quote")]
    pub async fn get_stock_quote(&self, symbol: &str) -> Result<UnderlyingQuote> {
        let req = self
            .http
            .get(format!("{}/stocks/quotes/latest", self.data_base_url_beta))
            .query(&[("symbols", symbol), ("feed", "iex")]);
        let resp = self.auth_headers(req).send().await?;
        let parsed: StockQuotesResponse = self.handle_response(resp).await?;
        let q = parsed.quotes.get(symbol).ok_or_else(|| {
            AlpacaError::Api {
                status: 200,
                message: format!("No quote returned for {}", symbol),
            }
        })?;
        Ok(UnderlyingQuote {
            bid: q.bp,
            ask: q.ap,
            mid: (q.bp + q.ap) / 2.0,
        })
    }
}
