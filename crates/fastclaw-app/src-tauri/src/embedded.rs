use serde::Serialize;
use std::sync::Arc;
use tokio::sync::oneshot;

pub use fastclaw_gateway::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayInfo {
    pub port: u16,
    #[serde(rename = "wsUrl")]
    pub ws_url: String,
    #[serde(rename = "httpUrl")]
    pub http_url: String,
    pub version: String,
}

pub struct EmbeddedGateway {
    pub info: GatewayInfo,
    app_state: Arc<AppState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl EmbeddedGateway {
    pub async fn start(dev: bool, profile: Option<&str>) -> anyhow::Result<Self> {
        let mut config = fastclaw_core::config::load_config(dev, profile)?;
        if config.gateway.cors_origins.is_empty() {
            config.gateway.cors_origins = vec!["*".to_string()];
        }
        let bind_addr = config.gateway.bind_addr();

        let listener = match tokio::net::TcpListener::bind(bind_addr).await {
            Ok(l) => l,
            Err(_) => {
                tracing::warn!(
                    %bind_addr,
                    "port in use, binding to random port"
                );
                tokio::net::TcpListener::bind("127.0.0.1:0").await?
            }
        };

        let local_addr = listener.local_addr()?;
        let port = local_addr.port();

        let info = GatewayInfo {
            port,
            ws_url: format!("ws://127.0.0.1:{port}/ws"),
            http_url: format!("http://127.0.0.1:{port}"),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let app_state = Arc::new(AppState::new(config).await?);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let state_for_server = (*app_state).clone();
        tokio::spawn(async move {
            if let Err(e) =
                fastclaw_gateway::serve_with_state(state_for_server, listener, shutdown_rx).await
            {
                tracing::error!("embedded gateway exited with error: {e}");
            }
        });

        wait_for_health(port, std::time::Duration::from_secs(10)).await?;

        tracing::info!(port, "embedded gateway ready");

        Ok(Self {
            info,
            app_state,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn info(&self) -> &GatewayInfo {
        &self.info
    }

    pub fn app_state(&self) -> &AppState {
        &self.app_state
    }

    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for EmbeddedGateway {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn wait_for_health(port: u16, timeout: std::time::Duration) -> anyhow::Result<()> {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let start = std::time::Instant::now();
    loop {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {
                if start.elapsed() > timeout {
                    anyhow::bail!("gateway health check timed out after {timeout:?}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}
