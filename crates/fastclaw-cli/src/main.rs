mod tui;

use clap::{CommandFactory, Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Parser)]
#[command(
    name = "fastclaw",
    version,
    about = "FastClaw — AI Agent Orchestration Engine"
)]
struct Cli {
    #[arg(long, help = "Use development state directory (~/.fastclaw-dev/)")]
    dev: bool,

    #[arg(long, help = "Use named profile (~/.fastclaw-<name>/)")]
    profile: Option<String>,

    #[arg(long, help = "Disable colored output")]
    no_color: bool,

    #[arg(long, help = "Output in JSON format")]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive initial setup
    Setup,
    /// First-use onboarding
    Onboard,
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Environment diagnostics
    Doctor,
    /// Gateway service management
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Shortcut for `gateway run`
    Serve,
    /// Health check against a running gateway
    Health,
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Agent management
    Agents {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Tool management
    Tools {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Interactive terminal chat UI (connects to gateway via WebSocket)
    Tui {
        #[arg(
            long,
            default_value = "ws://127.0.0.1:18789/ws",
            help = "Gateway WebSocket URL"
        )]
        url: String,
        #[arg(long, help = "API key for auth")]
        token: Option<String>,
        #[arg(long, help = "Resume a specific session")]
        session: Option<String>,
    },
    /// Start MCP server (stdio transport) — exposes FastClaw tools to external agents
    McpServer,
    /// Generate shell completions (bash, zsh, fish, powershell, elvish)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Get a configuration value by dotted key path (e.g. gateway.port)
    Get { key: String },
    /// Set a configuration value by dotted key path
    Set { key: String, value: String },
    /// Validate the current configuration
    Check,
    /// Print the full resolved configuration
    File,
    /// Print the config file path
    Path,
    /// Attempt to auto-repair a broken config file
    Fix,
}

#[derive(Subcommand)]
enum GatewayAction {
    /// Run gateway in the foreground
    Run,
    /// Start gateway as a background process (daemon)
    Start,
    /// Stop background gateway
    Stop,
    /// Restart background gateway
    Restart,
    /// Check background gateway status
    Status,
    /// Health check
    Health,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List recent sessions
    List {
        #[arg(short, long, default_value = "20")]
        limit: i64,
        #[arg(short, long, default_value = "0")]
        offset: i64,
    },
    /// Get a specific session's details
    Get { session_id: String },
    /// Delete a session
    Delete { session_id: String },
    /// Clean up expired sessions
    Cleanup {
        #[arg(long, default_value = "168", help = "TTL in hours (default: 7 days)")]
        ttl_hours: u64,
    },
}

#[derive(Subcommand)]
enum AgentAction {
    /// List configured agents
    List,
    /// Get agent configuration details
    Get { agent_id: String },
}

#[derive(Subcommand)]
enum ToolAction {
    /// List available built-in tools
    List,
}

fn state_dir(dev: bool, profile: Option<&str>) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match (dev, profile) {
        (true, _) => home.join(".fastclaw-dev"),
        (_, Some(name)) => home.join(format!(".fastclaw-{name}")),
        _ => home.join(".fastclaw"),
    }
}

fn config_file_path(dev: bool, profile: Option<&str>) -> PathBuf {
    state_dir(dev, profile).join("config/default.json")
}

/// PID file for a background `fastclaw serve` process (per state dir / profile).
fn daemon_pid_path(dev: bool, profile: Option<&str>) -> PathBuf {
    state_dir(dev, profile).join("daemon.pid")
}

/// Log file for background gateway daemon output.
fn daemon_log_path(dev: bool, profile: Option<&str>) -> PathBuf {
    state_dir(dev, profile).join("logs/gateway-daemon.log")
}

fn read_daemon_pid(path: &Path) -> anyhow::Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let pid: u32 = trimmed
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid pid in {}: {e}", path.display()))?;
    Ok(Some(pid))
}

fn write_daemon_pid(path: &Path, pid: u32) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            anyhow::anyhow!(
                "cannot create directory {} (permission denied or I/O error): {e}",
                dir.display()
            )
        })?;
    }
    std::fs::write(path, format!("{pid}\n")).map_err(|e| {
        anyhow::anyhow!(
            "cannot write pid file {} (permission denied or I/O error): {e}",
            path.display()
        )
    })
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn signal_daemon_stop(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::anyhow!(
                    "permission denied sending SIGTERM to pid {pid}: try with appropriate privileges"
                )
            } else {
                anyhow::anyhow!("failed to run kill for pid {pid}: {e}")
            }
        })?;
    if !status.success() {
        anyhow::bail!("kill -TERM {pid} exited with {status}");
    }
    Ok(())
}

#[cfg(not(unix))]
fn signal_daemon_stop(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("gateway daemon stop is only supported on Unix-like systems")
}

fn cmd_daemon_start(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (dev, profile);
        anyhow::bail!("gateway daemon start is only supported on Unix-like systems; use `fastclaw serve` in a terminal multiplexer instead");
    }
    #[cfg(unix)]
    {
        let pid_path = daemon_pid_path(dev, profile);
        if let Some(pid) = read_daemon_pid(&pid_path)? {
            if process_alive(pid) {
                anyhow::bail!(
                    "gateway daemon already running (pid {pid}). Stop it first with `fastclaw gateway stop`."
                );
            }
            let _ = std::fs::remove_file(&pid_path);
        }

        let exe = std::env::current_exe()
            .map_err(|e| anyhow::anyhow!("cannot resolve current executable: {e}"))?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("serve");
        if dev {
            cmd.arg("--dev");
        }
        if let Some(p) = profile {
            cmd.arg("--profile");
            cmd.arg(p);
        }
        cmd.stdin(Stdio::null());
        let log_path = daemon_log_path(dev, profile);
        if let Some(dir) = log_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| anyhow::anyhow!("cannot open daemon log file {}: {e}", log_path.display()))?;
        let log_file_err = log_file
            .try_clone()
            .map_err(|e| anyhow::anyhow!("cannot clone daemon log file handle: {e}"))?;
        cmd.stdout(Stdio::from(log_file));
        cmd.stderr(Stdio::from(log_file_err));
        cmd.env("RUST_BACKTRACE", "1");

        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::anyhow!("permission denied spawning gateway daemon: {e}")
            } else {
                anyhow::anyhow!("failed to spawn gateway daemon: {e}")
            }
        })?;
        let pid = child.id();
        if pid == 0 {
            anyhow::bail!("spawned gateway has no pid on this platform");
        }
        write_daemon_pid(&pid_path, pid)?;
        // Do not drop `Child` on Unix: destructor would terminate the server. Detach from this CLI.
        std::mem::forget(child);
        println!(
            "Started FastClaw gateway daemon (pid {pid}). PID file: {}. Logs: {}",
            pid_path.display(),
            log_path.display()
        );
        Ok(())
    }
}

fn cmd_daemon_stop(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    let pid_path = daemon_pid_path(dev, profile);
    let Some(pid) = read_daemon_pid(&pid_path)? else {
        anyhow::bail!(
            "no gateway daemon pid file at {} (daemon not running)",
            pid_path.display()
        );
    };
    if !process_alive(pid) {
        let _ = std::fs::remove_file(&pid_path);
        anyhow::bail!("daemon not running (stale pid {pid} removed from {})", pid_path.display());
    }
    signal_daemon_stop(pid)?;
    let _ = std::fs::remove_file(&pid_path);
    println!("Stopped FastClaw gateway daemon (pid {pid}).");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let quiet = matches!(
        cli.command,
        Commands::Config { .. }
            | Commands::Sessions { .. }
            | Commands::Agents { .. }
            | Commands::Tools { .. }
            | Commands::Health
            | Commands::Doctor
    );

    if !quiet {
        fastclaw_observe::init_observability(if cli.json { "json" } else { "pretty" });
    }

    match cli.command {
        Commands::Serve
        | Commands::Gateway {
            action: GatewayAction::Run,
        } => {
            let config = fastclaw_core::config::load_config(cli.dev, cli.profile.as_deref())?;
            let log_format = if cli.json {
                "json"
            } else {
                &config.logging.format
            };
            fastclaw_observe::init_observability_with_level(
                log_format,
                Some(&config.logging.level),
            );

            let port = config.gateway.port;
            eprintln!();
            eprintln!("  ⚡ FastClaw v{}", env!("CARGO_PKG_VERSION"));
            eprintln!("  ➜  Local:   http://localhost:{port}/");
            eprintln!("  ➜  Network: http://0.0.0.0:{port}/");
            eprintln!();

            fastclaw_gateway::run(config).await?;
        }
        Commands::Health => {
            cmd_health(cli.dev, cli.profile.as_deref()).await?;
        }
        Commands::Doctor => {
            cmd_doctor(cli.dev, cli.profile.as_deref(), cli.json).await?;
        }
        Commands::Config { action } => {
            cmd_config(action, cli.dev, cli.profile.as_deref(), cli.json)?;
        }
        Commands::Sessions { action } => {
            cmd_sessions(action, cli.dev, cli.profile.as_deref(), cli.json).await?;
        }
        Commands::Agents { action } => {
            cmd_agents(action, cli.json)?;
        }
        Commands::Tools { action } => {
            cmd_tools(action, cli.json)?;
        }
        Commands::Tui {
            url,
            token,
            session,
        } => {
            let config = fastclaw_core::config::load_config(cli.dev, cli.profile.as_deref())
                .unwrap_or_default();
            let effective_url = if url == "ws://127.0.0.1:18789/ws" {
                format!("ws://127.0.0.1:{}/ws", config.gateway.port)
            } else {
                url
            };

            let sd = state_dir(cli.dev, cli.profile.as_deref());
            let ws_root = fastclaw_core::workspace::resolve_workspace_root(
                &sd,
                "main",
                config.workspace.as_deref().map(std::path::Path::new),
            );
            let _ = std::fs::create_dir_all(&ws_root);
            let work_dir = ws_root.to_string_lossy().to_string();

            tui::run_tui(
                &effective_url,
                token.as_deref(),
                session.as_deref(),
                Some(work_dir),
                cli.dev,
                cli.profile.clone(),
            )
            .await?;
        }
        Commands::McpServer => {
            cmd_mcp_server().await?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "fastclaw", &mut std::io::stdout());
        }
        Commands::Setup => {
            cmd_setup(cli.dev, cli.profile.as_deref())?;
        }
        Commands::Onboard => {
            cmd_onboard(cli.dev, cli.profile.as_deref())?;
        }
        Commands::Gateway { action } => match action {
            GatewayAction::Run => {
                let config = fastclaw_core::config::load_config(cli.dev, cli.profile.as_deref())?;
                fastclaw_gateway::run(config).await?;
            }
            GatewayAction::Start => {
                cmd_daemon_start(cli.dev, cli.profile.as_deref())?;
            }
            GatewayAction::Stop => {
                cmd_daemon_stop(cli.dev, cli.profile.as_deref())?;
            }
            GatewayAction::Restart => {
                let _ = cmd_daemon_stop(cli.dev, cli.profile.as_deref());
                cmd_daemon_start(cli.dev, cli.profile.as_deref())?;
            }
            GatewayAction::Status => {
                cmd_daemon_status(cli.dev, cli.profile.as_deref()).await?;
            }
            GatewayAction::Health => {
                cmd_health(cli.dev, cli.profile.as_deref()).await?;
            }
        },
    }

    Ok(())
}

// --- Config ---

fn cmd_config(
    action: ConfigAction,
    dev: bool,
    profile: Option<&str>,
    as_json: bool,
) -> anyhow::Result<()> {
    match action {
        ConfigAction::File => {
            let config = fastclaw_core::config::load_config(dev, profile)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigAction::Path => {
            println!("{}", config_file_path(dev, profile).display());
        }
        ConfigAction::Check => match fastclaw_core::config::load_config(dev, profile) {
            Ok(config) => {
                if as_json {
                    println!(
                        "{}",
                        serde_json::json!({ "valid": true, "port": config.gateway.port })
                    );
                } else {
                    println!("Configuration valid.");
                    println!("  Gateway port: {}", config.gateway.port);
                    println!("  Log level:    {}", config.logging.level);
                }
            }
            Err(e) => {
                if as_json {
                    println!(
                        "{}",
                        serde_json::json!({ "valid": false, "error": e.to_string() })
                    );
                } else {
                    eprintln!("Configuration invalid: {e}");
                }
                std::process::exit(1);
            }
        },
        ConfigAction::Get { key } => {
            let config = fastclaw_core::config::load_config(dev, profile)?;
            let full = serde_json::to_value(&config)?;
            let value = navigate_json(&full, &key);
            match value {
                Some(v) => println!("{}", serde_json::to_string_pretty(v)?),
                None => {
                    eprintln!("key not found: {key}");
                    std::process::exit(1);
                }
            }
        }
        ConfigAction::Set { key, value } => {
            let path = config_file_path(dev, profile);
            let mut config_value = if path.exists() {
                let text = std::fs::read_to_string(&path)?;
                serde_json::from_str::<serde_json::Value>(&text)?
            } else {
                serde_json::json!({})
            };

            let parsed_value: serde_json::Value =
                serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));

            set_json_path(&mut config_value, &key, parsed_value)?;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(&config_value)?)?;
            println!("set {key} in {}", path.display());
        }
        ConfigAction::Fix => {
            let path = config_file_path(dev, profile);
            if !path.exists() {
                eprintln!("No config file at {}", path.display());
                std::process::exit(1);
            }
            match fastclaw_core::config::repair_config_file(&path) {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

fn navigate_json<'a>(val: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = val;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn set_json_path(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> anyhow::Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            current[*part] = value;
            return Ok(());
        }
        if !current.get(*part).map_or(false, |v| v.is_object()) {
            current[*part] = serde_json::json!({});
        }
        current = current
            .get_mut(*part)
            .ok_or_else(|| anyhow::anyhow!("JSON path segment missing after insert: {part}"))?;
    }
    Ok(())
}

// --- Doctor ---

async fn cmd_doctor(dev: bool, profile: Option<&str>, as_json: bool) -> anyhow::Result<()> {
    let mut checks: Vec<(&str, bool, String)> = Vec::new();

    let version = env!("CARGO_PKG_VERSION");
    checks.push(("version", true, format!("FastClaw v{version}")));

    let sd = state_dir(dev, profile);
    let data_exists = sd.join("data").exists();
    checks.push(("state_dir", true, format!("{}", sd.display())));
    checks.push((
        "data_dir",
        data_exists,
        if data_exists {
            "exists".into()
        } else {
            "missing (will be created on first run)".into()
        },
    ));

    let cfg_path = config_file_path(dev, profile);
    let config_ok = cfg_path.exists();
    checks.push((
        "config_file",
        config_ok,
        if config_ok {
            format!("{}", cfg_path.display())
        } else {
            "not found (using defaults)".into()
        },
    ));

    let config = fastclaw_core::config::load_config(dev, profile).unwrap_or_default();

    let agents_dir = PathBuf::from("config/agents");
    let agent_count = if agents_dir.exists() {
        std::fs::read_dir(&agents_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
            .count()
    } else {
        0
    };
    checks.push((
        "agents",
        agent_count > 0,
        format!("{agent_count} agent(s) found"),
    ));

    let tools = fastclaw_core::tool::ToolRegistry::new();
    fastclaw_agent::builtin_tools::register_builtin_tools(&tools);
    checks.push((
        "tools",
        true,
        format!("{} built-in tool(s)", tools.definitions().len()),
    ));

    let db_path = sd.join("data/sessions.db");
    let db_exists = db_path.exists();
    checks.push((
        "session_db",
        db_exists,
        if db_exists {
            format!("{}", db_path.display())
        } else {
            "not created yet".into()
        },
    ));

    let has_llm_key = !config.credentials.providers.is_empty()
        && config
            .credentials
            .providers
            .values()
            .any(|c| c.api_key.is_some());
    checks.push((
        "llm_api_key",
        has_llm_key,
        if has_llm_key {
            let providers: Vec<&str> = config
                .credentials
                .providers
                .iter()
                .filter(|(_, c)| c.api_key.is_some())
                .map(|(k, _)| k.as_str())
                .collect();
            format!("credentials configured for: {}", providers.join(", "))
        } else {
            "no LLM credentials in config (add to \"credentials\" section)".into()
        },
    ));

    let auth_ok = !config.security.api_keys.is_empty();
    checks.push((
        "api_auth",
        auth_ok,
        if auth_ok {
            format!(
                "{} API key(s) configured in security.apiKeys",
                config.security.api_keys.len()
            )
        } else {
            "no API keys in security.apiKeys (authentication disabled)".into()
        },
    ));

    // Gateway connectivity check — use the configured port, not a hardcoded default.
    let gateway_port = config.gateway.port;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let gateway_url = format!("http://localhost:{gateway_port}/health");
    let gateway_ok = match client.get(&gateway_url).send().await {
        Ok(resp) if resp.status().is_success() => true,
        _ => false,
    };
    checks.push((
        "gateway",
        gateway_ok,
        if gateway_ok {
            format!("running at localhost:{gateway_port}")
        } else {
            format!("not running on port {gateway_port} (start with `fastclaw serve`)")
        },
    ));

    // Agent model config validation — check credentials in config
    if agents_dir.exists() {
        if let Ok(agents) = fastclaw_core::agent_config::load_agent_configs(&agents_dir) {
            for agent in &agents {
                let provider = &agent.model.provider;
                let model = &agent.model.model;
                let needs_key = !matches!(provider.as_str(), "ollama" | "lmstudio" | "vllm");
                let has_key = config.credentials.get_api_key(provider).is_some();
                let key_ok = !needs_key || has_key;
                let check_name = format!("agent:{}", agent.agent_id);
                let detail = if key_ok {
                    format!("{provider}/{model} — ready")
                } else {
                    format!("{provider}/{model} — missing credentials.{provider}.apiKey in config")
                };
                checks.push((Box::leak(check_name.into_boxed_str()), key_ok, detail));
            }
        }
    }

    // Shell completions available
    checks.push((
        "shell_completions",
        true,
        "run `fastclaw completions bash|zsh|fish`".into(),
    ));

    // Docker available
    let docker_ok = std::process::Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push((
        "docker",
        docker_ok,
        if docker_ok {
            "available".into()
        } else {
            "not found (optional, for containerized deployment)".into()
        },
    ));

    if as_json {
        let items: Vec<_> = checks.iter().map(|(name, ok, detail)| {
            serde_json::json!({ "check": name, "ok": ok, "detail": detail })
        }).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "version": version,
                "checks": items,
                "all_passed": checks.iter().all(|(_, ok, _)| *ok),
            }))?
        );
    } else {
        println!("FastClaw Doctor v{version}");
        println!("{}", "=".repeat(40));
        for (name, ok, detail) in &checks {
            let status = if *ok { "OK" } else { "WARN" };
            println!("  [{status:>4}] {name}: {detail}");
        }
        println!("{}", "=".repeat(40));
        let all_ok = checks.iter().all(|(_, ok, _)| *ok);
        if all_ok {
            println!("All checks passed.");
        } else {
            println!("Some checks need attention (see WARN above).");
        }
    }

    Ok(())
}

// --- Health ---

/// Probe the gateway health endpoint. Reads the configured port so that the
/// check always targets the FastClaw instance for the active profile instead
/// of a hardcoded default that may be occupied by a different process.
async fn cmd_health(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    let config = fastclaw_core::config::load_config(dev, profile).unwrap_or_default();
    let port = config.gateway.port;
    let url = format!("http://localhost:{port}/health");
    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("Gateway is running (status: {})", resp.status());
        }
        Ok(resp) => {
            eprintln!("Gateway returned status: {}", resp.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Cannot connect to gateway at localhost:{port}: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Check the **daemon** gateway status:
/// 1. Verify PID file exists and the daemon process is alive.
/// 2. If alive, confirm the gateway is responsive on the configured port.
///
/// This prevents false-positives when another process happens to be listening
/// on the same address (e.g. an openclaw-gateway on the default port 18789).
async fn cmd_daemon_status(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    let pid_path = daemon_pid_path(dev, profile);
    let log_path = daemon_log_path(dev, profile);
    let config = fastclaw_core::config::load_config(dev, profile).unwrap_or_default();
    let port = config.gateway.port;

    let pid = match read_daemon_pid(&pid_path)? {
        Some(p) => p,
        None => {
            eprintln!(
                "FastClaw gateway daemon is not running (no PID file). Logs: {}",
                log_path.display()
            );
            std::process::exit(1);
        }
    };

    if !process_alive(pid) {
        let _ = std::fs::remove_file(&pid_path);
        eprintln!(
            "FastClaw gateway daemon is not running (stale PID {pid} removed). Logs: {}",
            log_path.display()
        );
        std::process::exit(1);
    }

    // Confirm the FastClaw process is actually serving HTTP on its configured port.
    let url = format!("http://localhost:{port}/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!(
                "FastClaw gateway daemon is running (pid {pid}, port {port}, status: {}). Logs: {}",
                resp.status(),
                log_path.display()
            );
        }
        Ok(resp) => {
            eprintln!(
                "FastClaw daemon pid {pid} is alive but health check returned {}",
                resp.status()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "FastClaw daemon pid {pid} is alive but not responding on port {port}: {e}"
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

// --- Sessions ---

async fn cmd_sessions(
    action: SessionAction,
    dev: bool,
    profile: Option<&str>,
    as_json: bool,
) -> anyhow::Result<()> {
    let db_path = state_dir(dev, profile).join("data/sessions.db");
    if !db_path.exists() {
        eprintln!("No session database found at {}", db_path.display());
        eprintln!("Start the gateway first with `fastclaw serve`.");
        std::process::exit(1);
    }

    let store = fastclaw_session::SessionStore::open(&db_path).await?;

    match action {
        SessionAction::List { limit, offset } => {
            let sessions = store.list_sessions(limit, offset).await?;
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "sessions": sessions.iter().map(|s| serde_json::json!({
                            "id": s.id,
                            "agent_id": s.agent_id,
                            "title": s.title,
                            "message_count": s.message_count,
                            "created_at": s.created_at,
                            "updated_at": s.updated_at,
                        })).collect::<Vec<_>>(),
                        "count": sessions.len(),
                    }))?
                );
            } else {
                if sessions.is_empty() {
                    println!("No sessions found.");
                } else {
                    println!("{:<40} {:<10} {:<6} {}", "ID", "Agent", "Msgs", "Updated");
                    println!("{}", "-".repeat(80));
                    for s in &sessions {
                        println!(
                            "{:<40} {:<10} {:<6} {}",
                            s.id, s.agent_id, s.message_count, s.updated_at
                        );
                    }
                    println!("\n{} session(s)", sessions.len());
                }
            }
        }
        SessionAction::Get { session_id } => match store.get_session(&session_id).await? {
            Some(session) => {
                let messages = store.load_messages(&session_id).await?;
                if as_json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "session": {
                                "id": session.id,
                                "agent_id": session.agent_id,
                                "title": session.title,
                                "message_count": session.message_count,
                                "created_at": session.created_at,
                                "updated_at": session.updated_at,
                            },
                            "messages": messages.iter().map(|m| serde_json::json!({
                                "id": m.id,
                                "role": m.role,
                                "content": m.content,
                                "tool_call_id": m.tool_call_id,
                                "created_at": m.created_at,
                            })).collect::<Vec<_>>(),
                        }))?
                    );
                } else {
                    println!("Session: {}", session.id);
                    println!("  Agent:    {}", session.agent_id);
                    println!(
                        "  Title:    {}",
                        session.title.as_deref().unwrap_or("(none)")
                    );
                    println!("  Messages: {}", session.message_count);
                    println!("  Created:  {}", session.created_at);
                    println!("  Updated:  {}", session.updated_at);
                    println!("\nMessages:");
                    for m in &messages {
                        let content = m.content.as_deref().unwrap_or("(empty)");
                        let preview = if content.len() > 80 {
                            let end = content
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i <= 77)
                                .last()
                                .unwrap_or(0);
                            format!("{}...", &content[..end])
                        } else {
                            content.to_string()
                        };
                        println!("  [{}] {}: {}", m.created_at, m.role, preview);
                    }
                }
            }
            None => {
                eprintln!("Session not found: {session_id}");
                std::process::exit(1);
            }
        },
        SessionAction::Delete { session_id } => {
            let deleted = store.delete_session(&session_id).await?;
            if deleted {
                println!("Session {session_id} deleted.");
            } else {
                eprintln!("Session not found: {session_id}");
                std::process::exit(1);
            }
        }
        SessionAction::Cleanup { ttl_hours } => {
            let count = store.cleanup_expired(ttl_hours).await?;
            println!("Cleaned up {count} expired session(s) (TTL: {ttl_hours}h).");
        }
    }

    Ok(())
}

// --- Agents ---

fn cmd_agents(action: AgentAction, as_json: bool) -> anyhow::Result<()> {
    let agents_dir = PathBuf::from("config/agents");
    let agents = fastclaw_core::agent_config::load_agent_configs(&agents_dir)?;

    match action {
        AgentAction::List => {
            if as_json {
                let items: Vec<_> = agents
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "id": a.agent_id,
                            "name": a.name,
                            "model": a.model.model,
                            "provider": a.model.provider,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "agents": items,
                        "count": agents.len(),
                    }))?
                );
            } else {
                if agents.is_empty() {
                    println!("No agents configured in config/agents/.");
                } else {
                    println!("{:<15} {:<25} {:<15} {}", "ID", "Name", "Provider", "Model");
                    println!("{}", "-".repeat(70));
                    for a in &agents {
                        let name = a.name.as_deref().unwrap_or("(unnamed)");
                        println!(
                            "{:<15} {:<25} {:<15} {}",
                            a.agent_id, name, a.model.provider, a.model.model
                        );
                    }
                    println!("\n{} agent(s)", agents.len());
                }
            }
        }
        AgentAction::Get { agent_id } => match agents.iter().find(|a| a.agent_id == agent_id) {
            Some(agent) => {
                let name = agent.name.as_deref().unwrap_or("(unnamed)");
                let desc = agent.description.as_deref().unwrap_or("(none)");
                let prompt = agent.system_prompt.as_deref().unwrap_or("");
                let tool_ids: Vec<_> = agent.tools.iter().map(|t| t.id.as_str()).collect();

                if as_json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "id": agent.agent_id,
                            "name": name,
                            "description": desc,
                            "model": {
                                "provider": agent.model.provider,
                                "model": agent.model.model,
                                "temperature": agent.model.temperature,
                                "max_tokens": agent.model.max_tokens,
                                "fallbacks_count": agent.model.fallbacks.len(),
                            },
                            "system_prompt_length": prompt.len(),
                            "tools": tool_ids,
                        }))?
                    );
                } else {
                    println!("Agent: {}", agent.agent_id);
                    println!("  Name:        {}", name);
                    println!("  Description: {}", desc);
                    println!("  Provider:    {}", agent.model.provider);
                    println!("  Model:       {}", agent.model.model);
                    println!("  Temperature: {}", agent.model.temperature);
                    if let Some(mt) = agent.model.max_tokens {
                        println!("  Max Tokens:  {}", mt);
                    }
                    println!("  Fallbacks:   {}", agent.model.fallbacks.len());
                    println!(
                        "  Tools:       {}",
                        if tool_ids.is_empty() {
                            "(all built-in)".to_string()
                        } else {
                            tool_ids.join(", ")
                        }
                    );
                    let prompt_preview = if prompt.len() > 100 {
                        let end = prompt
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 97)
                            .last()
                            .unwrap_or(0);
                        format!("{}...", &prompt[..end])
                    } else {
                        prompt.to_string()
                    };
                    println!("  Prompt:      {}", prompt_preview);
                }
            }
            None => {
                eprintln!("Agent not found: {agent_id}");
                eprintln!(
                    "Available: {}",
                    agents
                        .iter()
                        .map(|a| a.agent_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(1);
            }
        },
    }
    Ok(())
}

// --- Tools ---

fn cmd_tools(action: ToolAction, as_json: bool) -> anyhow::Result<()> {
    match action {
        ToolAction::List => {
            let registry = fastclaw_core::tool::ToolRegistry::new();
            fastclaw_agent::builtin_tools::register_builtin_tools(&registry);
            let tools = registry.definitions();
            if as_json {
                let items: Vec<_> = tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.function.name,
                            "description": t.function.description,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "tools": items,
                        "count": tools.len(),
                    }))?
                );
            } else {
                println!("{:<25} {}", "Name", "Description");
                println!("{}", "-".repeat(70));
                for t in &tools {
                    println!("{:<25} {}", t.function.name, t.function.description);
                }
                println!("\n{} tool(s)", tools.len());
            }
        }
    }
    Ok(())
}

// --- Setup / Onboard ---

fn prompt_line(msg: &str) -> String {
    eprint!("{msg}");
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).unwrap_or_default();
    buf.trim().to_string()
}

fn cmd_setup(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    let sd = state_dir(dev, profile);
    let cfg_path = sd.join("config/default.json");

    println!("FastClaw Setup");
    println!("{}", "=".repeat(40));

    if cfg_path.exists() {
        println!("Config already exists at: {}", cfg_path.display());
        let answer = prompt_line("Overwrite? [y/N] ");
        if !answer.eq_ignore_ascii_case("y") {
            println!("Setup cancelled.");
            return Ok(());
        }
    }

    println!("\nChoose your primary LLM provider:");
    println!("  1. OpenAI (default)");
    println!("  2. Anthropic");
    println!("  3. DashScope (Alibaba/Qwen)");
    println!("  4. DeepSeek");
    println!("  5. Google Gemini");
    println!("  6. Ollama (local)");
    println!("  7. Custom (OpenAI-compatible)");
    let choice = prompt_line("Select [1-7, default=1]: ");

    let (provider, model, _base_url_env, key_env) = match choice.as_str() {
        "2" => (
            "anthropic",
            "claude-sonnet-4-20250514",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_KEY",
        ),
        "3" => (
            "dashscope",
            "qwen3.5-plus",
            "DASHSCOPE_BASE_URL",
            "DASHSCOPE_API_KEY",
        ),
        "4" => (
            "deepseek",
            "deepseek-chat",
            "DEEPSEEK_BASE_URL",
            "DEEPSEEK_API_KEY",
        ),
        "5" => (
            "google",
            "gemini-2.5-flash",
            "GOOGLE_BASE_URL",
            "GOOGLE_API_KEY",
        ),
        "6" => ("ollama", "llama3.1:8b", "OLLAMA_BASE_URL", ""),
        "7" => {
            let p = prompt_line("Provider name: ");
            let m = prompt_line("Model name: ");
            let b = prompt_line("Base URL env var [OPENAI_BASE_URL]: ");
            let k = prompt_line("API key env var [OPENAI_API_KEY]: ");
            let p_l = Box::leak(p.into_boxed_str());
            let m_l = Box::leak(m.into_boxed_str());
            let b_l = Box::leak(
                if b.is_empty() {
                    "OPENAI_BASE_URL".to_string()
                } else {
                    b
                }
                .into_boxed_str(),
            );
            let k_l = Box::leak(
                if k.is_empty() {
                    "OPENAI_API_KEY".to_string()
                } else {
                    k
                }
                .into_boxed_str(),
            );
            (p_l as &str, m_l as &str, b_l as &str, k_l as &str)
        }
        _ => ("openai", "gpt-4o", "OPENAI_BASE_URL", "OPENAI_API_KEY"),
    };

    let mut api_key_value = String::new();
    if !key_env.is_empty() {
        let key = prompt_line(&format!(
            "\nEnter API key for {} (or press Enter to skip): ",
            provider
        ));
        if !key.is_empty() {
            api_key_value = key;
        }
    }

    let port_str = prompt_line("\nGateway port [18789]: ");
    let port: u16 = port_str.parse().unwrap_or(18789);

    let api_key_str = prompt_line("Gateway API key (for authentication, empty=disabled): ");

    // Create config
    std::fs::create_dir_all(sd.join("config"))?;
    std::fs::create_dir_all(sd.join("data"))?;

    let mut credentials = serde_json::Map::new();
    if !api_key_value.is_empty() {
        credentials.insert(
            provider.to_string(),
            serde_json::json!({
                "apiKey": api_key_value
            }),
        );
    }

    let mut config = serde_json::json!({
        "gateway": { "port": port },
        "logging": { "level": "info", "format": "pretty" },
    });
    if !credentials.is_empty() {
        config["credentials"] = serde_json::Value::Object(credentials);
    }
    if !api_key_str.is_empty() {
        config["security"] = serde_json::json!({ "apiKeys": [api_key_str] });
    }
    let config = config;
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&config)?)?;
    println!("\nConfig written to: {}", cfg_path.display());

    // Create agents dir and default agent
    let agents_dir = PathBuf::from("config/agents");
    std::fs::create_dir_all(&agents_dir)?;
    let agent = serde_json::json!({
        "agentId": "main",
        "name": "Main Agent",
        "description": "Default assistant agent",
        "model": {
            "provider": provider,
            "model": model,
        },
        "systemPrompt": "You are a helpful AI assistant powered by FastClaw.",
        "tools": [],
    });
    let agent_path = agents_dir.join("main.json");
    if !agent_path.exists() {
        std::fs::write(&agent_path, serde_json::to_string_pretty(&agent)?)?;
        println!("Agent config written to: {}", agent_path.display());
    }

    if !api_key_str.is_empty() {
        println!("\nTo enable authentication, set:");
        println!("  export FASTCLAW_API_KEYS={}", api_key_str);
    }

    println!("\nSetup complete! Start the gateway with:");
    println!("  fastclaw serve");
    println!(
        "  fastclaw tui{}",
        if api_key_str.is_empty() {
            String::new()
        } else {
            format!(" --token {}", api_key_str)
        }
    );

    Ok(())
}

fn cmd_onboard(dev: bool, profile: Option<&str>) -> anyhow::Result<()> {
    println!("Welcome to FastClaw!");
    println!("{}", "=".repeat(40));
    println!();
    println!("FastClaw is a high-performance AI agent orchestration engine.");
    println!("This wizard will help you get started.\n");
    println!("What FastClaw offers:");
    println!("  - Multi-agent orchestration with tool calling");
    println!("  - WebSocket & HTTP APIs for real-time chat");
    println!("  - Interactive terminal UI (TUI)");
    println!("  - MCP server/client for interop");
    println!("  - DAG workflow execution");
    println!("  - Session persistence & memory");
    println!("  - Plugin system (WASM)");
    println!();

    let ready = prompt_line("Ready to configure? [Y/n] ");
    if ready.eq_ignore_ascii_case("n") {
        println!("\nYou can run `fastclaw setup` anytime to configure.");
        return Ok(());
    }

    cmd_setup(dev, profile)?;

    println!("\n--- Quick Start Guide ---");
    println!("1. Start gateway:  fastclaw serve");
    println!("2. Open TUI:       fastclaw tui");
    println!("3. Health check:   fastclaw health");
    println!("4. Diagnostics:    fastclaw doctor");
    println!("5. Web UI:         http://localhost:<port>/  (see `fastclaw config get gateway.port`)");
    println!("\nDocumentation: https://github.com/your-org/fastclaw");

    Ok(())
}

// --- MCP Server ---

async fn cmd_mcp_server() -> anyhow::Result<()> {
    let registry = fastclaw_core::tool::ToolRegistry::new();
    fastclaw_agent::builtin_tools::register_builtin_tools(&registry);

    let tool_registry = std::sync::Arc::new(registry);
    let server = fastclaw_collab::create_fastclaw_mcp_server(tool_registry);
    server.run_stdio().await
}
