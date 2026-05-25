use std::process::ExitCode;

use fastclaw_core::types::{ChatMessage, ChatRequest, Role};
use fastclaw_gateway::AppState;
use fastclaw_protocol::AgentEvent;

/// Approval policy for non-interactive exec mode.
#[derive(Debug, Clone, Copy)]
pub enum ApprovalPolicy {
    /// Automatically approve all tool calls.
    AutoApprove,
    /// Deny all tool calls that require approval.
    DenyAll,
    /// Use ExecPolicy rules (default).
    PolicyBased,
}

/// Run a single prompt in non-interactive mode.
///
/// Builds an in-process AppState (no HTTP listener), creates a temporary session,
/// executes the prompt, and outputs the result.
pub async fn run_exec(
    prompt: &str,
    json_output: bool,
    approval_policy: ApprovalPolicy,
    mode: &fastclaw_core::config::ConfigMode,
) -> ExitCode {
    let config = match fastclaw_core::config::load_config(mode) {
        Ok(c) => c,
        Err(e) => {
            output_error(json_output, &format!("config error: {e}"));
            return ExitCode::FAILURE;
        }
    };

    fastclaw_gateway::set_config_mode(mode.clone());

    let state = match AppState::new(config).await {
        Ok(s) => s,
        Err(e) => {
            output_error(json_output, &format!("init error: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let messages = vec![ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(prompt.to_string())),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        compact_metadata: None,
    }];

    let request = ChatRequest {
        model: None,
        messages: messages.clone(),
        stream: true,
        temperature: None,
        max_tokens: None,
        tools: None,
        session_id: None,
        agent_id: None,
        slash_intent: None,
        work_dir: None,
    };

    let agent_config = {
        let router = state.rt.router.read().await;
        match router.resolve(&request) {
            Ok(config) => config.clone(),
            Err(e) => {
                output_error(json_output, &format!("no agent: {e}"));
                return ExitCode::FAILURE;
            }
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(1024);

    let orchestrator = match approval_policy {
        ApprovalPolicy::AutoApprove | ApprovalPolicy::DenyAll => None,
        ApprovalPolicy::PolicyBased => Some(state.strm.tool_orchestrator.clone()),
    };

    let runtime = state.rt.runtime.clone();
    let tool_reg = state.rt.tool_registry.clone();
    let config_clone = agent_config.clone();
    let request_clone = request.clone();
    let confirm_pending = state.strm.ask_question_pending.clone();

    tokio::spawn(async move {
        let result = runtime
            .execute_stream_with_confirm(
                &config_clone,
                &request_clone,
                &tool_reg,
                tx.clone(),
                None,
                confirm_pending,
                None,
                None,
                None,
                None,
                orchestrator,
            )
            .await;
        if let Err(e) = result {
            let _ = tx
                .send(AgentEvent::Error {
                    turn_id: fastclaw_protocol::TurnId::generate(),
                    message: e.to_string(),
                    error_code: None,
                })
                .await;
        }
    });

    let mut assistant_content = String::new();
    let mut exit_code = ExitCode::SUCCESS;

    while let Some(event) = rx.recv().await {
        if json_output {
            if let Ok(json_str) = serde_json::to_string(&event) {
                println!("{json_str}");
            }
        }

        match &event {
            AgentEvent::ContentDelta { delta, .. } => {
                if let Some(choices) = delta.get("choices").and_then(|c| c.as_array()) {
                    for choice in choices {
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d: &serde_json::Value| d.get("content"))
                            .and_then(|c: &serde_json::Value| c.as_str())
                        {
                            assistant_content.push_str(content);
                            if !json_output {
                                print!("{content}");
                            }
                        }
                    }
                }
            }
            AgentEvent::ApprovalRequired {
                approval_id, ..
            } => {
                let decision = match approval_policy {
                    ApprovalPolicy::AutoApprove => {
                        fastclaw_protocol::ApprovalDecision::Approved
                    }
                    ApprovalPolicy::DenyAll | ApprovalPolicy::PolicyBased => {
                        fastclaw_protocol::ApprovalDecision::Denied
                    }
                };
                state.strm.tool_orchestrator.resolve(approval_id, decision);
            }
            AgentEvent::TurnEnd { .. } => {
                break;
            }
            AgentEvent::Error { message, .. } => {
                if !json_output {
                    eprintln!("\nError: {message}");
                }
                exit_code = ExitCode::FAILURE;
                break;
            }
            _ => {}
        }
    }

    if !json_output && !assistant_content.is_empty() {
        println!();
    }

    exit_code
}

fn output_error(json_output: bool, message: &str) {
    if json_output {
        let _ = serde_json::to_writer(
            std::io::stdout(),
            &serde_json::json!({"type": "error", "message": message}),
        );
        println!();
    } else {
        eprintln!("Error: {message}");
    }
}
