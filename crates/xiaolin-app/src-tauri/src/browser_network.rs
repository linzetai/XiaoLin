//! Browser network configuration: built-in proxy, host mappings, agent confirm flow.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock, oneshot};
use uuid::Uuid;
use xiaolin_network_proxy::{
    BrowserNetworkConfig, BrowserProxyMode, HostMapping, NetworkMode, NetworkProxy,
    NetworkProxyBuilder, NetworkProxyHandle, NetworkProxySettings, NetworkProxyState, load_config,
    save_config,
};

use crate::browser_panel::BrowserPanelState;
use xiaolin_tools_browser::{broadcast_network_event, BrowserNetworkBridge};

const CONFIRM_TIMEOUT_SECS: u64 = 30;
const MAX_PENDING_CONFIRMS: usize = 32;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostMappingPayload {
    pub pattern: String,
    pub target_ip: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConfirmRequest {
    pub request_id: String,
    pub kind: String,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mappings: Option<Vec<HostMappingPayload>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    pub expires_at: u64,
}

pub struct BrowserNetworkState {
    inner: Arc<BrowserNetworkManager>,
}

impl BrowserNetworkState {
    pub async fn new(app: AppHandle) -> Result<Self, String> {
        let inner = BrowserNetworkManager::new(app).await?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub fn manager(&self) -> &Arc<BrowserNetworkManager> {
        &self.inner
    }
}

pub struct BrowserNetworkManager {
    config: Arc<RwLock<BrowserNetworkConfig>>,
    proxy: Arc<NetworkProxy>,
    _proxy_handle: NetworkProxyHandle,
    proxy_state: Arc<NetworkProxyState>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    cached_webview_proxy: Arc<std::sync::RwLock<Option<String>>>,
    cached_webview_proxy_setting: Arc<std::sync::RwLock<WebviewProxySetting>>,
    app: AppHandle,
}

impl BrowserNetworkManager {
    pub async fn new(app: AppHandle) -> Result<Self, String> {
        let config = load_config(None);
        config.validate()?;

        let settings = NetworkProxySettings {
            enabled: true,
            mode: Some(NetworkMode::Full),
            proxy_url: config.upstream_proxy_url.clone(),
            ..Default::default()
        };

        let proxy_state = Arc::new(NetworkProxyState::for_settings(settings));
        proxy_state
            .set_host_mappings(config.effective_host_mappings())
            .await;

        let (proxy, handle) = NetworkProxyBuilder::new()
            .with_state(Arc::clone(&proxy_state))
            .build()
            .await
            .map_err(|e| format!("failed to start browser network proxy: {e}"))?;

        let proxy = Arc::new(proxy);
        tracing::info!(
            http_addr = %proxy.http_addr(),
            "browser network proxy started"
        );

        let cached_webview_proxy = Arc::new(std::sync::RwLock::new(None));
        let cached_webview_proxy_setting =
            Arc::new(std::sync::RwLock::new(WebviewProxySetting::Direct));
        let manager = Self {
            config: Arc::new(RwLock::new(config.clone())),
            proxy,
            _proxy_handle: handle,
            proxy_state,
            pending: Arc::new(Mutex::new(HashMap::new())),
            cached_webview_proxy,
            cached_webview_proxy_setting,
            app,
        };
        manager.update_proxy_cache(&config);
        Ok(manager)
    }

    fn update_proxy_cache(&self, cfg: &BrowserNetworkConfig) {
        let setting = webview_proxy_setting(cfg, &self.proxy.http_proxy_url());
        if let Ok(mut guard) = self.cached_webview_proxy.write() {
            *guard = setting.proxy_url();
        }
        if let Ok(mut guard) = self.cached_webview_proxy_setting.write() {
            *guard = setting;
        }
    }

    pub fn webview_proxy_url_sync(&self) -> Option<String> {
        self.cached_webview_proxy.read().ok().and_then(|g| g.clone())
    }

    pub(crate) fn webview_proxy_setting_sync(&self) -> WebviewProxySetting {
        self.cached_webview_proxy_setting
            .read()
            .ok()
            .map(|g| g.clone())
            .unwrap_or(WebviewProxySetting::Direct)
    }

    pub fn proxy_http_url(&self) -> String {
        self.proxy.http_proxy_url()
    }

    pub async fn get_config_json(&self) -> Result<String, String> {
        let cfg = self.config.read().await;
        serde_json::to_string(&*cfg)
            .map_err(|e| format!("failed to serialize network config: {e}"))
    }

    pub async fn save_user_config(&self, config: BrowserNetworkConfig) -> Result<(), String> {
        config.validate()?;
        self.apply_config(config).await?;
        let cfg = self.config.read().await.clone();
        save_config(&cfg, None)?;
        Ok(())
    }

    pub async fn webview_proxy_url(&self) -> Option<String> {
        let cfg = self.config.read().await;
        webview_proxy_for_mode(&cfg, &self.proxy.http_proxy_url())
    }

    pub async fn resolve_confirm(&self, request_id: &str, approved: bool) -> Result<(), String> {
        let tx = self.pending.lock().await.remove(request_id);
        match tx {
            Some(sender) => {
                let _ = sender.send(approved);
                Ok(())
            }
            None => Err("confirm request not found or already resolved".to_string()),
        }
    }

    async fn apply_config(&self, config: BrowserNetworkConfig) -> Result<(), String> {
        config.validate()?;
        self.proxy_state
            .set_host_mappings(config.effective_host_mappings())
            .await;
        self.proxy_state
            .replace_upstream_proxy_url(config.upstream_proxy_url.clone())
            .await;
        self.update_proxy_cache(&config);
        *self.config.write().await = config.clone();
        reconfigure_open_webview_proxies(&self.app, &config, &self.proxy.http_proxy_url());
        let _ = self.app.emit("browser-network-config-changed", ());
        Ok(())
    }

    async fn wait_for_confirm(&self, request: NetworkConfirmRequest) -> bool {
        let request_id = request.request_id.clone();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            if pending.len() >= MAX_PENDING_CONFIRMS {
                tracing::warn!(
                    pending = pending.len(),
                    max = MAX_PENDING_CONFIRMS,
                    "browser network confirm queue full"
                );
                return false;
            }
            pending.insert(request_id.clone(), tx);
        }

        self.emit_confirm_request(&request);

        let approved = match tokio::time::timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS), rx).await
        {
            Ok(Ok(v)) => v,
            _ => false,
        };

        self.pending.lock().await.remove(&request_id);
        approved
    }

    fn emit_confirm_request(&self, request: &NetworkConfirmRequest) {
        let payload = serde_json::to_value(request).unwrap_or_default();
        let _ = self
            .app
            .emit("browser-network-confirm-request", payload.clone());
        broadcast_network_event("browser_network_confirm", payload);
    }

    fn mappings_to_host(mappings: &[(String, String)]) -> Vec<HostMapping> {
        mappings
            .iter()
            .map(|(pattern, target_ip)| HostMapping::new(pattern.clone(), target_ip.clone()))
            .collect()
    }
}

fn webview_proxy_for_mode(cfg: &BrowserNetworkConfig, xiaolin_url: &str) -> Option<String> {
    webview_proxy_setting(cfg, xiaolin_url).proxy_url()
}

/// WebView-side proxy mode (distinct from built-in XiaoLin proxy host mappings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebviewProxySetting {
    /// Direct connection — no WebView proxy.
    Direct,
    /// Follow OS / environment proxy settings.
    System,
    /// Route through the given proxy URL.
    Custom(String),
}

impl WebviewProxySetting {
    pub fn proxy_url(&self) -> Option<String> {
        match self {
            Self::Custom(url) => Some(url.clone()),
            Self::Direct | Self::System => None,
        }
    }
}

fn webview_proxy_setting(cfg: &BrowserNetworkConfig, xiaolin_url: &str) -> WebviewProxySetting {
    match cfg.proxy_mode {
        BrowserProxyMode::None => WebviewProxySetting::Direct,
        BrowserProxyMode::System => WebviewProxySetting::System,
        BrowserProxyMode::Custom => cfg
            .custom_proxy_url
            .clone()
            .map(WebviewProxySetting::Custom)
            .unwrap_or(WebviewProxySetting::Direct),
        BrowserProxyMode::XiaolinProxy => {
            WebviewProxySetting::Custom(xiaolin_url.to_string())
        }
    }
}

/// Re-apply WebView proxy settings to every open browser page after network config changes.
fn reconfigure_open_webview_proxies(
    app: &AppHandle,
    cfg: &BrowserNetworkConfig,
    xiaolin_url: &str,
) {
    let setting = webview_proxy_setting(cfg, xiaolin_url);
    let labels: Vec<String> = match app.try_state::<BrowserPanelState>() {
        Some(state) => match state.0.lock() {
            Ok(guard) => guard.webview_labels(),
            Err(e) => {
                tracing::warn!(error = %e, "reconfigure webview proxy: browser panel lock poisoned");
                return;
            }
        },
        None => return,
    };

    if labels.is_empty() {
        return;
    }

    let page_count = labels.len();
    let setting_for_log = setting.clone();

    #[cfg(target_os = "linux")]
    {
        let Some(window) = app.get_window("main") else {
            tracing::warn!("reconfigure webview proxy: main window not found");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        if window
            .run_on_main_thread(move || {
                for label in &labels {
                    crate::browser_gtk::reapply_webview_proxy(label, &setting);
                }
                let _ = tx.send(());
            })
            .is_err()
        {
            tracing::warn!("reconfigure webview proxy: failed to dispatch GTK main thread");
            return;
        }
        if rx.recv().is_err() {
            tracing::warn!("reconfigure webview proxy: GTK channel closed");
        }
        tracing::info!(
            pages = page_count,
            proxy = ?setting_for_log,
            "reconfigured proxy on open browser webviews"
        );
    }

    // TODO(platform): macOS/Windows lack runtime WebView proxy API.
    // Tauri/wry only supports proxy at WebViewBuilder time. Investigate
    // platform-specific runtime APIs (WKWebView URLSessionConfiguration,
    // WebView2 put_ProxySettings) to enable hot-reload on non-Linux.
    #[cfg(not(target_os = "linux"))]
    {
        tracing::info!(
            pages = page_count,
            proxy = ?setting_for_log,
            "network config changed; reload open browser pages to apply WebView proxy on this platform"
        );
    }
}

struct TauriNetworkBridge {
    manager: Arc<BrowserNetworkManager>,
}

#[async_trait]
impl BrowserNetworkBridge for TauriNetworkBridge {
    async fn get_network_config(&self) -> Result<String, String> {
        self.manager.get_config_json().await
    }

    async fn set_hosts(
        &self,
        mappings: Vec<(String, String)>,
        temporary: bool,
        reason: Option<&str>,
        require_confirm: bool,
    ) -> Result<String, String> {
        let host_mappings = BrowserNetworkManager::mappings_to_host(&mappings);
        for m in &host_mappings {
            xiaolin_network_proxy::validate_host_mapping(m)?;
        }

        if require_confirm {
            let mapping_payloads: Vec<HostMappingPayload> = mappings
                .iter()
                .map(|(pattern, target_ip)| HostMappingPayload {
                    pattern: pattern.clone(),
                    target_ip: target_ip.clone(),
                })
                .collect();
            let confirmed_mappings = host_mappings.clone();

            let request = NetworkConfirmRequest {
                request_id: Uuid::new_v4().to_string(),
                kind: "set_hosts".to_string(),
                reason: reason.map(str::to_string),
                mappings: Some(mapping_payloads),
                proxy_url: None,
                expires_at: chrono::Utc::now().timestamp_millis() as u64
                    + CONFIRM_TIMEOUT_SECS * 1000,
            };
            if !self.manager.wait_for_confirm(request).await {
                return Ok(
                    serde_json::json!({ "ok": false, "reason": "user_denied_or_timeout" })
                        .to_string(),
                );
            }

            let mut cfg = self.manager.config.read().await.clone();
            if temporary {
                cfg.set_session_hosts(confirmed_mappings)?;
            } else {
                cfg.set_persistent_hosts(confirmed_mappings)?;
            }
            self.manager.apply_config(cfg).await?;
            let cfg = self.manager.config.read().await.clone();
            save_config(&cfg, None)?;
            return Ok(serde_json::json!({ "ok": true, "temporary": temporary }).to_string());
        }

        let mut cfg = self.manager.config.read().await.clone();
        if temporary {
            cfg.set_session_hosts(host_mappings)?;
        } else {
            cfg.set_persistent_hosts(host_mappings)?;
        }
        self.manager.apply_config(cfg).await?;
        let cfg = self.manager.config.read().await.clone();
        save_config(&cfg, None)?;
        Ok(serde_json::json!({ "ok": true, "temporary": temporary }).to_string())
    }

    async fn set_proxy(
        &self,
        proxy_url: Option<&str>,
        reason: Option<&str>,
        require_confirm: bool,
    ) -> Result<String, String> {
        if let Some(url) = proxy_url {
            xiaolin_network_proxy::validate_proxy_url(url)?;
        }

        if require_confirm {
            let request = NetworkConfirmRequest {
                request_id: Uuid::new_v4().to_string(),
                kind: "set_proxy".to_string(),
                reason: reason.map(str::to_string),
                mappings: None,
                proxy_url: proxy_url.map(str::to_string),
                expires_at: chrono::Utc::now().timestamp_millis() as u64
                    + CONFIRM_TIMEOUT_SECS * 1000,
            };
            if !self.manager.wait_for_confirm(request).await {
                return Ok(
                    serde_json::json!({ "ok": false, "reason": "user_denied_or_timeout" })
                        .to_string(),
                );
            }
        }

        let mut cfg = self.manager.config.read().await.clone();
        cfg.upstream_proxy_url = proxy_url.map(str::to_string);
        self.manager.apply_config(cfg).await?;
        let cfg = self.manager.config.read().await.clone();
        save_config(&cfg, None)?;
        Ok(serde_json::json!({ "ok": true }).to_string())
    }

    async fn clear_hosts(&self, temporary_only: bool) -> Result<String, String> {
        let mut cfg = self.manager.config.read().await.clone();
        if temporary_only {
            cfg.clear_session_hosts();
        } else {
            cfg.host_mappings.clear();
            cfg.clear_session_hosts();
        }
        self.manager.apply_config(cfg).await?;
        let cfg = self.manager.config.read().await.clone();
        save_config(&cfg, None)?;
        Ok(serde_json::json!({ "ok": true }).to_string())
    }
}

/// Start browser network manager, register bridge, and expose Tauri state.
pub async fn install_browser_network(app: &AppHandle) -> Result<(), String> {
    let state = BrowserNetworkState::new(app.clone()).await?;
    let bridge = Arc::new(TauriNetworkBridge {
        manager: Arc::clone(state.manager()),
    });
    if let Err(existing) = xiaolin_tools_browser::set_browser_network_bridge(bridge) {
        tracing::warn!("browser network bridge already registered");
        drop(existing);
    } else {
        tracing::info!("browser network bridge registered");
    }
    app.manage(state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_network_proxy::BrowserNetworkConfig;

    #[test]
    fn webview_proxy_setting_modes() {
        let mut cfg = BrowserNetworkConfig::default();
        cfg.proxy_mode = BrowserProxyMode::None;
        assert_eq!(
            webview_proxy_setting(&cfg, "http://127.0.0.1:1"),
            WebviewProxySetting::Direct
        );

        cfg.proxy_mode = BrowserProxyMode::System;
        assert_eq!(
            webview_proxy_setting(&cfg, "http://127.0.0.1:1"),
            WebviewProxySetting::System
        );

        cfg.proxy_mode = BrowserProxyMode::Custom;
        cfg.custom_proxy_url = Some("http://proxy:8080".into());
        assert_eq!(
            webview_proxy_setting(&cfg, "http://127.0.0.1:1"),
            WebviewProxySetting::Custom("http://proxy:8080".into())
        );

        cfg.proxy_mode = BrowserProxyMode::XiaolinProxy;
        assert_eq!(
            webview_proxy_setting(&cfg, "http://127.0.0.1:9999"),
            WebviewProxySetting::Custom("http://127.0.0.1:9999".into())
        );
    }
}
