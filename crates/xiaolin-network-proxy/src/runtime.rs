use std::collections::VecDeque;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use globset::GlobSet;
use tokio::sync::RwLock;

use crate::config::{NetworkMode, NetworkProxySettings};
use crate::mitm::MitmState;
use crate::network_policy::{
    HostBlockDecision, HostBlockReason, NetworkDecision, NetworkDecisionSource,
    NetworkPolicyRequest,
};

const MAX_BLOCKED_EVENTS: usize = 200;

/// Metadata attached to proxy audit events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkProxyAuditMetadata {
    pub conversation_id: Option<String>,
    pub app_version: Option<String>,
    pub auth_mode: Option<String>,
    pub originator: Option<String>,
    pub user_account_id: Option<String>,
    pub user_email: Option<String>,
    pub terminal_type: Option<String>,
    pub model: Option<String>,
    pub slug: Option<String>,
}

/// A request that was blocked by the proxy.
#[derive(Debug, Clone)]
pub struct BlockedRequest {
    pub host: String,
    pub reason: String,
    pub client: Option<String>,
    pub method: Option<String>,
    pub protocol: String,
    pub port: Option<u16>,
}

/// Arguments for constructing a `BlockedRequest`.
pub struct BlockedRequestArgs {
    pub host: String,
    pub reason: String,
    pub client: Option<String>,
    pub method: Option<String>,
    pub protocol: String,
    pub port: Option<u16>,
}

/// Observer for blocked requests — implement this to receive notifications.
pub trait BlockedRequestObserver: Send + Sync + 'static {
    fn on_blocked_request(
        &self,
        request: BlockedRequest,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Derived proxy configuration state from `NetworkProxySettings`.
#[derive(Debug, Clone)]
pub struct ConfigState {
    pub settings: NetworkProxySettings,
    pub http_addr: Option<SocketAddr>,
    pub socks_addr: Option<SocketAddr>,
}

impl ConfigState {
    pub fn from_settings(settings: NetworkProxySettings) -> Self {
        let http_addr = settings
            .proxy_url
            .as_deref()
            .and_then(|url| url.parse::<SocketAddr>().ok());
        let socks_addr = settings
            .socks_url
            .as_deref()
            .and_then(|url| url.parse::<SocketAddr>().ok());
        Self {
            settings,
            http_addr,
            socks_addr,
        }
    }
}

/// Reloads proxy config from an external source.
pub trait ConfigReloader: Send + Sync + 'static {
    fn source_label(&self) -> String {
        "unknown".to_string()
    }

    fn maybe_reload(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<NetworkProxySettings>, String>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    fn reload(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkProxySettings, String>> + Send + '_>>;
}

/// Snapshot of frequently-accessed config fields for proxy engine hot paths.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub enabled: bool,
    pub mode: NetworkMode,
    pub allow_local_binding: bool,
    pub mitm_enabled: bool,
    pub enable_socks5: bool,
    pub allow_globset: Option<GlobSet>,
    pub deny_globset: Option<GlobSet>,
    pub allowed_unix_socket_paths: Vec<String>,
}

impl ConfigSnapshot {
    fn from_settings(settings: &NetworkProxySettings) -> Self {
        let allow_globset = {
            let allowed = settings.allowed_domains();
            if allowed.is_empty() {
                None
            } else {
                let patterns: Vec<String> = allowed.into_iter().map(String::from).collect();
                crate::policy::compile_allowlist_globset(&patterns).ok()
            }
        };
        let deny_globset = {
            let denied = settings.denied_domains();
            if denied.is_empty() {
                None
            } else {
                let patterns: Vec<String> = denied.into_iter().map(String::from).collect();
                crate::policy::compile_denylist_globset(&patterns).ok()
            }
        };
        let allowed_unix_socket_paths = settings
            .allow_unix_sockets()
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        Self {
            enabled: settings.enabled,
            mode: settings.mode.unwrap_or(NetworkMode::Full),
            allow_local_binding: settings.allow_local_binding,
            mitm_enabled: settings.mitm,
            enable_socks5: settings.enable_socks5,
            allow_globset,
            deny_globset,
            allowed_unix_socket_paths,
        }
    }
}

/// Dynamic domain entries added at runtime.
#[derive(Debug, Clone, Default)]
struct DynamicDomains {
    allowed: Vec<String>,
    denied: Vec<String>,
}

/// Shared proxy runtime state used by the HTTP and SOCKS5 proxy engines.
pub struct NetworkProxyState {
    hot: Arc<RwLock<ConfigSnapshot>>,
    state: Arc<RwLock<ConfigState>>,
    reloader: Arc<dyn ConfigReloader>,
    blocked_request_observer: Arc<RwLock<Option<Arc<dyn BlockedRequestObserver>>>>,
    blocked_requests: Arc<RwLock<VecDeque<BlockedRequest>>>,
    audit_metadata: NetworkProxyAuditMetadata,
    network_mode: Arc<RwLock<Option<NetworkMode>>>,
    dynamic_domains: Arc<RwLock<DynamicDomains>>,
    mitm_state: Option<Arc<MitmState>>,
}

impl std::fmt::Debug for NetworkProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkProxyState")
            .field("audit_metadata", &self.audit_metadata)
            .finish_non_exhaustive()
    }
}

impl NetworkProxyState {
    pub fn new(
        config_state: ConfigState,
        reloader: Arc<dyn ConfigReloader>,
        audit_metadata: NetworkProxyAuditMetadata,
    ) -> Self {
        let hot = ConfigSnapshot::from_settings(&config_state.settings);
        Self {
            hot: Arc::new(RwLock::new(hot)),
            state: Arc::new(RwLock::new(config_state)),
            reloader,
            blocked_request_observer: Arc::new(RwLock::new(None)),
            blocked_requests: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_BLOCKED_EVENTS))),
            audit_metadata,
            network_mode: Arc::new(RwLock::new(None)),
            dynamic_domains: Arc::new(RwLock::new(DynamicDomains::default())),
            mitm_state: None,
        }
    }

    pub fn for_settings(settings: NetworkProxySettings) -> Self {
        struct NoopReloader;
        impl ConfigReloader for NoopReloader {
            fn reload(
                &self,
            ) -> Pin<Box<dyn Future<Output = Result<NetworkProxySettings, String>> + Send + '_>>
            {
                Box::pin(async { Ok(NetworkProxySettings::default()) })
            }
        }
        Self::new(
            ConfigState::from_settings(settings),
            Arc::new(NoopReloader),
            NetworkProxyAuditMetadata::default(),
        )
    }

    /// Set the MITM state for this proxy instance.
    pub fn set_mitm_state(&mut self, state: Arc<MitmState>) {
        self.mitm_state = Some(state);
    }

    /// Get the MITM state, if configured.
    pub fn mitm_state(&self) -> Option<&Arc<MitmState>> {
        self.mitm_state.as_ref()
    }

    // ── Snapshot & config ───────────────────────────────────────────────

    pub async fn snapshot(&self) -> ConfigSnapshot {
        self.hot.read().await.clone()
    }

    pub async fn reload_if_needed(&self) -> Result<(), String> {
        match self.reloader.maybe_reload().await? {
            None => Ok(()),
            Some(new_settings) => {
                self.apply_settings(new_settings).await;
                let source = self.reloader.source_label();
                tracing::info!("reloaded proxy config from {source}");
                Ok(())
            }
        }
    }

    pub async fn force_reload(&self) -> Result<(), String> {
        let new_settings = self.reloader.reload().await.map_err(|e| {
            let source = self.reloader.source_label();
            tracing::warn!("failed to reload config from {source}: {e}; keeping previous");
            e
        })?;
        self.apply_settings(new_settings).await;
        let source = self.reloader.source_label();
        tracing::info!("force-reloaded proxy config from {source}");
        Ok(())
    }

    pub async fn replace_config_state(&self, new_settings: NetworkProxySettings) {
        self.apply_settings(new_settings).await;
    }

    async fn apply_settings(&self, new_settings: NetworkProxySettings) {
        let new_hot = ConfigSnapshot::from_settings(&new_settings);
        let new_state = ConfigState::from_settings(new_settings);
        *self.hot.write().await = new_hot;
        *self.state.write().await = new_state;
    }

    pub async fn config_state(&self) -> ConfigState {
        let _ = self.reload_if_needed().await;
        self.state.read().await.clone()
    }

    pub fn audit_metadata(&self) -> &NetworkProxyAuditMetadata {
        &self.audit_metadata
    }

    pub async fn enabled(&self) -> bool {
        self.hot.read().await.enabled
    }

    pub async fn reload_config(&self) -> Result<(), String> {
        self.force_reload().await
    }

    // ── Network mode ────────────────────────────────────────────────────

    pub async fn set_network_mode(&self, mode: NetworkMode) {
        *self.network_mode.write().await = Some(mode);
    }

    pub async fn network_mode(&self) -> NetworkMode {
        if let Some(mode) = *self.network_mode.read().await {
            return mode;
        }
        self.hot
            .read()
            .await
            .mode
    }

    // ── Dynamic domain allow/deny ───────────────────────────────────────

    pub async fn add_allowed_domain(&self, domain: String) {
        self.dynamic_domains.write().await.allowed.push(domain);
    }

    pub async fn add_denied_domain(&self, domain: String) {
        self.dynamic_domains.write().await.denied.push(domain);
    }

    pub async fn dynamic_allowed_domains(&self) -> Vec<String> {
        self.dynamic_domains.read().await.allowed.clone()
    }

    pub async fn dynamic_denied_domains(&self) -> Vec<String> {
        self.dynamic_domains.read().await.denied.clone()
    }

    // ── Unix socket whitelist ───────────────────────────────────────────

    pub async fn is_unix_socket_allowed(&self, path: &Path) -> bool {
        let snapshot = self.hot.read().await;
        let path_str = path.to_string_lossy();
        snapshot
            .allowed_unix_socket_paths
            .iter()
            .any(|allowed| path_str.starts_with(allowed.as_str()))
    }

    // ── Method / upstream / local binding checks ────────────────────────

    pub async fn method_allowed(&self, method: &str) -> bool {
        let mode = self.network_mode().await;
        match mode {
            NetworkMode::Full => true,
            NetworkMode::Limited | NetworkMode::Audit | NetworkMode::Off => {
                matches!(
                    method.to_uppercase().as_str(),
                    "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT" | "PATCH" | "DELETE" | "CONNECT"
                )
            }
        }
    }

    pub async fn allow_upstream_proxy(&self) -> bool {
        self.hot.read().await.allow_local_binding
    }

    pub async fn allow_local_binding(&self) -> bool {
        self.hot.read().await.allow_local_binding
    }

    // ── Host blocking decision chain ────────────────────────────────────

    /// Determine whether a host:port should be blocked.
    ///
    /// Decision chain:
    /// 1. Dynamic denied domains → ExplicitlyDenied
    /// 2. Loopback check → LoopbackBlocked (unless allow_local_binding)
    /// 3. DNS resolution for non-IP hosts → DnsResolutionFailed / ResolvedToLoopback
    /// 4. Dynamic allowed domains → Allowed
    /// 5. Static allow/deny globsets → Allowed / ExplicitlyDenied
    /// 6. Default → NotAllowed
    pub async fn host_blocked(&self, host: &str, _port: u16) -> anyhow::Result<HostBlockDecision> {
        let mode = self.network_mode().await;
        if matches!(mode, NetworkMode::Off) {
            return Ok(HostBlockDecision::Allowed);
        }

        let dynamic = self.dynamic_domains.read().await;
        for pattern in &dynamic.denied {
            if host_matches(pattern, host) {
                return Ok(HostBlockDecision::Blocked(
                    HostBlockReason::ExplicitlyDenied,
                ));
            }
        }

        let snapshot = self.hot.read().await;

        if is_loopback_host(host) && !snapshot.allow_local_binding {
            return Ok(HostBlockDecision::Blocked(
                HostBlockReason::LoopbackBlocked,
            ));
        }

        if !is_ip_address(host) {
            match resolve_host(host).await {
                Ok(addrs) => {
                    if !snapshot.allow_local_binding
                        && addrs.iter().any(|a| a.is_loopback())
                    {
                        return Ok(HostBlockDecision::Blocked(
                            HostBlockReason::ResolvedToLoopback,
                        ));
                    }
                }
                Err(_) => {
                    // DNS failure doesn't block by itself; we continue
                    // to pattern-based checks. Only block if the host
                    // isn't in any allowlist.
                }
            }
        }

        for pattern in &dynamic.allowed {
            if host_matches(pattern, host) {
                return Ok(HostBlockDecision::Allowed);
            }
        }

        if let Some(ref deny_globset) = snapshot.deny_globset {
            if deny_globset.is_match(host) {
                return Ok(HostBlockDecision::Blocked(
                    HostBlockReason::ExplicitlyDenied,
                ));
            }
        }

        if let Some(ref allow_globset) = snapshot.allow_globset {
            if allow_globset.is_match(host) {
                return Ok(HostBlockDecision::Allowed);
            }
            return Ok(HostBlockDecision::Blocked(HostBlockReason::NotAllowed));
        }

        if matches!(mode, NetworkMode::Limited) {
            return Ok(HostBlockDecision::Blocked(HostBlockReason::NotAllowed));
        }

        Ok(HostBlockDecision::Allowed)
    }

    // ── Blocked request recording ───────────────────────────────────────

    pub async fn set_blocked_request_observer(
        &self,
        observer: Arc<dyn BlockedRequestObserver>,
    ) {
        *self.blocked_request_observer.write().await = Some(observer);
    }

    pub async fn record_blocked(&self, args: BlockedRequestArgs) {
        let request = BlockedRequest {
            host: args.host,
            reason: args.reason,
            client: args.client,
            method: args.method,
            protocol: args.protocol,
            port: args.port,
        };

        if let Some(observer) = self.blocked_request_observer.read().await.as_ref() {
            observer.on_blocked_request(request.clone()).await;
        }

        let mut blocked = self.blocked_requests.write().await;
        if blocked.len() >= MAX_BLOCKED_EVENTS {
            blocked.pop_front();
        }
        blocked.push_back(request);
    }

    pub async fn recent_blocked_requests(&self) -> Vec<BlockedRequest> {
        self.blocked_requests.read().await.iter().cloned().collect()
    }

    // ── Policy evaluation ───────────────────────────────────────────────

    pub async fn evaluate(&self, request: &NetworkPolicyRequest) -> NetworkDecision {
        let _ = self.reload_if_needed().await;

        let mode = self.network_mode().await;
        if matches!(mode, NetworkMode::Off) {
            return NetworkDecision::allow();
        }

        let host_decision = match self.host_blocked(&request.host, request.port).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("host block check failed: {e}");
                return NetworkDecision::deny(format!("host check failed: {e}"));
            }
        };

        match host_decision {
            HostBlockDecision::Allowed => NetworkDecision::allow(),
            HostBlockDecision::Blocked(reason) => {
                if matches!(mode, NetworkMode::Audit) {
                    tracing::info!(
                        "audit mode: would block {}:{} reason={}",
                        request.host,
                        request.port,
                        reason.as_str()
                    );
                    NetworkDecision::allow()
                } else {
                    NetworkDecision::deny_with_source(
                        reason.as_str(),
                        NetworkDecisionSource::BaselinePolicy,
                    )
                }
            }
        }
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern == host {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host.ends_with(suffix) && host.len() > suffix.len();
    }
    false
}

fn is_loopback_host(host: &str) -> bool {
    host == "127.0.0.1"
        || host == "::1"
        || host.eq_ignore_ascii_case("localhost")
}

fn is_ip_address(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

async fn resolve_host(host: &str) -> anyhow::Result<Vec<IpAddr>> {
    let host = host.to_string();
    let addrs = tokio::net::lookup_host(format!("{host}:0"))
        .await?
        .map(|addr| addr.ip())
        .collect();
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        NetworkDomainPermission, NetworkDomainPermissionEntry, NetworkDomainPermissions,
    };
    use crate::network_policy::NetworkProtocol;

    fn make_settings_with_domains(
        entries: Vec<NetworkDomainPermissionEntry>,
    ) -> NetworkProxySettings {
        NetworkProxySettings {
            domains: NetworkDomainPermissions { entries },
            ..Default::default()
        }
    }

    #[test]
    fn host_matches_exact() {
        assert!(host_matches("example.com", "example.com"));
        assert!(!host_matches("example.com", "other.com"));
    }

    #[test]
    fn host_matches_wildcard() {
        assert!(host_matches("*.example.com", "sub.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
    }

    fn make_request(host: &str) -> NetworkPolicyRequest {
        NetworkPolicyRequest {
            protocol: NetworkProtocol::HttpsConnect,
            host: host.into(),
            port: 443,
            client_addr: None,
            method: None,
            command: None,
            exec_policy_hint: None,
        }
    }

    #[tokio::test]
    async fn evaluate_allows_when_mode_off() {
        let settings = NetworkProxySettings::default();
        let state = NetworkProxyState::for_settings(settings);
        state.set_network_mode(NetworkMode::Off).await;
        let result = state.evaluate(&make_request("blocked.com")).await;
        assert!(result.is_allowed());
    }

    #[tokio::test]
    async fn host_blocked_denies_dynamic_denied_domain() {
        let settings = NetworkProxySettings::default();
        let state = NetworkProxyState::for_settings(settings);
        state.add_denied_domain("evil.com".into()).await;
        let decision = state.host_blocked("evil.com", 443).await.unwrap();
        assert_eq!(
            decision,
            HostBlockDecision::Blocked(HostBlockReason::ExplicitlyDenied)
        );
    }

    #[tokio::test]
    async fn host_blocked_allows_dynamic_allowed_domain() {
        let settings = NetworkProxySettings {
            mode: Some(NetworkMode::Limited),
            ..Default::default()
        };
        let state = NetworkProxyState::for_settings(settings);
        state.add_allowed_domain("good.com".into()).await;
        let decision = state.host_blocked("good.com", 443).await.unwrap();
        assert_eq!(decision, HostBlockDecision::Allowed);
    }

    #[tokio::test]
    async fn network_mode_switching() {
        let settings = NetworkProxySettings::default();
        let state = NetworkProxyState::for_settings(settings);
        assert_eq!(state.network_mode().await, NetworkMode::Full);
        state.set_network_mode(NetworkMode::Audit).await;
        assert_eq!(state.network_mode().await, NetworkMode::Audit);
    }

    #[tokio::test]
    async fn is_unix_socket_allowed_checks_prefix() {
        use crate::config::{UnixSocketPermissionEntry, NetworkUnixSocketPermission};
        use std::path::PathBuf;
        let mut settings = NetworkProxySettings::default();
        settings.unix_sockets.entries.push(UnixSocketPermissionEntry {
            path: PathBuf::from("/tmp/safe/"),
            permission: NetworkUnixSocketPermission::Allow,
        });
        let state = NetworkProxyState::for_settings(settings);
        assert!(state.is_unix_socket_allowed(Path::new("/tmp/safe/test.sock")).await);
        assert!(!state.is_unix_socket_allowed(Path::new("/tmp/evil/test.sock")).await);
    }

    #[tokio::test]
    async fn replace_config_state_works() {
        let settings = NetworkProxySettings {
            enabled: false,
            ..Default::default()
        };
        let state = NetworkProxyState::for_settings(settings);
        assert!(!state.enabled().await);

        let new_settings = NetworkProxySettings {
            enabled: true,
            ..Default::default()
        };
        state.replace_config_state(new_settings).await;
        assert!(state.enabled().await);
    }

    #[tokio::test]
    async fn evaluate_audit_mode_allows_but_logs() {
        let settings = NetworkProxySettings::default();
        let state = NetworkProxyState::for_settings(settings);
        state.set_network_mode(NetworkMode::Audit).await;
        state.add_denied_domain("evil.com".into()).await;
        let result = state.evaluate(&make_request("evil.com")).await;
        assert!(result.is_allowed());
    }

    #[test]
    fn config_state_from_settings() {
        let settings = NetworkProxySettings::default();
        let state = ConfigState::from_settings(settings.clone());
        assert_eq!(state.settings.enabled, settings.enabled);
    }

    #[tokio::test]
    async fn record_and_retrieve_blocked_requests() {
        let settings = NetworkProxySettings::default();
        let state = NetworkProxyState::for_settings(settings);
        state
            .record_blocked(BlockedRequestArgs {
                host: "evil.com".into(),
                reason: "denied".into(),
                client: None,
                method: None,
                protocol: "https".into(),
                port: Some(443),
            })
            .await;
        let recent = state.recent_blocked_requests().await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].host, "evil.com");
    }
}
