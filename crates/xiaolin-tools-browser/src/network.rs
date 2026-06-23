//! Browser network tool actions (host mapping, proxy config).

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::broadcast;

use crate::engine::EngineActionResult;

/// Bridge for browser network configuration (implemented by Tauri app).
#[async_trait]
pub trait BrowserNetworkBridge: Send + Sync {
    async fn get_network_config(&self) -> Result<String, String>;
    async fn set_hosts(
        &self,
        mappings: Vec<(String, String)>,
        temporary: bool,
        reason: Option<&str>,
        require_confirm: bool,
    ) -> Result<String, String>;
    async fn set_proxy(
        &self,
        proxy_url: Option<&str>,
        reason: Option<&str>,
        require_confirm: bool,
    ) -> Result<String, String>;
    async fn clear_hosts(&self, temporary_only: bool) -> Result<String, String>;
}

static NETWORK_BRIDGE: OnceLock<Arc<dyn BrowserNetworkBridge>> = OnceLock::new();
static WS_BROADCAST: OnceLock<broadcast::Sender<String>> = OnceLock::new();

pub fn set_browser_network_bridge(
    bridge: Arc<dyn BrowserNetworkBridge>,
) -> Result<(), Arc<dyn BrowserNetworkBridge>> {
    NETWORK_BRIDGE.set(bridge)
}

/// Register gateway WS broadcast sender for browser network confirm events.
pub fn set_network_ws_broadcast(tx: broadcast::Sender<String>) {
    let _ = WS_BROADCAST.set(tx);
}

pub fn broadcast_network_event(event_type: &str, data: serde_json::Value) {
    if let Some(tx) = WS_BROADCAST.get() {
        let msg = json!({ "type": event_type, "data": data }).to_string();
        let _ = tx.send(msg);
    }
}

pub fn network_bridge_configured() -> bool {
    NETWORK_BRIDGE.get().is_some()
}

fn bridge() -> Result<&'static Arc<dyn BrowserNetworkBridge>, String> {
    NETWORK_BRIDGE
        .get()
        .ok_or_else(network_unavailable_message)
}

pub fn validate_network_action(action: &str, args: &serde_json::Value) -> Result<(), String> {
    match action {
        "set_hosts" => {
            let mappings = parse_host_mappings(args)?;
            if mappings.is_empty() {
                return Err(
                    "browser set_hosts: provide 'mappings' array of {pattern, target_ip} objects."
                        .to_string(),
                );
            }
            for (pattern, ip) in &mappings {
                xiaolin_network_proxy::validate_host_mapping(
                    &xiaolin_network_proxy::HostMapping::new(pattern.clone(), ip.clone()),
                )?;
            }
        }
        "set_proxy" => {
            if let Some(url) = args.get("proxy_url").and_then(|v| v.as_str()) {
                xiaolin_network_proxy::validate_proxy_url(url)?;
            }
        }
        "get_network_config" | "clear_hosts" => {}
        _ => {}
    }
    Ok(())
}

fn parse_host_mappings(args: &serde_json::Value) -> Result<Vec<(String, String)>, String> {
    let arr = args
        .get("mappings")
        .or(args.get("hosts"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            "browser set_hosts: missing 'mappings' array. Example: [{\"pattern\":\"api.dev.com\",\"target_ip\":\"192.168.1.1\"}]".to_string()
        })?;
    let mut out = Vec::new();
    for item in arr {
        let pattern = item
            .get("pattern")
            .or(item.get("host"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "each mapping needs 'pattern' (or 'host')".to_string())?;
        let target_ip = item
            .get("target_ip")
            .or(item.get("ip"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "each mapping needs 'target_ip' (or 'ip')".to_string())?;
        out.push((pattern.to_string(), target_ip.to_string()));
    }
    Ok(out)
}

pub async fn execute_network_action(
    action: &str,
    args: &serde_json::Value,
) -> Result<EngineActionResult, String> {
    validate_network_action(action, args)?;
    let bridge = bridge()?;
    let require_confirm = args
        .get("require_confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let reason = args.get("reason").and_then(|v| v.as_str());

    match action {
        "get_network_config" => {
            let raw = bridge.get_network_config().await?;
            Ok(EngineActionResult::text(raw))
        }
        "set_hosts" => {
            let mappings = parse_host_mappings(args)?;
            let temporary = args
                .get("temporary")
                .and_then(|v| v.as_bool())
                .unwrap_or(require_confirm);
            let raw = bridge
                .set_hosts(mappings, temporary, reason, require_confirm)
                .await?;
            Ok(EngineActionResult::text(raw))
        }
        "set_proxy" => {
            let proxy_url = args.get("proxy_url").and_then(|v| v.as_str());
            let raw = bridge.set_proxy(proxy_url, reason, require_confirm).await?;
            Ok(EngineActionResult::text(raw))
        }
        "clear_hosts" => {
            let temporary_only = args
                .get("temporary_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let raw = bridge.clear_hosts(temporary_only).await?;
            Ok(EngineActionResult::text(raw))
        }
        other => Err(format!("browser network: unknown action '{other}'")),
    }
}

pub fn network_unavailable_message() -> String {
    json!({
        "error": "network_config_unavailable",
        "message": "Browser network configuration requires the Tauri desktop app with built-in proxy.",
        "hint": "Use Settings → Browser → Network to configure host mappings manually."
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockNetworkBridge;

    #[async_trait]
    impl BrowserNetworkBridge for MockNetworkBridge {
        async fn get_network_config(&self) -> Result<String, String> {
            Ok(r#"{"proxy_mode":"xiaolin_proxy"}"#.to_string())
        }
        async fn set_hosts(
            &self,
            _mappings: Vec<(String, String)>,
            _temporary: bool,
            _reason: Option<&str>,
            _require_confirm: bool,
        ) -> Result<String, String> {
            Ok(r#"{"ok":true}"#.to_string())
        }
        async fn set_proxy(
            &self,
            _proxy_url: Option<&str>,
            _reason: Option<&str>,
            _require_confirm: bool,
        ) -> Result<String, String> {
            Ok(r#"{"ok":true}"#.to_string())
        }
        async fn clear_hosts(&self, _temporary_only: bool) -> Result<String, String> {
            Ok(r#"{"ok":true}"#.to_string())
        }
    }

    #[test]
    fn validate_rejects_loopback_mapping() {
        let args = json!({
            "mappings": [{ "pattern": "bank.com", "target_ip": "127.0.0.1" }]
        });
        let err = validate_network_action("set_hosts", &args).unwrap_err();
        assert!(err.contains("loopback"));
    }

    #[test]
    fn parse_host_mappings_accepts_aliases() {
        let args = json!({
            "hosts": [{ "host": "a.com", "ip": "10.0.0.1" }]
        });
        let m = parse_host_mappings(&args).unwrap();
        assert_eq!(m, vec![("a.com".to_string(), "10.0.0.1".to_string())]);
    }

    #[tokio::test]
    async fn execute_get_config() {
        let _ = set_browser_network_bridge(Arc::new(MockNetworkBridge));
        let result = execute_network_action("get_network_config", &json!({}))
            .await
            .unwrap();
        assert!(result.text.contains("xiaolin"));
    }
}
