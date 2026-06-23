use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayInfo {
    pub port: u16,
    #[serde(rename = "wsUrl")]
    pub ws_url: String,
    #[serde(rename = "httpUrl")]
    pub http_url: String,
    pub version: String,
}

/// Gateway process manager for Tauri.
///
/// The gateway always runs in-process (embedded). There is no daemon mode.
/// The Tauri frontend connects to the gateway via WebSocket for all
/// business logic (chat, sessions, agents, etc.).
pub struct GatewayProcess {
    pub info: GatewayInfo,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl GatewayProcess {
    /// Start the embedded gateway in-process.
    pub async fn start(mode: &xiaolin_core::config::ConfigMode) -> anyhow::Result<Self> {
        let config = xiaolin_core::config::load_config(mode)?;
        xiaolin_gateway::set_config_mode(mode.clone());

        let port = config.gateway.port;
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(_) => {
                tracing::warn!(port, "default port occupied, binding to random port");
                tokio::net::TcpListener::bind("127.0.0.1:0").await?
            }
        };
        let actual_port = listener.local_addr()?.port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            match xiaolin_gateway::AppState::new(config).await {
                Ok(state) => {
                    xiaolin_tools_browser::set_network_ws_broadcast(state.strm.ws_broadcast.clone());
                    if let Err(e) =
                        xiaolin_gateway::serve_with_state(state, listener, shutdown_rx).await
                    {
                        tracing::error!("embedded gateway error: {e}");
                    }
                }
                Err(e) => tracing::error!("embedded gateway init error: {e}"),
            }
        });

        let timeout = std::time::Duration::from_secs(60);
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if probe_health(actual_port).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !probe_health(actual_port).await {
            anyhow::bail!(
                "embedded gateway did not become ready within {}s",
                timeout.as_secs()
            );
        }

        let info = GatewayInfo {
            port: actual_port,
            ws_url: format!("ws://127.0.0.1:{actual_port}/ws"),
            http_url: format!("http://127.0.0.1:{actual_port}"),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        tracing::info!(port = actual_port, "embedded gateway ready");

        Ok(Self {
            info,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    /// Send shutdown signal to the in-process gateway.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            tracing::info!("sent shutdown signal to embedded gateway");
        }
    }

    pub fn info(&self) -> &GatewayInfo {
        &self.info
    }
}

async fn probe_health(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
    else {
        return false;
    };
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}
