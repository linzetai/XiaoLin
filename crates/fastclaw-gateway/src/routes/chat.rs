use std::time::Duration;

use axum::{body::Body, extract::{Query, State}, http::header, response::IntoResponse, Json};
use serde_json::json;

use fastclaw_core::types::ChatRequest;

use crate::chat_pipeline::{
    after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
};
use crate::extract::AppJson;
use crate::state::AppState;

use super::common::{record_chat_budget_actual, record_chat_budget_stream_estimate};
use super::error::AppError;

pub(super) async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents: Vec<_> = {
        let router = state.router.read().await;
        router
            .list_agents()
            .into_iter()
            .map(|a| {
                json!({
                    "agentId": a.agent_id,
                    "name": a.name,
                    "description": a.description,
                    "model": a.model.model,
                })
            })
            .collect()
    };
    Json(json!({ "agents": agents }))
}

pub(super) async fn list_tools(State(state): State<AppState>) -> impl IntoResponse {
    let tools = state.tool_registry.definitions();
    Json(json!({ "tools": tools }))
}

#[derive(serde::Deserialize)]
pub(super) struct SkillsQuery {
    #[serde(default, alias = "agentId")]
    agent_id: Option<String>,
}

pub(super) async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> impl IntoResponse {
    let agent_id = query.agent_id.unwrap_or_else(|| "main".to_string());
    let registry = state.skill_registry_for(&agent_id);
    let skills: Vec<_> = registry
        .list()
        .into_iter()
        .filter(|s| s.frontmatter.enabled.unwrap_or(true))
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
            })
        })
        .collect();
    Json(json!({
        "agentId": agent_id,
        "skills": skills,
        "count": skills.len(),
    }))
}

pub(super) async fn chat_completions(
    State(state): State<AppState>,
    AppJson(request): AppJson<ChatRequest>,
) -> Result<axum::response::Response, AppError> {
    if request.stream {
        handle_stream(state, request).await
    } else {
        handle_non_stream(state, request).await
    }
}

async fn handle_non_stream(
    state: AppState,
    request: ChatRequest,
) -> Result<axum::response::Response, AppError> {
    let request_start = std::time::Instant::now();
    let setup = setup_chat(
        &state,
        &request,
        SetupChatOptions {
            chat_stream: false,
            propagate_context_ingest_errors: true,
            set_resolved_session_on_request: false,
            ..Default::default()
        },
    )
    .await?;

    let result = match state
        .runtime
        .execute(
            &setup.agent_config,
            &setup.enriched_request,
            &state.tool_registry,
            setup.llm_override.clone(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if setup.reserved_cost > 0.0 {
                let _ = state
                    .budget_tracker
                    .release_reservation(setup.reserved_cost);
            }
            return Err(e.into());
        }
    };

    let charged_model = result.response.model.clone();
    record_chat_budget_actual(
        &state,
        charged_model.as_str(),
        result.response.usage.as_ref(),
    );

    for msg in &setup.user_messages {
        state.session_store.append_message(&setup.session_id, msg).await?;
    }
    if let Some(choice) = result.response.choices.first() {
        after_chat(&state, &setup, &choice.message, false).await?;
    }

    state
        .context_engine
        .after_turn(
            &setup.enriched_request.messages,
            &setup.agent_id,
            &setup.session_id,
        )
        .await?;

    if let Some(assistant_text) = result
        .response
        .choices
        .first()
        .and_then(|c| c.message.text_content())
    {
        maybe_spawn_smart_title_background(&state, &setup, assistant_text.as_str());
    }

    fastclaw_observe::record_chat_latency(&setup.agent_id, request_start);

    let mut resp_json = serde_json::to_value(&result.response)
        .map_err(|e| anyhow::anyhow!("serialization error: {e}"))?;
    if let Some(obj) = resp_json.as_object_mut() {
        let mut meta = json!({
            "sessionId": &setup.session_id,
            "toolCallsMade": result.tool_calls_made,
            "iterations": result.iterations,
            "memoryInjected": state.config.memory.enabled,
            "resolvedAgent": &setup.agent_id,
            "resolveReason": setup.resolve_reason,
        });
        if let Some(intent) = setup.prompt_intent.as_deref() {
            meta["intent"] = json!(intent);
        }
        if let Some(profile) = setup.prompt_profile.as_deref() {
            meta["promptProfile"] = json!(profile);
        }
        if let Some(reason) = setup.prompt_route_reason {
            meta["promptRouteReason"] = json!(reason);
        }
        if let Some(t) = setup.slash_intent_type.as_deref() {
            meta["slashIntentType"] = json!(t);
        }
        if let Some(v) = setup.slash_intent_value.as_deref() {
            meta["slashIntentValue"] = json!(v);
        }
        if let Some(exact) = setup.slash_exact_match {
            meta["slashExactMatch"] = json!(exact);
        }
        if let Some(skill_loaded) = setup.slash_skill_loaded {
            meta["slashSkillLoaded"] = json!(skill_loaded);
        }
        if setup.budget_degraded {
            meta["budgetDegraded"] = json!(true);
        }
        obj.insert("_meta".to_string(), meta);
    }

    Ok(Json(resp_json).into_response())
}

/// SSE queue capacity between the LLM streaming task and the HTTP SSE encoder.
/// Token deltas may be dropped under backpressure (see `send_stream_event` in `fastclaw-agent`).
const SSE_EVENT_CHANNEL_CAP: usize = 1024;
const SSE_SEND_TIMEOUT: Duration = Duration::from_millis(150);

async fn handle_stream(
    state: AppState,
    request: ChatRequest,
) -> Result<axum::response::Response, AppError> {
    use fastclaw_core::types::StreamEvent;

    let setup = setup_chat(
        &state,
        &request,
        SetupChatOptions {
            chat_stream: true,
            propagate_context_ingest_errors: true,
            set_resolved_session_on_request: true,
            ..Default::default()
        },
    )
    .await?;

    let session_id = setup.session_id.clone();
    let resolve_reason = setup.resolve_reason;
    let needs_title_stream = setup.needs_title;
    let model_for_budget = setup.model_for_budget.clone();
    let input_estimate = setup.input_estimate;
    let budget_degraded = setup.budget_degraded;
    let reserved = setup.reserved_cost;
    let enriched = setup.enriched_request.clone();
    let agent_config = setup.agent_config.clone();

    for msg in &setup.user_messages {
        if let Err(e) = state.session_store.append_message(&session_id, msg).await {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "stream: failed to persist user message to session"
            );
        }
    }

    let state_budget = state.clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(SSE_EVENT_CHANNEL_CAP);

    let state_for_task = state.clone();
    let config_for_task = agent_config.clone();
    let llm_for_task = setup.llm_override.clone();
    let agent_id_for_hook = setup.agent_id.clone();
    let session_id_for_hook = session_id.clone();

    tokio::spawn(async move {
        let result = state_for_task
            .runtime
            .execute_stream(
                &config_for_task,
                &enriched,
                &state_for_task.tool_registry,
                tx.clone(),
                llm_for_task,
            )
            .await;

        // Run after_turn hooks (matching non-stream path behavior)
        let _ = state_for_task
            .context_engine
            .after_turn(&enriched.messages, &agent_id_for_hook, &session_id_for_hook)
            .await;

        if let Err(e) = result {
            let _ =
                tokio::time::timeout(SSE_SEND_TIMEOUT, tx.send(StreamEvent::Error(e.to_string())))
                    .await;
        }
    });

    let session_id_header = session_id.clone();
    let state_for_persist = state.clone();
    let setup_for_persist = setup.clone();

    let sse_stream = async_stream::stream! {
        let mut streamed_chars: usize = 0;
        let mut assistant_content = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::Delta(delta) => {
                    let has_content = delta.choices.iter().any(|c| {
                        c.delta.role.is_some()
                            || c.delta
                                .content
                                .as_deref()
                                .map(|s| !s.is_empty())
                                .unwrap_or(false)
                            || c.delta.tool_calls.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
                    });
                    if !has_content {
                        continue;
                    }
                    for choice in &delta.choices {
                        if let Some(ref c) = choice.delta.content {
                            assistant_content.push_str(c);
                            streamed_chars = streamed_chars.saturating_add(c.len());
                        }
                    }
                    let json_str = serde_json::to_string(&delta).unwrap_or_default();
                    yield Ok::<_, std::io::Error>(format!("data: {json_str}\n\n"));
                }
                StreamEvent::ToolExecuting { tool_name, call_id, args } => {
                    let ev = json!({
                        "type": "tool_executing",
                        "tool": tool_name,
                        "call_id": call_id,
                        "args": args,
                    });
                    yield Ok(format!("event: tool\ndata: {ev}\n\n"));
                }
                StreamEvent::ToolResult { tool_name, call_id, output, success } => {
                    let ev = json!({
                        "type": "tool_result",
                        "tool": tool_name,
                        "call_id": call_id,
                        "output": output,
                        "success": success,
                    });
                    yield Ok(format!("event: tool\ndata: {ev}\n\n"));
                }
                StreamEvent::Done { session_id, tool_calls_made, iterations, final_tool_calls, usage, elapsed_ms, .. } => {
                    record_chat_budget_stream_estimate(
                        &state_budget,
                        model_for_budget.as_str(),
                        input_estimate,
                        streamed_chars,
                    );
                    // Persist assistant message (matching non-stream + WS path behavior)
                    if !assistant_content.is_empty() || final_tool_calls.is_some() {
                        let assistant_msg = fastclaw_core::types::ChatMessage {
                            role: fastclaw_core::types::Role::Assistant,
                            content: if assistant_content.is_empty() {
                                None
                            } else {
                                Some(serde_json::Value::String(assistant_content.clone()))
                            },
                            name: None,
                            tool_calls: final_tool_calls,
                            tool_call_id: None,
                        };
                        let _ = after_chat(
                            &state_for_persist,
                            &setup_for_persist,
                            &assistant_msg,
                            needs_title_stream,
                        )
                        .await;
                    }
                    let mut done_ev = json!({
                        "type": "done",
                        "sessionId": session_id,
                        "toolCallsMade": tool_calls_made,
                        "iterations": iterations,
                        "resolvedAgent": &setup_for_persist.agent_id,
                        "resolveReason": resolve_reason,
                        "elapsedMs": elapsed_ms,
                    });
                    if let Some(ref u) = usage {
                        done_ev["usage"] = json!({
                            "promptTokens": u.prompt_tokens,
                            "completionTokens": u.completion_tokens,
                            "totalTokens": u.total_tokens,
                        });
                    }
                    if budget_degraded {
                        done_ev["budgetDegraded"] = json!(true);
                    }
                    if let Some((est_tokens, ctx_window)) = setup_for_persist.context_tokens_estimate {
                        let actual_prompt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                        done_ev["contextTokens"] = json!(if actual_prompt > 0 { actual_prompt } else { est_tokens });
                        if ctx_window > 0 {
                            done_ev["contextWindow"] = json!(ctx_window);
                        }
                    }
                    let ev = done_ev;
                    yield Ok(format!("event: done\ndata: {ev}\n\n"));
                    yield Ok("data: [DONE]\n\n".to_string());
                    break;
                }
                StreamEvent::AskQuestion { request_id, question, options, timeout_secs, allow_multiple } => {
                    let ev = json!({
                        "type": "ask_question",
                        "requestId": request_id,
                        "question": question,
                        "options": options,
                        "timeoutSecs": timeout_secs,
                        "allowMultiple": allow_multiple,
                    });
                    yield Ok(format!("event: ask_question\ndata: {ev}\n\n"));
                }
                StreamEvent::Error(e) => {
                    if reserved > 0.0 {
                        let _ = state_budget.budget_tracker.release_reservation(reserved);
                    }
                    if !assistant_content.is_empty() {
                        let assistant_msg = fastclaw_core::types::ChatMessage {
                            role: fastclaw_core::types::Role::Assistant,
                            content: Some(serde_json::Value::String(assistant_content.clone())),
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                        };
                        let _ = after_chat(
                            &state_for_persist,
                            &setup_for_persist,
                            &assistant_msg,
                            false,
                        )
                        .await;
                    }
                    let ev = json!({"error": e});
                    yield Ok(format!("data: {ev}\n\n"));
                    yield Ok("data: [DONE]\n\n".to_string());
                    break;
                }
            }
        }
    };

    let body = Body::from_stream(sse_stream);
    let response = axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .header("X-Session-Id", session_id_header)
        .body(body)
        .map_err(|e| anyhow::anyhow!("failed to build SSE response: {e}"))?;

    Ok(response)
}
