//! Feishu WebSocket long-connection client.
//!
//! Protocol reverse-engineered from the official Go/Python/Node SDKs:
//!   1. POST /callback/ws/endpoint  →  WSS URL + ClientConfig
//!   2. Dial WSS URL (contains device_id, service_id in query)
//!   3. Binary frames: Protobuf `pbbp2.Frame`
//!   4. Ping loop (default 120s) to keep alive
//!   5. Receive DATA frames → dispatch events → ACK back

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use super::frame::*;

const GEN_ENDPOINT_URI: &str = "/callback/ws/endpoint";
const DEFAULT_PING_INTERVAL: u64 = 120;
const DEFAULT_RECONNECT_INTERVAL: u64 = 120;
const DEFAULT_RECONNECT_NONCE: u64 = 30;

type WsWriter = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;
type WsReader = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

// ---------------------------------------------------------------------------
// API types for /callback/ws/endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct EndpointRequest {
    #[serde(rename = "AppID")]
    app_id: String,
    #[serde(rename = "AppSecret")]
    app_secret: String,
}

#[derive(Debug, Deserialize)]
struct EndpointResp {
    code: i32,
    #[serde(default)]
    msg: String,
    data: Option<EndpointData>,
}

#[derive(Debug, Deserialize)]
struct EndpointData {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<ClientConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientConfig {
    #[serde(rename = "ReconnectCount", default)]
    pub reconnect_count: i32,
    #[serde(rename = "ReconnectInterval", default)]
    pub reconnect_interval: u64,
    #[serde(rename = "ReconnectNonce", default)]
    pub reconnect_nonce: u64,
    #[serde(rename = "PingInterval", default)]
    pub ping_interval: u64,
}

// ---------------------------------------------------------------------------
// Event callback
// ---------------------------------------------------------------------------

/// The payload delivered to the handler after a DATA frame is assembled.
#[derive(Debug, Clone)]
pub struct WsEvent {
    pub message_type: String,
    pub message_id: String,
    pub trace_id: String,
    pub payload: Vec<u8>,
}

pub type EventSender = mpsc::UnboundedSender<WsEvent>;
pub type EventReceiver = mpsc::UnboundedReceiver<WsEvent>;

// ---------------------------------------------------------------------------
// FeishuWsClient
// ---------------------------------------------------------------------------

pub struct FeishuWsClient {
    app_id: String,
    app_secret: String,
    domain: String,
    http: reqwest::Client,
    event_tx: Arc<Mutex<Option<EventSender>>>,

    writer: Arc<Mutex<Option<WsWriter>>>,
    service_id: Arc<Mutex<i32>>,
    conn_id: Arc<Mutex<String>>,

    ping_interval: Arc<Mutex<u64>>,
    reconnect_interval: Arc<Mutex<u64>>,
    reconnect_nonce: Arc<Mutex<u64>>,
    reconnect_count: Arc<Mutex<i32>>,

    shutdown: Arc<Notify>,
    cancel: CancellationToken,
    #[allow(clippy::type_complexity)]
    fragment_cache: Arc<Mutex<HashMap<String, Vec<Option<Vec<u8>>>>>>,
}

impl FeishuWsClient {
    pub fn new(
        app_id: &str,
        app_secret: &str,
        domain: &str,
        event_tx: EventSender,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("XiaoLin/0.1.0")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            domain: domain.trim_end_matches('/').to_string(),
            http,
            event_tx: Arc::new(Mutex::new(Some(event_tx))),
            writer: Arc::new(Mutex::new(None)),
            service_id: Arc::new(Mutex::new(0)),
            conn_id: Arc::new(Mutex::new(String::new())),
            ping_interval: Arc::new(Mutex::new(DEFAULT_PING_INTERVAL)),
            reconnect_interval: Arc::new(Mutex::new(DEFAULT_RECONNECT_INTERVAL)),
            reconnect_nonce: Arc::new(Mutex::new(DEFAULT_RECONNECT_NONCE)),
            reconnect_count: Arc::new(Mutex::new(-1)),
            shutdown: Arc::new(Notify::new()),
            cancel: CancellationToken::new(),
            fragment_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Start the client — connects, spawns ping + receive loops, auto-reconnects.
    pub async fn start(self: Arc<Self>) -> anyhow::Result<()> {
        if let Err(e) = self.connect().await {
            tracing::error!(error = %e, "feishu ws: initial connect failed");
            self.reconnect().await?;
        }
        self.spawn_ping_loop();
        Ok(())
    }

    /// Stop the client gracefully.
    pub fn stop(&self) {
        self.cancel.cancel();
        self.shutdown.notify_waiters();

        let event_tx = Arc::clone(&self.event_tx);
        let writer = Arc::clone(&self.writer);
        let conn_id = Arc::clone(&self.conn_id);
        tokio::spawn(async move {
            *event_tx.lock().await = None;
            let mut w = writer.lock().await;
            if let Some(ref mut ws_writer) = *w {
                let _ = ws_writer.close().await;
            }
            *w = None;
            *conn_id.lock().await = String::new();
            tracing::info!("feishu ws: stopped (writer closed, event channel dropped)");
        });
    }

    // -----------------------------------------------------------------------
    // Connection lifecycle
    // -----------------------------------------------------------------------

    async fn get_conn_url(&self) -> anyhow::Result<(String, Option<ClientConfig>)> {
        let url = format!("{}{}", self.domain, GEN_ENDPOINT_URI);
        let body = EndpointRequest {
            app_id: self.app_id.clone(),
            app_secret: self.app_secret.clone(),
        };

        let raw_resp = self
            .http
            .post(&url)
            .header("locale", "zh")
            .json(&body)
            .send()
            .await?;

        let status = raw_resp.status();
        let text = raw_resp.text().await?;
        tracing::debug!(status = %status, body_len = text.len(), "feishu ws endpoint response");

        if !status.is_success() {
            anyhow::bail!(
                "feishu ws endpoint HTTP {}: {}",
                status,
                &text[..text.floor_char_boundary(text.len().min(500))]
            );
        }

        let resp: EndpointResp = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "feishu ws endpoint parse error: {} — body: {}",
                e,
                &text[..text.floor_char_boundary(text.len().min(500))]
            )
        })?;

        match resp.code {
            0 => {}
            _ => anyhow::bail!("feishu ws endpoint error ({}): {}", resp.code, resp.msg),
        }

        let data = resp
            .data
            .ok_or_else(|| anyhow::anyhow!("feishu ws endpoint returned no data"))?;
        if data.url.is_empty() {
            anyhow::bail!("feishu ws endpoint returned empty URL");
        }
        tracing::info!(url_len = data.url.len(), "feishu ws: obtained WSS URL");
        Ok((data.url, data.client_config))
    }

    async fn connect(self: &Arc<Self>) -> anyhow::Result<()> {
        let (wss_url, config) = self.get_conn_url().await?;

        if let Some(conf) = config {
            self.apply_config(&conf).await;
        }

        let parsed = Url::parse(&wss_url)?;
        let device_id = parsed
            .query_pairs()
            .find(|(k, _)| k == "device_id")
            .map(|(_, v): (_, std::borrow::Cow<'_, str>)| v.to_string())
            .unwrap_or_default();
        let service_id: i32 = parsed
            .query_pairs()
            .find(|(k, _)| k == "service_id")
            .map(|(_, v): (_, std::borrow::Cow<'_, str>)| v.parse().unwrap_or(0))
            .unwrap_or(0);

        let (ws_stream, _) = connect_async(&wss_url).await?;
        let (writer, reader) = ws_stream.split();

        *self.writer.lock().await = Some(writer);
        *self.service_id.lock().await = service_id;
        *self.conn_id.lock().await = device_id.clone();

        tracing::info!(
            conn_id = %device_id,
            service_id,
            "feishu ws: connected to {}",
            parsed.host_str().unwrap_or("?")
        );

        self.spawn_receive_loop(reader);
        Ok(())
    }

    async fn disconnect(&self) {
        let mut w = self.writer.lock().await;
        if let Some(ref mut writer) = *w {
            let _ = writer.close().await;
        }
        *w = None;
        *self.conn_id.lock().await = String::new();
        tracing::info!("feishu ws: disconnected");
    }

    async fn reconnect(self: &Arc<Self>) -> anyhow::Result<()> {
        let nonce = *self.reconnect_nonce.lock().await;
        if nonce > 0 {
            let jitter = rand::random::<u64>() % (nonce * 1000);
            tokio::time::sleep(Duration::from_millis(jitter)).await;
        }

        let max_retries = *self.reconnect_count.lock().await;
        let interval = Duration::from_secs(*self.reconnect_interval.lock().await);
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            tracing::info!(attempt, "feishu ws: reconnecting...");

            self.disconnect().await;
            match self.connect().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "feishu ws: reconnect failed");
                    if max_retries >= 0 && attempt as i32 >= max_retries {
                        anyhow::bail!(
                            "feishu ws: unable to connect after {} retries: {}",
                            max_retries,
                            e
                        );
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = self.shutdown.notified() => {
                    return Ok(());
                }
                _ = self.cancel.cancelled() => {
                    return Ok(());
                }
            }
        }
    }

    async fn apply_config(&self, conf: &ClientConfig) {
        if conf.ping_interval > 0 {
            *self.ping_interval.lock().await = conf.ping_interval;
        }
        if conf.reconnect_interval > 0 {
            *self.reconnect_interval.lock().await = conf.reconnect_interval;
        }
        *self.reconnect_nonce.lock().await = conf.reconnect_nonce;
        *self.reconnect_count.lock().await = conf.reconnect_count;
    }

    // -----------------------------------------------------------------------
    // Ping loop
    // -----------------------------------------------------------------------

    fn spawn_ping_loop(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let interval = *this.ping_interval.lock().await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                    _ = this.shutdown.notified() => break,
                    _ = this.cancel.cancelled() => break,
                }

                let sid = *this.service_id.lock().await;
                let ping = Frame::new_ping(sid);
                if let Err(e) = this.write_frame(&ping).await {
                    tracing::warn!(error = %e, "feishu ws: ping failed");
                } else {
                    tracing::debug!("feishu ws: ping sent");
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Receive loop
    // -----------------------------------------------------------------------

    fn spawn_receive_loop(self: &Arc<Self>, reader: WsReader) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = this.receive_loop(reader).await {
                tracing::error!(error = %e, "feishu ws: receive loop ended");
            }
            this.disconnect().await;
            if this.cancel.is_cancelled() {
                tracing::info!("feishu ws: receive loop exit, stop requested");
                return;
            }
            if let Err(e) = this.reconnect().await {
                tracing::error!(error = %e, "feishu ws: reconnect failed after receive loop exit");
            }
        });
    }

    async fn receive_loop(&self, mut reader: WsReader) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                msg = reader.next() => {
                    match msg {
                        Some(Ok(WsMessage::Binary(data))) => {
                            if let Err(e) = self.handle_message(&data).await {
                                tracing::warn!(error = %e, "feishu ws: handle message error");
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            tracing::info!("feishu ws: server closed connection");
                            break;
                        }
                        Some(Ok(_)) => {} // ignore text, ping, pong at ws level
                        Some(Err(e)) => {
                            tracing::error!(error = %e, "feishu ws: read error");
                            break;
                        }
                        None => break,
                    }
                }
                _ = self.cancel.cancelled() => {
                    tracing::info!("feishu ws: receive loop cancelled");
                    break;
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Message handling
    // -----------------------------------------------------------------------

    async fn handle_message(&self, data: &[u8]) -> anyhow::Result<()> {
        let frame = Frame::decode(data)?;
        match frame.method {
            FRAME_TYPE_CONTROL => self.handle_control_frame(&frame).await,
            FRAME_TYPE_DATA => self.handle_data_frame(frame).await,
            other => {
                tracing::debug!(method = other, "feishu ws: unknown frame type");
                Ok(())
            }
        }
    }

    async fn handle_control_frame(&self, frame: &Frame) -> anyhow::Result<()> {
        let msg_type = frame.get_header(HEADER_TYPE).unwrap_or("");
        if msg_type == MSG_TYPE_PONG {
            tracing::debug!("feishu ws: received pong");
            if !frame.payload.is_empty() {
                if let Ok(conf) = serde_json::from_slice::<ClientConfig>(&frame.payload) {
                    self.apply_config(&conf).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_data_frame(&self, mut frame: Frame) -> anyhow::Result<()> {
        let msg_id = frame
            .get_header(HEADER_MESSAGE_ID)
            .unwrap_or("")
            .to_string();
        let trace_id = frame.get_header(HEADER_TRACE_ID).unwrap_or("").to_string();
        let sum: usize = frame
            .get_header(HEADER_SUM)
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let seq: usize = frame
            .get_header(HEADER_SEQ)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let msg_type = frame.get_header(HEADER_TYPE).unwrap_or("").to_string();

        let payload = if sum > 1 {
            match self.combine(&msg_id, sum, seq, &frame.payload).await {
                Some(combined) => combined,
                None => return Ok(()), // still waiting for more fragments
            }
        } else {
            std::mem::take(&mut frame.payload)
        };

        tracing::debug!(
            msg_type = %msg_type,
            msg_id = %msg_id,
            trace_id = %trace_id,
            payload_len = payload.len(),
            "feishu ws: received data frame"
        );

        // Immediately ACK the frame (Feishu expects response within 3s)
        let ack_payload = serde_json::to_vec(&serde_json::json!({"code": 200}))?;
        frame.payload = ack_payload;
        self.write_frame(&frame).await.ok();

        // Dispatch event asynchronously
        if msg_type == MSG_TYPE_EVENT {
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            if let Some(tx) = self.event_tx.lock().await.as_ref() {
                let _ = tx.send(WsEvent {
                    message_type: msg_type,
                    message_id: msg_id,
                    trace_id,
                    payload,
                });
            }
        }

        Ok(())
    }

    /// Reassemble fragmented messages (multi-frame payloads).
    async fn combine(&self, msg_id: &str, sum: usize, seq: usize, data: &[u8]) -> Option<Vec<u8>> {
        let mut cache = self.fragment_cache.lock().await;
        let buf = cache
            .entry(msg_id.to_string())
            .or_insert_with(|| vec![None; sum]);

        if seq < buf.len() {
            buf[seq] = Some(data.to_vec());
        }

        if buf.iter().all(|slot| slot.is_some()) {
            let combined: Vec<u8> = buf
                .iter()
                .filter_map(|slot| slot.as_ref())
                .flatten()
                .copied()
                .collect();
            cache.remove(msg_id);
            Some(combined)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Write helpers
    // -----------------------------------------------------------------------

    async fn write_frame(&self, frame: &Frame) -> anyhow::Result<()> {
        let data = frame.encode_to_vec();
        let mut w = self.writer.lock().await;
        match w.as_mut() {
            Some(writer) => {
                writer.send(WsMessage::Binary(data)).await?;
                Ok(())
            }
            None => anyhow::bail!("feishu ws: not connected"),
        }
    }
}
