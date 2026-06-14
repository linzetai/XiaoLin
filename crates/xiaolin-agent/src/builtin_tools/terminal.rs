use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio::time::timeout;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_pty::{PtySessionConfig, PtySessionManager};

const MAX_AGENT_SESSIONS: usize = 3;
const DEFAULT_WAIT_MS: u64 = 2000;
const MAX_WAIT_MS: u64 = 30000;
const MAX_OUTPUT_BYTES: usize = 32_000;

pub struct TerminalOpenTool {
    pty_manager: Arc<PtySessionManager>,
}

impl TerminalOpenTool {
    pub fn new(pty_manager: Arc<PtySessionManager>) -> Self {
        Self { pty_manager }
    }
}

#[async_trait]
impl Tool for TerminalOpenTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "terminal_open"
    }

    fn description(&self) -> &str {
        "Open a persistent interactive terminal session visible to the user. \
         Use for REPLs, dev servers, debuggers, long-running processes, or multi-step \
         workflows requiring sequential commands in the same shell session. \
         The terminal appears in the user's Shell tab. \
         For quick one-shot commands that finish within 2 minutes, prefer shell_exec instead."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "cwd".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Working directory for the terminal session. Defaults to project root."
            }),
        );
        props.insert(
            "name".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Display name for the terminal tab (e.g. 'Dev Server', 'Debug Session')."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: Vec::new(),
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        if self.pty_manager.count_by_source("agent") >= MAX_AGENT_SESSIONS {
            return ToolResult::err(format!(
                "Agent session limit reached ({MAX_AGENT_SESSIONS}). Close an existing terminal before opening a new one."
            ));
        }

        let cwd = args.get("cwd").and_then(|v| v.as_str()).map(String::from);
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Agent Terminal")
            .to_string();

        let config = PtySessionConfig {
            cwd,
            source: "agent".to_string(),
            ..Default::default()
        };

        let (session_id, mut rx) = match self.pty_manager.create_session_with_subscriber(config) {
            Ok(pair) => pair,
            Err(e) => return ToolResult::err(format!("Failed to create terminal: {e}")),
        };

        let initial_output = collect_output(&mut rx, Duration::from_millis(500), None).await;

        let result = serde_json::json!({
            "session_id": session_id,
            "name": name,
            "initial_output": initial_output,
        });
        ToolResult::ok(serde_json::to_string(&result).unwrap())
    }
}

pub struct TerminalInputTool {
    pty_manager: Arc<PtySessionManager>,
}

impl TerminalInputTool {
    pub fn new(pty_manager: Arc<PtySessionManager>) -> Self {
        Self { pty_manager }
    }
}

#[async_trait]
impl Tool for TerminalInputTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "terminal_input"
    }

    fn description(&self) -> &str {
        "Send input to an existing interactive terminal session and collect the output. \
         Use wait_for to wait until specific text appears (e.g. a prompt or 'Server ready'). \
         The input is visible to the user in real-time in the Shell tab."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "session_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The session_id returned by terminal_open."
            }),
        );
        props.insert(
            "input".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Text to send to the terminal. Include \\n for Enter key."
            }),
        );
        props.insert(
            "wait_ms".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Max milliseconds to wait for output (default 2000, max 30000)."
            }),
        );
        props.insert(
            "wait_for".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Stop collecting output early when this text appears in the output."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["session_id".to_string(), "input".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let session_id = match args.get("session_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolResult::err("Missing required field 'session_id'.".to_string()),
        };

        let input = match args.get("input").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("Missing required field 'input'.".to_string()),
        };

        let wait_ms = args
            .get("wait_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_WAIT_MS)
            .min(MAX_WAIT_MS);

        let wait_for = args.get("wait_for").and_then(|v| v.as_str()).map(String::from);

        let mut rx = match self.pty_manager.subscribe(session_id) {
            Some(rx) => rx,
            None => {
                return ToolResult::err(format!(
                    "Session '{session_id}' not found. It may have been closed."
                ))
            }
        };

        let write_result = self
            .pty_manager
            .get_session(session_id, |s| s.write_input(input.as_bytes()));
        match write_result {
            Some(Ok(())) => {}
            Some(Err(e)) => return ToolResult::err(format!("Failed to write input: {e}")),
            None => {
                return ToolResult::err(format!(
                    "Session '{session_id}' not found. It may have been closed."
                ))
            }
        }

        let output =
            collect_output(&mut rx, Duration::from_millis(wait_ms), wait_for.as_deref()).await;

        let alive = self
            .pty_manager
            .get_session(session_id, |s| s.is_alive())
            .unwrap_or(false);

        let result = serde_json::json!({
            "output": output,
            "alive": alive,
        });
        ToolResult::ok(serde_json::to_string(&result).unwrap())
    }
}

pub struct TerminalCloseTool {
    pty_manager: Arc<PtySessionManager>,
}

impl TerminalCloseTool {
    pub fn new(pty_manager: Arc<PtySessionManager>) -> Self {
        Self { pty_manager }
    }
}

#[async_trait]
impl Tool for TerminalCloseTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "terminal_close"
    }

    fn description(&self) -> &str {
        "Close an interactive terminal session. The session will be terminated and \
         removed from the user's Shell tab."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "session_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The session_id to close."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["session_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let session_id = match args.get("session_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolResult::err("Missing required field 'session_id'.".to_string()),
        };

        self.pty_manager.close_session(session_id);
        ToolResult::ok(format!("{{\"closed\": \"{session_id}\"}}"))
    }
}

pub fn register_terminal_tools(registry: &xiaolin_core::tool::ToolRegistry, pty_manager: Arc<PtySessionManager>) {
    registry.register(Arc::new(TerminalOpenTool::new(pty_manager.clone())));
    registry.register(Arc::new(TerminalInputTool::new(pty_manager.clone())));
    registry.register(Arc::new(TerminalCloseTool::new(pty_manager)));
}

async fn collect_output(
    rx: &mut broadcast::Receiver<Vec<u8>>,
    max_duration: Duration,
    wait_for: Option<&str>,
) -> String {
    let mut collected = Vec::new();
    let deadline = tokio::time::Instant::now() + max_duration;
    let idle_timeout = Duration::from_millis(500);
    let mut has_received_data = false;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let wait_time = if has_received_data && wait_for.is_none() {
            remaining.min(idle_timeout)
        } else {
            remaining
        };

        match timeout(wait_time, rx.recv()).await {
            Ok(Ok(data)) => {
                collected.extend_from_slice(&data);
                has_received_data = true;
                if collected.len() > MAX_OUTPUT_BYTES {
                    collected.truncate(MAX_OUTPUT_BYTES);
                    break;
                }
                if let Some(pattern) = wait_for {
                    let text = String::from_utf8_lossy(&collected);
                    if text.contains(pattern) {
                        break;
                    }
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => break, // idle timeout or deadline reached
        }
    }

    String::from_utf8_lossy(&collected).to_string()
}
