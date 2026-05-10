//! JSON-RPC 2.0 dispatcher for process-based plugins.
//!
//! Provides a generic JSON-RPC client that communicates with external processes
//! over stdio. Supports request/response matching via IDs, notifications, and
//! background message handling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};

/// JSON-RPC 2.0 request.
#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Deserialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

/// JSON-RPC 2.0 notification (no id).
#[derive(Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// Handle to a spawned process with JSON-RPC communication.
struct ProcessHandle {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    child: tokio::process::Child,
}

/// Shared inner state of the JSON-RPC dispatcher.
struct DispatcherInner {
    plugin_id: String,
    process: Mutex<Option<ProcessHandle>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<serde_json::Value, JsonRpcError>>>>,
}

/// Dispatcher for JSON-RPC communication with a process plugin.
///
/// Manages request IDs, pending responses, and background notification handling.
/// Clonable — all clones share the same process and pending map.
pub struct JsonRpcDispatcher {
    inner: Arc<DispatcherInner>,
    notification_tx: Arc<Mutex<Option<mpsc::UnboundedSender<JsonRpcNotification>>>>,
}

impl Clone for JsonRpcDispatcher {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            notification_tx: self.notification_tx.clone(),
        }
    }
}

impl JsonRpcDispatcher {
    /// Create a new dispatcher for the given plugin.
    pub fn new(plugin_id: &str) -> Self {
        Self {
            inner: Arc::new(DispatcherInner {
                plugin_id: plugin_id.to_string(),
                process: Mutex::new(None),
                next_id: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
            }),
            notification_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the notification channel for receiving async notifications from the plugin.
    pub async fn set_notification_channel(&self, tx: mpsc::UnboundedSender<JsonRpcNotification>) {
        let mut guard = self.notification_tx.lock().await;
        *guard = Some(tx);
    }

    /// Spawn the plugin process if not already running.
    pub async fn ensure_process(
        &self,
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let mut guard = self.inner.process.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        tracing::info!(
            plugin_id = %self.inner.plugin_id,
            command = %command,
            "spawning plugin process"
        );

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            anyhow!("failed to spawn plugin process '{}': {e}", command)
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("plugin process stdin not captured"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("plugin process stdout not captured"))?;

        *guard = Some(ProcessHandle {
            stdin,
            stdout: tokio::io::BufReader::new(stdout),
            child,
        });
        Ok(())
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);

        // Create response channel before sending.
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            pending.insert(id, tx);
        }

        // Send the request.
        self.send_request(id, method, params).await?;

        // Wait for the response.
        let result = rx
            .await
            .map_err(|_| anyhow!("response channel closed for request {}", id))??;

        Ok(result)
    }

    /// Send a JSON-RPC request without waiting for a response (fire-and-forget).
    pub async fn notify(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<()> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        self.send_request(id, method, params).await
    }

    async fn send_request(
        &self,
        id: u64,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut guard = self.inner.process.lock().await;
        let handle = guard
            .as_mut()
            .ok_or_else(|| anyhow!("plugin process not running"))?;

        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        handle
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| anyhow!("failed to write to plugin stdin: {e}"))?;
        handle.stdin.flush().await?;

        Ok(())
    }

    /// Start a background task to read responses and notifications from the process.
    ///
    /// This must be called after `ensure_process`. The task runs until the process
    /// exits or encounters an error.
    pub fn start_reader(&self) {
        let inner = self.inner.clone();
        let notification_tx = self.notification_tx.clone();

        tokio::spawn(async move {
            let mut line_count: u64 = 0;

            loop {
                let mut guard = inner.process.lock().await;
                let handle = match guard.as_mut() {
                    Some(h) => h,
                    None => {
                        tracing::debug!(
                            plugin_id = %inner.plugin_id,
                            "plugin reader: process handle gone, stopping"
                        );
                        break;
                    }
                };

                let mut line = String::new();
                match handle.stdout.read_line(&mut line).await {
                    Ok(0) => {
                        tracing::info!(
                            plugin_id = %inner.plugin_id,
                            line_count,
                            "plugin reader: EOF from process"
                        );
                        break;
                    }
                    Ok(_n) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        line_count += 1;

                        match parse_message(trimmed) {
                            Ok(Message::Response(resp)) => {
                                if let Some(id) = resp.id {
                                    let mut pending_guard = inner.pending.lock().await;
                                    if let Some(tx) = pending_guard.remove(&id) {
                                        let result = if let Some(err) = resp.error {
                                            Err(err)
                                        } else {
                                            Ok(resp.result.unwrap_or(serde_json::Value::Null))
                                        };
                                        let _ = tx.send(result);
                                    } else {
                                        tracing::warn!(
                                            plugin_id = %inner.plugin_id,
                                            id,
                                            "plugin reader: received response for unknown request id"
                                        );
                                    }
                                }
                            }
                            Ok(Message::Notification(notif)) => {
                                let tx_guard = notification_tx.lock().await;
                                if let Some(ref tx) = *tx_guard {
                                    let _ = tx.send(notif);
                                } else {
                                    tracing::debug!(
                                        plugin_id = %inner.plugin_id,
                                        "plugin reader: received notification but no handler"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    plugin_id = %inner.plugin_id,
                                    error = %e,
                                    line_preview = &trimmed[..trimmed.len().min(200)],
                                    "plugin reader: failed to parse message"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            plugin_id = %inner.plugin_id,
                            line_count,
                            error = %e,
                            "plugin reader: stdout read error"
                        );
                        break;
                    }
                }
            }

            tracing::info!(
                plugin_id = %inner.plugin_id,
                line_count,
                "plugin reader task finished"
            );
        });
    }

    /// Check if the process is running.
    pub async fn is_running(&self) -> bool {
        let guard = self.inner.process.lock().await;
        guard.is_some()
    }

    /// Kill the process and clean up.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let mut guard = self.inner.process.lock().await;
        if let Some(mut handle) = guard.take() {
            tracing::info!(plugin_id = %self.inner.plugin_id, "killing plugin process");
            let _ = handle.child.kill().await;
        }
        Ok(())
    }

    /// Return the plugin ID.
    pub fn plugin_id(&self) -> &str {
        &self.inner.plugin_id
    }
}

enum Message {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// Parse a JSON line as either a response or notification.
fn parse_message(line: &str) -> anyhow::Result<Message> {
    let v: serde_json::Value = serde_json::from_str(line)?;

    // If it has an "id" field, it's a response.
    if v.get("id").is_some() {
        let resp: JsonRpcResponse = serde_json::from_value(v)?;
        return Ok(Message::Response(resp));
    }

    // Otherwise, treat it as a notification.
    let notif: JsonRpcNotification = serde_json::from_value(v)?;
    Ok(Message::Notification(notif))
}
