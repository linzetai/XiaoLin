use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::http_proxy::run_http_proxy_on_listener;
use crate::network_policy::NetworkDecision;
use crate::runtime::{BlockedRequestObserver, NetworkProxyState};
use crate::socks5::run_socks5_proxy_on_listener;

/// The running network proxy with bound addresses.
#[derive(Clone)]
pub struct NetworkProxy {
    state: Arc<NetworkProxyState>,
    http_addr: SocketAddr,
    socks_addr: Option<SocketAddr>,
    socks_enabled: bool,
}

impl std::fmt::Debug for NetworkProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkProxy")
            .field("http_addr", &self.http_addr)
            .field("socks_addr", &self.socks_addr)
            .field("socks_enabled", &self.socks_enabled)
            .finish()
    }
}

impl NetworkProxy {
    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    pub fn socks_addr(&self) -> Option<SocketAddr> {
        self.socks_addr
    }

    pub fn socks_enabled(&self) -> bool {
        self.socks_enabled
    }

    pub fn state(&self) -> &Arc<NetworkProxyState> {
        &self.state
    }

    /// Returns the proxy URL for HTTP clients.
    pub fn http_proxy_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }

    /// Returns the SOCKS5 proxy URL, if enabled.
    pub fn socks_proxy_url(&self) -> Option<String> {
        self.socks_addr.map(|addr| format!("socks5://{addr}"))
    }

    /// Environment variables that should be set for child processes to
    /// route traffic through this proxy.
    ///
    /// Covers 30+ variables to ensure broad tool compatibility (npm, yarn,
    /// pip, docker, bundle, websocket, ftp, electron, git-ssh, etc.).
    pub fn env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        let http_url = self.http_proxy_url();

        // Core proxy variables (case variants)
        env.insert("HTTP_PROXY".into(), http_url.clone());
        env.insert("HTTPS_PROXY".into(), http_url.clone());
        env.insert("http_proxy".into(), http_url.clone());
        env.insert("https_proxy".into(), http_url.clone());

        // npm
        env.insert("npm_config_http_proxy".into(), http_url.clone());
        env.insert("npm_config_https_proxy".into(), http_url.clone());
        env.insert("npm_config_proxy".into(), http_url.clone());
        env.insert("NPM_CONFIG_HTTP_PROXY".into(), http_url.clone());
        env.insert("NPM_CONFIG_HTTPS_PROXY".into(), http_url.clone());
        env.insert("NPM_CONFIG_PROXY".into(), http_url.clone());

        // yarn
        env.insert("YARN_HTTP_PROXY".into(), http_url.clone());
        env.insert("YARN_HTTPS_PROXY".into(), http_url.clone());

        // bundle (Ruby)
        env.insert("BUNDLE_HTTP_PROXY".into(), http_url.clone());
        env.insert("BUNDLE_HTTPS_PROXY".into(), http_url.clone());

        // pip (Python)
        env.insert("PIP_PROXY".into(), http_url.clone());

        // docker
        env.insert("DOCKER_HTTP_PROXY".into(), http_url.clone());
        env.insert("DOCKER_HTTPS_PROXY".into(), http_url.clone());

        // websocket
        env.insert("WS_PROXY".into(), http_url.clone());
        env.insert("WSS_PROXY".into(), http_url.clone());
        env.insert("ws_proxy".into(), http_url.clone());
        env.insert("wss_proxy".into(), http_url.clone());

        // ftp
        env.insert("FTP_PROXY".into(), http_url.clone());
        env.insert("ftp_proxy".into(), http_url.clone());

        // Electron
        env.insert("ELECTRON_GET_USE_PROXY".into(), "true".into());

        // NO_PROXY with RFC1918 private ranges
        let no_proxy = super::proxy_env::DEFAULT_NO_PROXY_VALUE;
        env.insert("NO_PROXY".into(), no_proxy.into());
        env.insert("no_proxy".into(), no_proxy.into());

        // Proxy active marker
        env.insert(
            super::proxy_env::PROXY_ACTIVE_ENV_KEY.into(),
            "1".into(),
        );

        // SOCKS5-specific overrides
        if let Some(socks_url) = self.socks_proxy_url() {
            let socks_h_url = socks_url.replace("socks5://", "socks5h://");
            env.insert("ALL_PROXY".into(), socks_h_url.clone());
            env.insert("all_proxy".into(), socks_h_url.clone());
            env.insert("FTP_PROXY".into(), socks_h_url.clone());
            env.insert("ftp_proxy".into(), socks_h_url);

            // macOS: route git SSH through SOCKS5 proxy
            #[cfg(target_os = "macos")]
            if let Some(socks_addr) = self.socks_addr {
                let ssh_cmd = format!(
                    "ssh -o ProxyCommand='nc -X 5 -x {} %h %p'",
                    socks_addr
                );
                env.insert("GIT_SSH_COMMAND".into(), ssh_cmd);
            }
        }

        env
    }
}

/// Builder for constructing a `NetworkProxy`.
#[derive(Clone, Default)]
pub struct NetworkProxyBuilder {
    state: Option<Arc<NetworkProxyState>>,
    http_addr: Option<SocketAddr>,
    socks_addr: Option<SocketAddr>,
    policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
    blocked_request_observer: Option<Arc<dyn BlockedRequestObserver>>,
}

/// Trait for custom synchronous network policy decisions.
pub trait NetworkPolicyDecider: Send + Sync + 'static {
    fn decide(
        &self,
        request: &crate::network_policy::NetworkPolicyRequest,
    ) -> NetworkDecision;
}

impl NetworkProxyBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_state(mut self, state: Arc<NetworkProxyState>) -> Self {
        self.state = Some(state);
        self
    }

    pub fn with_http_addr(mut self, addr: SocketAddr) -> Self {
        self.http_addr = Some(addr);
        self
    }

    pub fn with_socks_addr(mut self, addr: SocketAddr) -> Self {
        self.socks_addr = Some(addr);
        self
    }

    pub fn with_policy_decider(mut self, decider: Arc<dyn NetworkPolicyDecider>) -> Self {
        self.policy_decider = Some(decider);
        self
    }

    pub fn with_blocked_request_observer(
        mut self,
        observer: Arc<dyn BlockedRequestObserver>,
    ) -> Self {
        self.blocked_request_observer = Some(observer);
        self
    }

    /// Build and start the proxy, binding to the configured addresses.
    ///
    /// The returned `NetworkProxyHandle` owns the background tasks.
    pub async fn build(self) -> Result<(NetworkProxy, NetworkProxyHandle), ProxyBuildError> {
        let state = self.state.ok_or(ProxyBuildError::MissingState)?;

        let http_listener = tokio::net::TcpListener::bind(
            self.http_addr.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()),
        )
        .await
        .map_err(|e| ProxyBuildError::BindFailed(format!("http: {e}")))?;

        let http_addr = http_listener
            .local_addr()
            .map_err(|e| ProxyBuildError::BindFailed(format!("http local_addr: {e}")))?;

        if let Some(observer) = self.blocked_request_observer {
            state.set_blocked_request_observer(observer).await;
        }

        // Spawn the HTTP proxy engine on the pre-bound listener
        let http_state = Arc::clone(&state);
        let http_task = tokio::spawn(async move {
            if let Err(e) = run_http_proxy_on_listener(http_state, http_listener).await {
                tracing::error!("HTTP proxy engine exited with error: {e}");
            }
        });

        // Bind and spawn the SOCKS5 proxy engine if requested
        let (socks_addr, socks_task) = if let Some(socks_bind) = self.socks_addr {
            let socks_listener = tokio::net::TcpListener::bind(socks_bind)
                .await
                .map_err(|e| ProxyBuildError::BindFailed(format!("socks: {e}")))?;
            let actual_addr = socks_listener
                .local_addr()
                .map_err(|e| ProxyBuildError::BindFailed(format!("socks local_addr: {e}")))?;
            let socks_state = Arc::clone(&state);
            let task = tokio::spawn(async move {
                if let Err(e) = run_socks5_proxy_on_listener(socks_state, socks_listener).await {
                    tracing::error!("SOCKS5 proxy engine exited with error: {e}");
                }
            });
            (Some(actual_addr), Some(task))
        } else {
            (None, None)
        };

        let socks_enabled = socks_addr.is_some();

        let proxy = NetworkProxy {
            state,
            http_addr,
            socks_addr,
            socks_enabled,
        };

        let handle = NetworkProxyHandle {
            http_task: Some(http_task),
            socks_task,
            completed: false,
        };

        Ok((proxy, handle))
    }
}

/// Handle to the running proxy tasks.
pub struct NetworkProxyHandle {
    http_task: Option<JoinHandle<()>>,
    socks_task: Option<JoinHandle<()>>,
    completed: bool,
}

impl NetworkProxyHandle {
    pub fn noop() -> Self {
        Self {
            http_task: None,
            socks_task: None,
            completed: true,
        }
    }

    /// Wait for both proxy tasks to complete (i.e., until shutdown).
    pub async fn wait(mut self) -> Result<(), String> {
        if let Some(http_task) = self.http_task.take() {
            http_task
                .await
                .map_err(|e| format!("HTTP proxy task panicked: {e}"))?;
        }
        if let Some(socks_task) = self.socks_task.take() {
            socks_task
                .await
                .map_err(|e| format!("SOCKS proxy task panicked: {e}"))?;
        }
        self.completed = true;
        Ok(())
    }

    /// Abort both proxy tasks immediately.
    pub fn abort(&mut self) {
        if let Some(task) = self.http_task.take() {
            task.abort();
        }
        if let Some(task) = self.socks_task.take() {
            task.abort();
        }
        self.completed = true;
    }
}

impl Drop for NetworkProxyHandle {
    fn drop(&mut self) {
        if !self.completed {
            self.abort();
        }
    }
}

/// Errors from building/starting the network proxy.
#[derive(Debug, thiserror::Error)]
pub enum ProxyBuildError {
    #[error("NetworkProxyState is required")]
    MissingState,
    #[error("failed to bind proxy listener: {0}")]
    BindFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{ConfigState, NetworkProxyAuditMetadata};

    struct NoopReloader;
    impl crate::runtime::ConfigReloader for NoopReloader {
        fn reload(
            &self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<crate::config::NetworkProxySettings, String>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Ok(crate::config::NetworkProxySettings::default()) })
        }
    }

    #[tokio::test]
    async fn builder_creates_proxy() {
        let settings = crate::config::NetworkProxySettings::default();
        let state = Arc::new(NetworkProxyState::new(
            ConfigState::from_settings(settings),
            Arc::new(NoopReloader),
            NetworkProxyAuditMetadata::default(),
        ));
        let (proxy, mut handle) = NetworkProxyBuilder::new()
            .with_state(state)
            .build()
            .await
            .unwrap();
        assert!(proxy.http_addr().port() > 0);
        handle.abort();
    }

    #[test]
    fn proxy_env_vars_contain_http_proxy() {
        let state = Arc::new(NetworkProxyState::new(
            ConfigState::from_settings(crate::config::NetworkProxySettings::default()),
            Arc::new(NoopReloader),
            NetworkProxyAuditMetadata::default(),
        ));
        let proxy = NetworkProxy {
            state,
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            socks_addr: None,
            socks_enabled: false,
        };
        let env = proxy.env_vars();
        assert_eq!(env.get("HTTP_PROXY").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(env.get("HTTPS_PROXY").unwrap(), "http://127.0.0.1:8080");
    }
}
