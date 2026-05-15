use std::time::Instant;

use futures::SinkExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::state::*;
use super::WsTx;

pub(crate) async fn handle_slash_command(app: &mut TuiApp, ws_tx: &mut WsTx, text: &str) {
    app.input.clear();
    app.cursor_pos = 0;

    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).copied().unwrap_or("");

    match cmd {
        "/help" => {
            app.show_popup = Some(PopupKind::Help);
        }
        "/quit" | "/exit" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.messages.clear();
            app.scroll_offset = 0;
        }
        "/new" => {
            app.messages.clear();
            app.session_id = None;
            app.scroll_offset = 0;
            app.push_system("New session started.".into());
        }
        "/agent" if !arg.is_empty() => {
            if app.agents.iter().any(|a| a.id == arg) {
                app.agent_id = arg.to_string();
                app.push_system(format!("Switched to agent: {arg}"));
            } else {
                let available: Vec<_> = app.agents.iter().map(|a| a.id.as_str()).collect();
                app.push_system(format!(
                    "Agent '{arg}' not found. Available: {}",
                    available.join(", ")
                ));
            }
        }
        "/agent" => {
            app.push_system(format!("Current agent: {}", app.agent_id));
        }
        "/agents" => {
            app.show_popup = Some(PopupKind::Agents);
        }
        "/model" if !arg.is_empty() => {
            let resolved = if let Ok(idx) = arg.parse::<usize>() {
                app.models_cache
                    .get(idx.saturating_sub(1))
                    .map(|(_, m)| m.clone())
            } else {
                let found = app
                    .models_cache
                    .iter()
                    .find(|(p, m)| m == arg || format!("{p}/{m}") == arg);
                found
                    .map(|(_, m)| m.clone())
                    .or_else(|| Some(arg.to_string()))
            };
            if let Some(model_name) = resolved {
                app.model_override = model_name.clone();
                app.current_model = model_name.clone();
                app.push_system(format!("Model switched to: {model_name}"));
                app.push_system(
                    "This override applies to all subsequent messages in this session.".into(),
                );
                app.status = format!("Model: {model_name}");
            }
        }
        "/model" => {
            if app.models_cache.is_empty() {
                let active = if app.model_override.is_empty() {
                    app.agents
                        .iter()
                        .find(|a| a.id == app.agent_id)
                        .map(|a| format!("{} (agent default)", a.model))
                        .unwrap_or_else(|| "unknown".into())
                } else {
                    format!("{} (user override)", app.model_override)
                };
                app.push_system(format!("Current model: {active}"));
                app.push_system(
                    "Use /models to fetch list, then /model to open interactive picker.".into(),
                );
            } else {
                let current = if app.model_override.is_empty() {
                    &app.current_model
                } else {
                    &app.model_override
                };
                let items: Vec<SelectItem> = app
                    .models_cache
                    .iter()
                    .map(|(provider, model)| SelectItem {
                        id: model.clone(),
                        label: model.clone(),
                        description: provider.clone(),
                        is_current: model == current,
                    })
                    .collect();
                app.select_state = Some(SelectState::new(items));
                app.show_popup = Some(PopupKind::ModelPicker);
            }
        }
        "/stats" => {
            if app.total_messages == 0 {
                app.push_system("No messages in this session yet.".into());
            } else {
                app.push_system(format!(
                    "Session stats: {} message(s), {} total, ↑{} ↓{} tokens ({}+{} = {} total tok)",
                    app.total_messages,
                    format_elapsed(app.total_elapsed_ms),
                    app.total_input_tokens,
                    app.total_output_tokens,
                    app.total_input_tokens,
                    app.total_output_tokens,
                    app.total_input_tokens + app.total_output_tokens,
                ));
                if let (Some(ms), Some(i), Some(o)) = (
                    app.last_elapsed_ms,
                    app.last_input_tokens,
                    app.last_output_tokens,
                ) {
                    app.push_system(format!(
                        "Last message: {} | ↑{} ↓{} tokens",
                        format_elapsed(ms),
                        i,
                        o,
                    ));
                }
            }
        }
        "/doctor" => {
            run_preflight_checks(app);
            if app.messages.last().is_none_or(|m| m.role != "system") {
                app.push_system("All checks passed.".into());
            }
        }
        "/plan" => {
            let new_mode = if app.execution_mode == "plan" {
                "agent"
            } else {
                "plan"
            };
            let id = app.next_id();
            let req = json!({
                "id": id,
                "method": "chat.set_mode",
                "params": {"mode": new_mode}
            });
            if ws_tx.send(Message::Text(req.to_string())).await.is_err() {
                app.push_system("Failed to send mode switch request.".into());
            } else {
                app.push_system(format!("Switching to {new_mode} mode..."));
            }
        }
        "/cancel" => {
            if app.streaming {
                if let Some(rid) = app.last_request_id.take() {
                    let cancel_id = app.next_id();
                    let cancel_req = json!({"id": cancel_id, "method": "chat.cancel", "params": {"requestId": rid}});
                    let _ = ws_tx.send(Message::Text(cancel_req.to_string())).await;
                }
                app.streaming = false;
                app.status = "Cancelled".into();
            } else {
                app.push_system("Nothing to cancel.".into());
            }
        }
        "/ping" => {
            let id = app.next_id();
            let ping_start = Instant::now();
            let req = json!({"id": id, "method": "ping"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system(format!(
                "Ping sent... (local send took {}μs)",
                ping_start.elapsed().as_micros()
            ));
        }
        "/models" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "models.list"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.status = "Loading models...".into();
        }
        "/mcp" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "mcp.status"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.status = "Loading MCP status...".into();
        }
        "/export" => {
            if app.messages.is_empty() {
                app.push_system("No messages to export.".into());
            } else {
                let mut export = String::new();
                for msg in &app.messages {
                    export.push_str(&format!(
                        "[{}] {}: {}\n",
                        msg.timestamp, msg.role, msg.content
                    ));
                }
                let sid = app
                    .session_id
                    .as_deref()
                    .unwrap_or("unsaved")
                    .chars()
                    .take(12)
                    .collect::<String>();
                let filename = format!("fastclaw-session-{sid}.txt");
                match std::fs::write(&filename, &export) {
                    Ok(_) => app.push_system(format!(
                        "Exported {} messages to {filename}",
                        app.messages.len()
                    )),
                    Err(e) => app.push_system(format!("Export failed: {e}")),
                }
            }
        }
        "/sessions" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "sessions.list", "params": {"limit": 10}});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.status = "Loading sessions...".into();
        }
        "/resume" if !arg.is_empty() => {
            app.session_id = Some(arg.to_string());
            app.messages.clear();
            app.scroll_offset = 0;

            let claim_id = app.next_id();
            let claim_req =
                json!({"id": claim_id, "method": "sessions.claim", "params": {"sessionId": arg}});
            let _ = ws_tx.send(Message::Text(claim_req.to_string())).await;

            let id = app.next_id();
            let req =
                json!({"id": id, "method": "sessions.messages", "params": {"sessionId": arg}});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system(format!("Resuming session: {}", &arg[..arg.len().min(12)]));
        }
        "/resume" => {
            app.push_system("Usage: /resume <session-id>".into());
        }
        "/compact" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "chat.compact"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system("Compacting context...".into());
        }
        "/todo" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "todo.list"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.status = "Loading todos...".into();
        }
        "/memory" if !arg.is_empty() => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "memory.search", "params": {"query": arg}});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system(format!("Searching memory: {arg}"));
        }
        "/memory" => {
            app.push_system("Usage: /memory <search query>".into());
        }
        "/undo" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "chat.undo"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system("Reverting last file change...".into());
        }
        "/diff" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "chat.diff"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
            app.push_system("Fetching recent changes...".into());
        }
        "/cost" => {
            let total_tok = app.total_input_tokens + app.total_output_tokens;
            if total_tok == 0 {
                app.push_system("No token usage yet.".into());
            } else {
                let est_cost_usd =
                    (app.total_input_tokens as f64 * 3.0 + app.total_output_tokens as f64 * 15.0)
                        / 1_000_000.0;
                app.push_system(format!(
                    "Cost estimate: ${est_cost_usd:.4} (↑{} ↓{} = {total_tok} tokens)",
                    app.total_input_tokens, app.total_output_tokens,
                ));
                app.push_system(
                    "Note: estimate uses generic pricing; actual cost depends on model.".into(),
                );
            }
        }
        "/copy" => {
            if let Some(last_assistant) = app
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant" && !m.content.is_empty())
            {
                app.push_system(format!(
                    "Last response ({} chars) — use terminal copy.",
                    last_assistant.content.len()
                ));
            } else {
                app.push_system("No assistant response to copy.".into());
            }
        }
        "/config" => {
            app.push_system(format!("Agent: {}", app.agent_id));
            app.push_system(format!("Mode: {}", app.execution_mode));
            app.push_system(format!(
                "Model: {}",
                if app.model_override.is_empty() {
                    &app.current_model
                } else {
                    &app.model_override
                }
            ));
            if let Some(ref wd) = app.work_dir {
                app.push_system(format!("Work dir: {wd}"));
            }
            if let Some(ref sid) = app.session_id {
                app.push_system(format!("Session: {}", &sid[..sid.len().min(12)]));
            }
        }
        "/context" => {
            if app.ctx_limit_tokens == 0 {
                app.push_system("Context window info not yet available. Send a message first.".into());
            } else {
                let pct =
                    (app.ctx_used_tokens as f64 / app.ctx_limit_tokens as f64 * 100.0) as u32;
                app.push_system(format!(
                    "Context: {}/{}k tokens ({pct}%)",
                    app.ctx_used_tokens / 1000,
                    app.ctx_limit_tokens / 1000,
                ));
                let remaining = app.ctx_limit_tokens.saturating_sub(app.ctx_used_tokens);
                app.push_system(format!(
                    "Remaining: ~{}k tokens",
                    remaining / 1000,
                ));
                if pct >= 80 {
                    app.push_system(
                        "⚠ Consider using /compact or starting a new session.".into(),
                    );
                }
            }
        }
        _ => {
            app.push_system(format!(
                "Unknown command: {cmd}. Type /help for available commands."
            ));
        }
    }
}

pub(crate) fn run_preflight_checks(app: &mut TuiApp) {
    let mut warnings: Vec<String> = Vec::new();

    if app.agents.is_empty() {
        warnings.push("No agents found. Configure agents in config/agents/.".into());
    }

    match fastclaw_core::config::load_config(&app.config_mode) {
        Ok(config) => {
            let has_creds = !config.credentials.providers.is_empty()
                && config
                    .credentials
                    .providers
                    .values()
                    .any(|c| c.api_key.is_some());
            if !has_creds {
                warnings.push(
                    "No LLM API keys configured. Run `fastclaw setup` or `fastclaw config set`."
                        .into(),
                );
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("json") || msg.contains("parse") || msg.contains("JSON") {
                warnings.push(format!(
                    "Config syntax error (try `fastclaw config fix`): {msg}"
                ));
            } else {
                warnings.push(format!("Config error: {msg}"));
            }
        }
    }

    if let Some(wd) = &app.work_dir {
        if !std::path::Path::new(wd).exists() {
            warnings.push(format!("Workspace dir not accessible: {wd}"));
        }
    }

    if !warnings.is_empty() {
        app.push_system(format!(
            "Preflight: {} issue{}",
            warnings.len(),
            if warnings.len() == 1 { "" } else { "s" }
        ));
        for w in warnings {
            app.push_system(format!("  - {w}"));
        }
    }
}
