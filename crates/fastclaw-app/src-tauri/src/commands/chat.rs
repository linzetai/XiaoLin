use crate::AppData;
use serde_json::json;
use tauri::Emitter;
use super::helpers::get_state;

// ─── Chat streaming via Tauri Channel ───

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_stream(
    state: tauri::State<'_, AppData>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<serde_json::Value>,
    messages: Vec<serde_json::Value>,
    agent_id: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    work_dir: Option<String>,
    request_id: Option<String>,
) -> Result<(), String> {
    use fastclaw_core::types::{ChatMessage, ChatRequest, Role, StreamEvent};
    use fastclaw_gateway::chat_pipeline::{
        after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
    };
    use fastclaw_gateway::routes::record_chat_budget_stream_estimate;

    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?.clone();
    drop(gw);
    let stream_request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut cancels = state.stream_cancels.lock().await;
        if let Some(prev) = cancels.insert(stream_request_id.clone(), cancel_tx) {
            let _ = prev.send(());
        }
    }

    let chat_messages: Vec<ChatMessage> = messages
        .into_iter()
        .map(|m| {
            let role_str = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let role = match role_str {
                "system" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let content = m.get("content").cloned();
            let name = m.get("name").and_then(|v| v.as_str()).map(String::from);
            let tool_call_id = m
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            ChatMessage {
                role,
                content,
                name,
                tool_calls: None,
                tool_call_id,
            }
        })
        .collect();

    let request = ChatRequest {
        messages: chat_messages,
        model,
        stream: true,
        max_tokens,
        temperature,
        agent_id: agent_id.map(Into::into),
        session_id,
        tools: None,
        slash_intent: None,
        work_dir,
    };

    let setup = setup_chat(
        &app,
        &request,
        SetupChatOptions {
            chat_stream: true,
            propagate_context_ingest_errors: false,
            set_resolved_session_on_request: true,
            record_chat_observe: false,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    let session_id = setup.session_id.clone();
    let agent_id = setup.agent_id.clone();
    let needs_title = setup.needs_title;
    let model_for_budget = setup.model_for_budget.clone();
    let input_estimate = setup.input_estimate;
    let budget_degraded = setup.budget_degraded;
    let mut reserved = setup.reserved_cost;
    let agent_config = setup.agent_config.clone();
    let enriched = setup.enriched_request.clone();
    let after_turn_messages = setup.enriched_request.messages.clone();
    let context_tokens_est = setup.context_tokens_estimate;

    for msg in &setup.user_messages {
        if let Err(e) = app.store.session_store.append_message(&session_id, msg).await {
            tracing::error!(session_id = %session_id, error = %e, "failed to persist user message");
        }
    }

    let start_model = enriched
        .model
        .as_deref()
        .unwrap_or(agent_config.model.model.as_str());

    let mut start_payload = json!({
        "model": start_model,
        "sessionId": &session_id,
        "resolvedAgent": &agent_id,
    });
    if budget_degraded {
        start_payload["budgetDegraded"] = json!(true);
    }
    let _ = channel.send(json!({"type": "chat.start", "data": start_payload}));

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

    let stream_context_key = uuid::Uuid::new_v4().to_string();
    app.strm
        .stream_event_tx
        .insert(stream_context_key.clone(), tx.clone());

    let subagent_prompt = {
        let policy = &agent_config.behavior.subagent;
        let available = app.strm.subagent_manager.agent_descriptions();
        let ctx = fastclaw_agent::SubAgentPromptContext {
            policy,
            available_agents: &available,
            current_depth: 0,
        };
        fastclaw_agent::build_subagent_prompt_block(&ctx)
    };

    let runtime = app.rt.runtime.clone();
    let tool_reg = app.rt.tool_registry.clone();
    let llm_override = setup.llm_override.clone();
    let stream_event_tx_ref = app.strm.stream_event_tx.clone();
    let stream_context_key_for_task = stream_context_key.clone();
    let stream_request_id_for_task = stream_request_id.clone();
    let stream_cancel_map_for_task = state.stream_cancels.clone();
    let confirm_pending_for_task = app.strm.ask_question_pending.clone();

    let task = tokio::spawn(async move {
        let result = tokio::select! {
            result = fastclaw_agent::builtin_tools::with_stream_context(
                stream_context_key_for_task.clone(),
                runtime.execute_stream_with_confirm(&agent_config, &enriched, &tool_reg, tx, llm_override, confirm_pending_for_task, subagent_prompt),
            ) => result,
            _ = &mut cancel_rx => Err(anyhow::anyhow!("cancelled")),
        };
        stream_event_tx_ref.remove(&stream_context_key_for_task);
        stream_cancel_map_for_task
            .lock()
            .await
            .remove(&stream_request_id_for_task);
        result
    });

    let mut assistant_content = String::new();
    let mut pending_question_ids: Vec<String> = Vec::new();
    let mut last_checkpoint = std::time::Instant::now();
    let checkpoint_interval = std::time::Duration::from_secs(5);
    #[allow(clippy::type_complexity)]
    let mut accumulated_tool_calls: Vec<(String, String, Option<String>, Option<String>, bool)> = Vec::new();
    while let Some(event) = rx.recv().await {
        match &event {
            StreamEvent::Delta(delta) => {
                if let Some(text) = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref())
                {
                    assistant_content.push_str(text);
                }
                if !assistant_content.is_empty() && last_checkpoint.elapsed() >= checkpoint_interval {
                    last_checkpoint = std::time::Instant::now();
                    let _ = app.store.session_store.save_partial_assistant(&session_id, &assistant_content).await;
                }
                let _ = channel.send(json!({
                    "type": "chat.delta",
                    "data": {"content": delta.choices.first().and_then(|c| c.delta.content.as_deref()), "model": delta.model}
                }));
            }
            StreamEvent::ToolExecuting {
                tool_name,
                call_id,
                args,
            } => {
                accumulated_tool_calls.push((call_id.clone(), tool_name.clone(), args.clone(), None, true));
                let _ = channel.send(json!({
                    "type": "chat.tool.start",
                    "data": {"tool": tool_name, "callId": call_id, "args": args}
                }));
            }
            StreamEvent::ToolResult {
                tool_name,
                call_id,
                output,
                display_output,
                success,
                metadata,
            } => {
                let ui_out = display_output.as_ref().unwrap_or(output);
                if let Some(tc) = accumulated_tool_calls.iter_mut().find(|(cid, _, _, _, _)| cid == call_id) {
                    tc.3 = Some(ui_out.clone());
                    tc.4 = *success;
                }
                let mut data = json!({"tool": tool_name, "callId": call_id, "output": ui_out, "success": success});
                if let Some(meta) = metadata {
                    data["metadata"] = meta.clone();
                }
                let _ = channel.send(json!({
                    "type": "chat.tool.done",
                    "data": data
                }));
            }
            StreamEvent::Done {
                session_id: sid,
                tool_calls_made,
                iterations,
                usage,
                elapsed_ms,
                ..
            } => {
                record_chat_budget_stream_estimate(
                    &app,
                    model_for_budget.as_str(),
                    input_estimate,
                    assistant_content.len(),
                );

                if !assistant_content.is_empty() {
                    let _ = app.store.session_store.remove_partial_assistant(&session_id).await;
                    let saved_tool_calls: Option<Vec<fastclaw_core::types::ToolCall>> = if accumulated_tool_calls.is_empty() {
                        None
                    } else {
                        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, output, success)| {
                            fastclaw_core::types::ToolCall {
                                id: cid.clone(),
                                call_type: "function".to_string(),
                                function: fastclaw_core::types::FunctionCall {
                                    name: tname.clone(),
                                    arguments: args.clone().unwrap_or_default(),
                                },
                                output: output.clone(),
                                success: Some(*success),
                                duration_ms: None,
                            }
                        }).collect())
                    };
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: saved_tool_calls,
                        tool_call_id: None,
                    };
                    let _ = after_chat(&app, &setup, &assistant_msg, false).await;
                }

                let mut complete_data = json!({
                    "sessionId": sid,
                    "toolCallsMade": tool_calls_made,
                    "iterations": iterations,
                    "elapsedMs": elapsed_ms,
                });
                if let Some(ref u) = usage {
                    complete_data["usage"] = json!({
                        "promptTokens": u.prompt_tokens,
                        "completionTokens": u.completion_tokens,
                        "totalTokens": u.total_tokens,
                    });
                }
                if let Some((est_tokens, ctx_window)) = context_tokens_est {
                    let actual_prompt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                    complete_data["contextTokens"] = json!(if actual_prompt > 0 { actual_prompt } else { est_tokens });
                    if ctx_window > 0 {
                        complete_data["contextWindow"] = json!(ctx_window);
                    }
                }
                let _ = channel.send(json!({
                    "type": "chat.complete",
                    "data": complete_data,
                }));

                // Persist usage metrics (session totals + per-message)
                if let Some(ref sid_str) = sid {
                    let pt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                    let ct = usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
                    let tt = usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                    let _ = app.store.session_store.accumulate_usage(sid_str, pt, ct, *elapsed_ms).await;
                    let _ = app.store.session_store.stamp_last_assistant_usage(sid_str, pt, ct, tt, *elapsed_ms).await;
                }

                // Emit Tauri event for session change
                let _ = app_handle.emit(
                    "sessions-changed",
                    json!({"sessionId": sid}),
                );
            }
            StreamEvent::AskQuestion {
                request_id,
                question,
                options,
                timeout_secs,
                allow_multiple,
            } => {
                pending_question_ids.push(request_id.clone());
                let _ = channel.send(json!({
                    "type": "chat.ask_question",
                    "data": {
                        "requestId": request_id,
                        "question": question,
                        "options": options,
                        "timeoutSecs": timeout_secs,
                        "allowMultiple": allow_multiple,
                    }
                }));
            }
            StreamEvent::Error(e) => {
                if reserved > 0.0 {
                    let _ = app.obs.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let _ = app.store.session_store.remove_partial_assistant(&session_id).await;
                    let err_tc = if accumulated_tool_calls.is_empty() { None } else {
                        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, o, s)| {
                            fastclaw_core::types::ToolCall { id: cid.clone(), call_type: "function".into(), function: fastclaw_core::types::FunctionCall { name: tname.clone(), arguments: args.clone().unwrap_or_default() }, output: o.clone(), success: Some(*s), duration_ms: None }
                        }).collect())
                    };
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: err_tc,
                        tool_call_id: None,
                    };
                    let _ = after_chat(&app, &setup, &assistant_msg, false).await;
                }
                let _ = channel.send(json!({
                    "type": "chat.error",
                    "error": {"message": e.to_string()}
                }));
            }

            // ── Sub-agent streaming events (Tauri IPC) ──────────────
            StreamEvent::SubAgentStart { run_id, agent_id, subagent_type, task, depth } => {
                let _ = channel.send(json!({
                    "type": "chat.subagent.start",
                    "data": {"runId": run_id, "agentId": agent_id, "subagentType": subagent_type, "task": task, "depth": depth}
                }));
            }
            StreamEvent::SubAgentDelta { run_id, content } => {
                let _ = channel.send(json!({
                    "type": "chat.subagent.delta",
                    "data": {"runId": run_id, "content": content}
                }));
            }
            StreamEvent::SubAgentToolExecuting { run_id, tool_name, call_id, args } => {
                let _ = channel.send(json!({
                    "type": "chat.subagent.tool.start",
                    "data": {"runId": run_id, "tool": tool_name, "callId": call_id, "args": args}
                }));
            }
            StreamEvent::SubAgentToolResult { run_id, tool_name, call_id, output, success } => {
                let _ = channel.send(json!({
                    "type": "chat.subagent.tool.done",
                    "data": {"runId": run_id, "tool": tool_name, "callId": call_id, "output": output, "success": success}
                }));
            }
            StreamEvent::ToolProgress { tool_name, call_id, message, progress, partial_output } => {
                let _ = channel.send(json!({
                    "type": "chat.tool.progress",
                    "data": {"tool": tool_name, "callId": call_id, "message": message, "progress": progress, "partialOutput": partial_output}
                }));
            }
            StreamEvent::ContextLimitWarning { used_tokens, limit_tokens, message } => {
                let _ = channel.send(json!({
                    "type": "chat.context.warning",
                    "data": {"usedTokens": used_tokens, "limitTokens": limit_tokens, "message": message}
                }));
            }
            StreamEvent::ContextUsageUpdate { used_tokens, limit_tokens, compressed, tokens_saved } => {
                let _ = channel.send(json!({
                    "type": "chat.context.usage",
                    "data": {"usedTokens": used_tokens, "limitTokens": limit_tokens, "compressed": compressed, "tokensSaved": tokens_saved}
                }));
            }
            StreamEvent::SubAgentComplete { run_id, status, result, tool_calls_made, iterations, usage, elapsed_ms } => {
                let mut data = json!({
                    "runId": run_id, "status": status, "result": result,
                    "toolCallsMade": tool_calls_made, "iterations": iterations, "elapsedMs": elapsed_ms,
                });
                if let Some(ref u) = usage {
                    data["usage"] = json!({"promptTokens": u.prompt_tokens, "completionTokens": u.completion_tokens, "totalTokens": u.total_tokens});
                }
                let _ = channel.send(json!({
                    "type": "chat.subagent.complete",
                    "data": data,
                }));
            }
            StreamEvent::BriefMessage { content, attachments, mode } => {
                let _ = channel.send(json!({
                    "type": "chat.brief",
                    "data": {"content": content, "attachments": attachments, "mode": mode}
                }));
            }
        }
    }

    if !assistant_content.is_empty() {
        let _ = app
            .store
            .context_engine
            .after_turn(&after_turn_messages, &agent_id, &session_id)
            .await;
    }

    if needs_title && !assistant_content.is_empty() {
        maybe_spawn_smart_title_background(&app, &setup, &assistant_content);
    }

    let build_tc_for_persist = || -> Option<Vec<fastclaw_core::types::ToolCall>> {
        if accumulated_tool_calls.is_empty() { return None; }
        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, o, s)| {
            fastclaw_core::types::ToolCall { id: cid.clone(), call_type: "function".into(), function: fastclaw_core::types::FunctionCall { name: tname.clone(), arguments: args.clone().unwrap_or_default() }, output: o.clone(), success: Some(*s), duration_ms: None }
        }).collect())
    };

    match task.await {
        Ok(Err(e)) => {
            if reserved > 0.0 {
                let _ = app.obs.budget_tracker.release_reservation(reserved);
            }
            if !assistant_content.is_empty() {
                let _ = app.store.session_store.remove_partial_assistant(&session_id).await;
                let assistant_msg = ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(assistant_content.clone())),
                    name: None,
                    tool_calls: build_tc_for_persist(),
                    tool_call_id: None,
                };
                let _ = after_chat(&app, &setup, &assistant_msg, false).await;
            }
            let _ = channel.send(json!({
                "type": "chat.error",
                "error": {"message": format!("{e}")}
            }));
        }
        Err(e) => {
            if reserved > 0.0 {
                let _ = app.obs.budget_tracker.release_reservation(reserved);
            }
            if !assistant_content.is_empty() {
                let _ = app.store.session_store.remove_partial_assistant(&session_id).await;
                let assistant_msg = ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(std::mem::take(&mut assistant_content))),
                    name: None,
                    tool_calls: build_tc_for_persist(),
                    tool_call_id: None,
                };
                let _ = after_chat(&app, &setup, &assistant_msg, false).await;
            }
            let _ = channel.send(json!({
                "type": "chat.error",
                "error": {"message": format!("task panic: {e}")}
            }));
        }
        _ => {}
    }

    if !pending_question_ids.is_empty() {
        let pending = &app.strm.ask_question_pending;
        for request_id in pending_question_ids {
            pending.remove(&request_id);
        }
    }
    state.stream_cancels.lock().await.remove(&stream_request_id);

    Ok(())
}

#[tauri::command]
pub async fn cancel_chat_stream(
    state: tauri::State<'_, AppData>,
    request_id: String,
) -> Result<serde_json::Value, String> {
    let sender = state.stream_cancels.lock().await.remove(&request_id);
    let cancelled = if let Some(tx) = sender {
        tx.send(()).is_ok()
    } else {
        false
    };
    Ok(json!({ "ok": true, "cancelled": cancelled }))
}

// ─── Ask Question answer submission ───

#[tauri::command]
pub async fn submit_tool_answer(
    state: tauri::State<'_, AppData>,
    request_id: String,
    answer: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let sender = app
        .strm
        .ask_question_pending
        .remove(&request_id)
        .map(|(_k, v)| v);
    if let Some(tx) = sender {
        let _ = tx.send(answer);
        Ok(json!({ "ok": true }))
    } else {
        Ok(json!({ "ok": false, "reason": "request not found or already answered" }))
    }
}
