use serde::Serialize;
use std::process::Stdio;
use tokio::process::Command;

pub use fastclaw_core::config::GatewayState;

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
/// This struct is responsible for:
/// 1. Discovering an already-running Gateway via gateway.json
/// 2. Starting a Gateway in-process or as daemon based on `gateway.embed` config
/// 3. Providing connection info to the frontend
///
/// The Tauri frontend connects to the Gateway via WebSocket for all
/// business logic (chat, sessions, agents, etc.). No IPC commands
/// for business logic are needed - everything goes through WS.
pub struct GatewayProcess {
    pub info: GatewayInfo,
    /// Shutdown sender for in-process embedded gateway. None when connected to external daemon.
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl GatewayProcess {
    /// Start or connect to a gateway.
    ///
    /// Flow:
    /// 1. Check gateway.json for existing gateway
    /// 2. If found and alive, use its connection info
    /// 3. Probe default port for orphaned gateway (no state file but port occupied)
    /// 4. Based on `gateway.embed` config: start in-process or as external daemon
    pub async fn start(mode: &fastclaw_core::config::ConfigMode) -> anyhow::Result<Self> {
        // 1. Check for existing gateway via gateway.json
        if let Ok(state) = GatewayState::read(mode) {
            if state.is_alive() {
                // Verify with HTTP health check
                if probe_gateway(state.port).await {
                    tracing::info!(
                        port = state.port,
                        pid = state.pid,
                        "found running gateway, connecting to it"
                    );
                    return Self::connect_existing(state);
                }
            }
            // Stale state file — clean it up
            tracing::debug!("stale gateway state file found, cleaning up");
            let _ = GatewayState::remove(mode);
        }

        // 2. Probe default port for orphaned gateway (running but no state file)
        let default_port = fastclaw_core::config::default_port_for_mode(mode);
        if probe_gateway(default_port).await {
            tracing::info!(
                port = default_port,
                "found orphaned gateway on default port (no state file), reconnecting"
            );
            let state = GatewayState::new(default_port);
            if let Err(e) = state.write(mode) {
                tracing::warn!(error = %e, "failed to write recovered gateway state file");
            }
            return Self::connect_existing(state);
        }

        // 3. Decide based on embed config
        let config = fastclaw_core::config::load_config(mode)?;
        if config.gateway.embed.should_embed() {
            tracing::info!("starting embedded gateway in-process");
            Self::start_embedded(config, mode).await
        } else {
            Self::start_daemon(mode).await
        }
    }

    /// Connect to an existing gateway without starting a new one.
    fn connect_existing(state: GatewayState) -> anyhow::Result<Self> {
        let info = GatewayInfo {
            port: state.port,
            ws_url: state.ws_url.clone(),
            http_url: state.http_url.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        tracing::info!(
            port = state.port,
            "connected to existing gateway"
        );

        Ok(Self { info, shutdown_tx: None })
    }

    /// Start the gateway in-process using `run_with_listener`.
    async fn start_embedded(
        config: fastclaw_core::config::FastClawConfig,
        mode: &fastclaw_core::config::ConfigMode,
    ) -> anyhow::Result<Self> {
        fastclaw_gateway::set_config_mode(mode.clone());

        let port = config.gateway.port;
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0").await?,
        };
        let actual_port = listener.local_addr()?.port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            if let Err(e) =
                fastclaw_gateway::run_with_listener(config, listener, shutdown_rx).await
            {
                tracing::error!("embedded gateway error: {e}");
            }
        });

        // Wait for health
        let timeout = std::time::Duration::from_secs(15);
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if probe_gateway(actual_port).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !probe_gateway(actual_port).await {
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

    /// Send shutdown signal to the in-process gateway (no-op for external daemons).
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            tracing::info!("sent shutdown signal to embedded gateway");
        }
    }

    /// Start a gateway daemon process using the fastclaw CLI.
    ///
    /// We use `fastclaw gateway start` which handles daemonization internally
    /// (double-fork, PID file, log file). We cannot use `current_exe()` from
    /// the Tauri app because that would be the Tauri binary itself, so we
    /// locate the `fastclaw` CLI binary explicitly.
    async fn start_daemon(mode: &fastclaw_core::config::ConfigMode) -> anyhow::Result<Self> {
        // Find fastclaw CLI binary
        let fastclaw_exe = find_fastclaw_cli()?;

        let mut cmd = Command::new(&fastclaw_exe);
        // --dev and --profile are top-level flags (before subcommand)
        if mode.is_dev() {
            cmd.arg("--dev");
        }
        if let Some(p) = mode.profile_name() {
            cmd.arg("--profile");
            cmd.arg(p);
        }
        cmd.arg("gateway").arg("start");

        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // `fastclaw gateway start` daemonizes internally and exits quickly
        let output = cmd
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to run `fastclaw gateway start`: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(anyhow::anyhow!(
                "`fastclaw gateway start` failed (exit {}): stdout={stdout} stderr={stderr}",
                output.status.code().unwrap_or(-1)
            ));
        }

        tracing::info!(exe = %fastclaw_exe.display(), "gateway start command succeeded");

        // Wait for the gateway to become ready
        let state = wait_for_gateway(mode, std::time::Duration::from_secs(15)).await?;

        tracing::info!(port = state.port, "gateway daemon ready");

        Self::connect_existing(state)
    }

    pub fn info(&self) -> &GatewayInfo {
        &self.info
    }
}

/// Find the fastclaw CLI executable.
///
/// Search order:
/// 1. FASTCLAW_CLI env var
/// 2. (Dev only) Cargo workspace target directory — ensures dev Tauri app uses dev-built CLI
/// 3. Same directory as current exe (for bundled installs)
/// 4. PATH lookup
fn find_fastclaw_cli() -> anyhow::Result<std::path::PathBuf> {
    // 1. Check environment variable
    if let Ok(path) = std::env::var("FASTCLAW_CLI") {
        let path = std::path::PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }

    // 2. (Dev only) Check cargo workspace target directory.
    //    CARGO_MANIFEST_DIR is set at compile time to `crates/fastclaw-app/src-tauri/`.
    //    Walk up to workspace root and look for `target/debug/fastclaw`.
    #[cfg(debug_assertions)]
    {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(workspace_root) = manifest_dir.ancestors().nth(3) {
            let dev_cli = workspace_root.join("target/debug/fastclaw");
            if dev_cli.exists() {
                tracing::debug!(path = %dev_cli.display(), "using dev-built CLI from workspace");
                return Ok(dev_cli);
            }
            tracing::warn!(
                expected = %dev_cli.display(),
                "dev-built CLI not found; run `cargo build -p fastclaw-cli` first, falling back to PATH"
            );
        }
    }

    // 3. Check same directory as current exe (bundled install scenario)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sibling = parent.join("fastclaw");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    // 4. Look up in PATH
    if let Some(path) = which_in_path("fastclaw") {
        return Ok(path);
    }

    Err(anyhow::anyhow!(
        "fastclaw CLI not found. Please install it or set FASTCLAW_CLI environment variable."
    ))
}

/// Simple PATH lookup for an executable name.
fn which_in_path(name: &str) -> Option<std::path::PathBuf> {
    let Ok(path_var) = std::env::var("PATH") else {
        return None;
    };
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Probe whether a gateway is alive on the given port via HTTP health check.
async fn probe_gateway(port: u16) -> bool {
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

/// Wait for a gateway to become ready by polling the state file and health endpoint.
async fn wait_for_gateway(
    mode: &fastclaw_core::config::ConfigMode,
    timeout: std::time::Duration,
) -> anyhow::Result<GatewayState> {
    let start = std::time::Instant::now();
    let check_interval = std::time::Duration::from_millis(200);

    while start.elapsed() < timeout {
        // Try reading the state file first
        if let Ok(state) = GatewayState::read(mode) {
            if state.is_alive() && probe_gateway(state.port).await {
                return Ok(state);
            }
        }

        tokio::time::sleep(check_interval).await;
    }

    anyhow::bail!("gateway did not become ready within {}s", timeout.as_secs())
}