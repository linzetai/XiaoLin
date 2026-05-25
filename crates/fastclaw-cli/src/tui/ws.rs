use std::time::Instant;

use futures::SinkExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use super::state::*;
use super::WsTx;

pub(crate) async fn send_chat(app: &mut TuiApp, ws_tx: &mut WsTx) {
    let text = app.input.drain(..).collect::<String>();
    app.cursor_pos = 0;

    app.messages.push(ChatMsg {
        role: "user".into(),
        content: text.clone(),
        timestamp: now_hms(),
    });

    let id = app.next_id();
    let mut params = json!({
        "messages": [{"role": "user", "content": text}],
        "agentId": app.agent_id,
    });
    if let Some(sid) = &app.session_id {
        params["sessionId"] = json!(sid);
    }
    if let Some(wd) = &app.work_dir {
        params["workDir"] = json!(wd);
    }
    if !app.model_override.is_empty() {
        params["model"] = json!(app.model_override);
    }

    let req = json!({"id": id, "method": "chat", "params": params});
    let _ = ws_tx.send(Message::Text(req.to_string())).await;

    app.last_request_id = Some(id);
    app.chat_start_time = Some(Instant::now());
    app.streaming = true;
    app.timeout_warned = false;
    app.spinner.set_thinking();
    app.scroll_offset = 0;
    app.messages.push(ChatMsg {
        role: "assistant".into(),
        content: String::new(),
        timestamp: now_hms(),
    });
}

pub(crate) fn handle_ws_message(app: &mut TuiApp, text: &str) {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };

    let msg_type = msg["type"].as_str().unwrap_or("");
    match msg_type {
        "connected" => {
            app.status = "Connected".into();
        }
        "turn_start" => {
            if let Some(sid) = msg["data"]["session_id"]
                .as_str()
                .or_else(|| msg["data"]["sessionId"].as_str())
            {
                app.session_id = Some(sid.to_string());
            }
            app.spinner.set_thinking();
        }
        "content_delta" => {
            let delta = &msg["data"]["delta"];
            let reasoning = delta
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("delta"))
                .and_then(|d| d.get("reasoning_content"))
                .and_then(|v| v.as_str());
            let content = delta
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("delta"))
                .and_then(|d| d.get("content"))
                .and_then(|v| v.as_str());
            if let Some(thinking) = reasoning {
                app.thinking_content.push_str(thinking);
                app.spinner.verb = "thinking".into();
            } else if let Some(text) = content {
                if !app.thinking_content.is_empty() {
                    if let Some(last) = app.messages.last_mut() {
                        if last.role == "assistant" {
                            let thinking_lines = app.thinking_content.lines().count();
                            last.content.push_str(&format!(
                                "\n∴ *Thinking ({thinking_lines} lines)* — toggle with Ctrl+K\n"
                            ));
                        }
                    }
                    app.thinking_content.clear();
                }
                if let Some(last) = app.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(text);
                    }
                }
                app.spinner.verb = "writing".into();
                app.spinner.tool_name = None;
            }
            app.scroll_offset = 0;
        }
        "tool_executing" => {
            let tool = msg["data"]["tool_name"].as_str().unwrap_or("unknown");
            app.spinner.set_tool(tool);
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" {
                    let readable_name = tool_display_name(tool);
                    let args_summary = msg["data"]["args"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .and_then(|obj| {
                            obj.get("path")
                                .or_else(|| obj.get("command"))
                                .or_else(|| obj.get("query"))
                                .or_else(|| obj.get("pattern"))
                                .or_else(|| obj.get("url"))
                                .or_else(|| obj.get("content"))
                                .and_then(|v| v.as_str())
                                .map(|s| {
                                    if s.chars().count() > 60 {
                                        let truncated: String = s.chars().take(60).collect();
                                        format!("{truncated}…")
                                    } else {
                                        s.to_string()
                                    }
                                })
                        })
                        .unwrap_or_default();
                    let tool_line = if args_summary.is_empty() {
                        format!("\n● {readable_name}\n")
                    } else {
                        format!("\n● {readable_name}: `{args_summary}`\n")
                    };
                    last.content.push_str(&tool_line);
                }
            }
        }
        "tool_result" => {
            app.spinner.set_thinking();
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" {
                    let status_icon = if msg["data"]["success"].as_bool().unwrap_or(true) {
                        "✓"
                    } else {
                        "✗"
                    };
                    last.content
                        .push_str(&format!("  ⎿ {status_icon}\n"));
                }
            }
        }
        "tool_progress" => {
            let content = msg["data"]["message"]
                .as_str()
                .or_else(|| msg["data"]["partial_output"].as_str());
            if let Some(text) = content {
                if let Some(last) = app.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(&format!("  ⎿ {text}\n"));
                    }
                }
            }
        }
        "context_usage_update" => {
            if let Some(used) = msg["data"]["used_tokens"].as_u64() {
                app.ctx_used_tokens = used as u32;
            }
            if let Some(limit) = msg["data"]["limit_tokens"].as_u64() {
                app.ctx_limit_tokens = limit as u32;
            }
        }
        "turn_end" => {
            app.streaming = false;

            let elapsed_ms = msg["data"]["elapsedMs"]
                .as_u64()
                .or_else(|| msg["data"]["summary"]["elapsed_ms"].as_u64())
                .or_else(|| app.chat_start_time.map(|t| t.elapsed().as_millis() as u64));
            let input_tokens = msg["data"]["inputTokensEstimate"]
                .as_u64()
                .or_else(|| {
                    msg["data"]["summary"]["usage"]["prompt_tokens"].as_u64()
                });
            let output_tokens = msg["data"]["outputTokensEstimate"]
                .as_u64()
                .or_else(|| {
                    msg["data"]["summary"]["usage"]["completion_tokens"].as_u64()
                });

            app.last_elapsed_ms = elapsed_ms;
            app.last_input_tokens = input_tokens;
            app.last_output_tokens = output_tokens;
            if let Some(ms) = elapsed_ms {
                app.total_elapsed_ms += ms;
            }
            if let Some(t) = input_tokens {
                app.total_input_tokens += t;
            }
            if let Some(t) = output_tokens {
                app.total_output_tokens += t;
            }
            app.total_messages += 1;
            app.chat_start_time = None;

            let time_str = format_elapsed(elapsed_ms.unwrap_or(0));
            let in_tok = input_tokens.unwrap_or(0);
            let out_tok = output_tokens.unwrap_or(0);
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" && !last.content.is_empty() {
                    last.content.push_str(&format!(
                        "\n---\n*{time_str} · ↑{in_tok} ↓{out_tok} tokens*\n"
                    ));
                }
            }

            app.status = format!("Ready · {time_str}");
        }
        "error" => {
            app.streaming = false;
            let err = msg["error"]["message"]
                .as_str()
                .or_else(|| msg["data"]["message"].as_str())
                .unwrap_or("unknown error");
            let code = msg["error"]["code"].as_i64();
            let status = match code {
                Some(401) => format!("Auth error: {err}"),
                Some(403) => format!("Access denied: {err}"),
                Some(404) => format!("Not found: {err}"),
                _ => format!("Error: {err}"),
            };
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" && last.content.is_empty() {
                    last.content = format!("[Error: {err}]");
                }
            }
            app.push_system(format!("[Error] {status}"));
            app.status = status;
        }
        "sessions.list" => {
            if let Some(sessions) = msg["data"]["sessions"].as_array() {
                app.show_popup = Some(PopupKind::Sessions(sessions.clone()));
                app.status = "Ready".into();
            }
        }
        "models.list" => {
            if let Some(models) = msg["data"]["models"].as_array() {
                app.models_cache.clear();
                for m in models {
                    let provider = m["provider"].as_str().unwrap_or("?").to_string();
                    let model = m["model"].as_str().unwrap_or("?").to_string();
                    app.models_cache.push((provider, model));
                }
                let active_model = if app.model_override.is_empty() {
                    app.agents
                        .iter()
                        .find(|a| a.id == app.agent_id)
                        .map(|a| a.model.clone())
                        .unwrap_or_default()
                } else {
                    app.model_override.clone()
                };
                let mut lines = vec![format!("Available models ({}):", app.models_cache.len())];
                for (i, (provider, model)) in app.models_cache.iter().enumerate() {
                    let marker = if *model == active_model {
                        " ◀ current"
                    } else {
                        ""
                    };
                    lines.push(format!("  {}. {provider}/{model}{marker}", i + 1));
                }
                lines.push(
                    "Use /model <number> or /model <provider/model> to switch.".into(),
                );
                for line in lines {
                    app.push_system(line);
                }
            }
            app.status = "Ready".into();
        }
        "mcp.status" => {
            if let Some(servers) = msg["data"]["servers"].as_array() {
                if servers.is_empty() {
                    app.push_system("No MCP servers configured.".into());
                } else {
                    app.push_system(format!("MCP servers ({}):", servers.len()));
                    for s in servers {
                        let id = s["id"].as_str().unwrap_or("?");
                        let status = s["status"].as_str().unwrap_or("unknown");
                        app.push_system(format!("  {id}: {status}"));
                    }
                }
            }
            app.status = "Ready".into();
        }
        "sessions.claim" => {}
        "sessions.messages" => {
            if let Some(messages) = msg["data"]["messages"].as_array() {
                for m in messages {
                    let role = m["role"].as_str().unwrap_or("unknown").to_string();
                    let content = match &m["content"] {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => serde_json::to_string_pretty(other).unwrap_or_default(),
                    };
                    let ts = m["createdAt"]
                        .as_str()
                        .or_else(|| m["created_at"].as_str())
                        .map(|s| {
                            s.split('T')
                                .next_back()
                                .unwrap_or(s)
                                .split(' ')
                                .next_back()
                                .unwrap_or(s)
                                .split('.')
                                .next()
                                .unwrap_or(s)
                                .to_string()
                        })
                        .unwrap_or_default();
                    app.messages.push(ChatMsg {
                        role,
                        content,
                        timestamp: ts,
                    });
                }
                app.scroll_offset = 0;
            }
        }
        "ask_question" => {
            let request_id = msg["data"]["request_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let question = msg["data"]["question"]
                .as_str()
                .unwrap_or("Agent is asking a question")
                .to_string();
            let options: Vec<(String, String)> = msg["data"]["options"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .enumerate()
                        .map(|(i, opt)| {
                            let id = opt["id"]
                                .as_str()
                                .unwrap_or(&format!("{}", i + 1))
                                .to_string();
                            let label = opt["label"]
                                .as_str()
                                .or_else(|| opt.as_str())
                                .unwrap_or(&id)
                                .to_string();
                            (id, label)
                        })
                        .collect()
                })
                .unwrap_or_default();
            app.show_popup = Some(PopupKind::AskQuestion {
                request_id,
                question,
                options,
            });
            app.status = "Agent is waiting for your answer...".into();
        }
        "set_mode" => {
            if let Some(true) = msg["data"]["ok"].as_bool() {
                let to = msg["data"]["to"].as_str().unwrap_or("agent");
                app.execution_mode = to.to_string();
                let label = if to == "plan" {
                    "Plan (read-only)"
                } else {
                    "Agent (full access)"
                };
                app.push_system(format!("Switched to {label} mode."));
                app.status = format!("Mode: {to}");
            }
        }
        "mode_change" => {
            let to = msg["data"]["to"].as_str().unwrap_or("agent");
            if to != app.execution_mode {
                app.execution_mode = to.to_string();
                let plan_info = if to == "plan" {
                    if let Some(ref path) = app.plan_file_path {
                        format!(" · plan: {path}")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                app.status = format!("Mode: {to}{plan_info}");
            }
        }
        "plan_file_update" => {
            let path = msg["data"]["path"].as_str().map(|s| s.to_string());
            let exists = msg["data"]["exists"].as_bool().unwrap_or(false);
            app.plan_file_path = path.clone();
            app.plan_file_exists = exists;
            if let Some(ref p) = path {
                let short = dirs::home_dir()
                    .and_then(|h| h.to_str().map(|s| p.replace(&format!("{s}/"), "~/")))
                    .unwrap_or_else(|| p.clone());
                let state = if exists { "已创建" } else { "待创建" };
                app.push_system(format!("Plan file ({state}): {short}"));
            }
        }
        "cancel" => {
            let cancelled = msg["data"]["cancelled"].as_bool().unwrap_or(false);
            if cancelled {
                app.streaming = false;
                app.status = "Cancelled (server confirmed)".into();
            }
        }
        "turn_aborted" => {
            app.streaming = false;
            let reason = msg["data"]["reason"].as_str().unwrap_or("interrupted");
            app.push_system(format!("Turn aborted: {reason}"));
            app.status = format!("Aborted: {reason}");
        }
        "stream_error" => {
            let err = msg["data"]["message"].as_str().unwrap_or("stream error");
            let retry = msg["data"]["retry_attempt"].as_u64().unwrap_or(0);
            if retry > 0 {
                app.push_system(format!("⚠ Stream error (retry {retry}): {err}"));
            } else {
                app.push_system(format!("⚠ Stream error: {err}"));
            }
        }
        "warning" => {
            let warn_msg = msg["data"]["message"].as_str().unwrap_or("warning");
            app.push_system(format!("⚠ {warn_msg}"));
        }
        "approval_required" => {
            let request_id = format!(
                "approval:{}",
                msg["data"]["approval_id"].as_str().unwrap_or("")
            );
            let action_desc = msg["data"]["action"]
                .as_object()
                .and_then(|a| a.get("type").and_then(|t| t.as_str()))
                .unwrap_or("action");
            let question = format!("Approve {action_desc}?");
            let options = vec![
                ("approved".to_string(), "Allow".to_string()),
                ("denied".to_string(), "Deny".to_string()),
                ("approved_for_session".to_string(), "Allow for session".to_string()),
            ];
            app.show_popup = Some(PopupKind::AskQuestion {
                request_id,
                question,
                options,
            });
            app.status = "Waiting for approval...".into();
        }
        "approval_resolved" => {
            app.show_popup = None;
            app.status = "Ready".into();
        }
        "context_warning" => {
            let msg_text = msg["data"]["message"].as_str().unwrap_or("");
            app.push_system(format!("⚠ {msg_text}"));
        }
        "compact_boundary" => {
            app.push_system("⚠ Context compacted to fit token limit.".into());
        }
        "brief_message" => {}
        "suggestions" => {
            if let Some(items) = msg["data"]["items"].as_array() {
                let suggestions: Vec<&str> =
                    items.iter().filter_map(|v| v.as_str()).take(3).collect();
                if !suggestions.is_empty() {
                    app.push_system(format!("Suggestions: {}", suggestions.join(" | ")));
                }
            }
        }
        "sub_agent_start" => {
            let label = msg["data"]["task"].as_str().unwrap_or("sub-agent");
            app.spinner.verb = format!("sub-agent: {label}");
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" {
                    last.content
                        .push_str(&format!("\n● Task: {label}\n"));
                }
            }
        }
        "sub_agent_delta" => {
            if let Some(content) = msg["data"]["content"].as_str() {
                if let Some(last) = app.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(content);
                    }
                }
            }
        }
        "sub_agent_tool_executing" => {
            let tool = msg["data"]["tool_name"].as_str().unwrap_or("unknown");
            app.spinner.verb = format!("sub-agent tool: {tool}");
        }
        "sub_agent_tool_result" => {
            app.spinner.set_thinking();
        }
        "sub_agent_complete" => {
            app.spinner.set_thinking();
        }
        "heartbeat" | "pong" => {}
        _ => {}
    }
}

fn tool_display_name(tool: &str) -> String {
    match tool {
        "file_read" | "read_file" | "Read" => "Read".into(),
        "file_write" | "write_file" | "Write" => "Write".into(),
        "edit_file" | "StrReplace" => "Edit".into(),
        "multi_edit" => "Multi-edit".into(),
        "file_search" | "search_in_files" | "Grep" => "Search".into(),
        "glob" | "Glob" | "list_directory" => "List".into(),
        "shell_exec" | "shell" | "Shell" => "Bash".into(),
        "web_search" | "WebSearch" => "WebSearch".into(),
        "web_fetch" | "http_fetch" | "WebFetch" => "WebFetch".into(),
        "todo_write" | "TodoWrite" => "TodoWrite".into(),
        "memory_search" | "memory_store" => "Memory".into(),
        "enter_plan_mode" | "exit_plan_mode" | "SwitchMode" => "SwitchMode".into(),
        "Task" => "Task".into(),
        "Delete" => "Delete".into(),
        _ => tool.to_string(),
    }
}
