use std::collections::HashSet;

use axum::{
    body::Body,
    extract::{Query, State},
    http::header,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use xiaolin_agent::{build_subagent_prompt_block, SubAgentPromptContext};
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::skill::SkillOrigin;
use xiaolin_core::types::ChatRequest;

use crate::chat_pipeline::{
    after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
};
use crate::extract::AppJson;
use crate::state::AppState;

use super::common::{record_chat_budget_actual, record_chat_budget_stream_estimate};
use super::error::AppError;

fn build_subagent_prompt_for_agent(state: &AppState, config: &AgentConfig) -> Option<String> {
    let policy = &config.behavior.subagent;
    let available = state.strm.subagent_manager.agent_descriptions();
    let ctx = SubAgentPromptContext {
        policy,
        available_agents: &available,
        current_depth: 0,
    };
    build_subagent_prompt_block(&ctx)
}

pub(super) async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents: Vec<_> = {
        let router = state.rt.router.read().await;
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
    let tools = state.rt.tool_registry.definitions();
    Json(json!({ "tools": tools }))
}

#[derive(serde::Deserialize)]
pub(super) struct SkillsQuery {
    #[serde(default, alias = "agentId")]
    agent_id: Option<String>,
}

fn skill_origin_str(origin: SkillOrigin) -> &'static str {
    match origin {
        SkillOrigin::XiaoLin => "xiaolin",
        SkillOrigin::Cursor => "cursor",
        SkillOrigin::Codex => "codex",
        SkillOrigin::SharedAgents => "shared_agents",
        SkillOrigin::Extension => "extension",
        SkillOrigin::Mcp => "mcp",
    }
}

pub(super) async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> impl IntoResponse {
    let _agent_id = query.agent_id.unwrap_or_else(|| "main".to_string());

    let deny_list: Vec<String> = {
        let live = state.cfg.config_live.load();
        live.get("skills")
            .and_then(|s| s.get("deny"))
            .and_then(|d| serde_json::from_value::<Vec<String>>(d.clone()).ok())
            .unwrap_or_default()
    };
    let deny_set: HashSet<&str> = deny_list.iter().map(String::as_str).collect();

    let usage_counts = match state.store.skill_usage_store.usage_counts(30).await {
        Ok(counts) => counts,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load skill usage counts for HTTP skills list");
            std::collections::HashMap::new()
        }
    };

    let registry = (*state.rt.unfiltered_skill_registry.load()).clone();
    let skills: Vec<_> = registry
        .list()
        .into_iter()
        .map(|s| {
            let enabled =
                s.frontmatter.enabled.unwrap_or(true) && !deny_set.contains(s.id.as_str());
            let origin = s
                .source
                .as_ref()
                .map(|src| skill_origin_str(src.origin))
                .unwrap_or("xiaolin");
            let usage_count = usage_counts.get(&s.id).copied().unwrap_or(0);
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
                "source": origin,
                "layer": format!("{:?}", s.layer),
                "enabled": enabled,
                "paths": s.frontmatter.paths,
                "conditional": s.is_conditional(),
                "usage_count": usage_count,
            })
        })
        .collect();
    Json(json!({
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

    let subagent_prompt = build_subagent_prompt_for_agent(&state, &setup.agent_config);
    let result = match state
        .rt
        .runtime
        .execute_with_subagent_prompt_and_runtime_quality_store(
            &setup.agent_config,
            &setup.enriched_request,
            &state.rt.tool_registry,
            setup.llm_override.clone(),
            subagent_prompt,
            Some(state.store.runtime_quality_store.clone()),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if setup.reserved_cost > 0.0 {
                let _ = state
                    .obs
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
        state
            .store
            .session_store
            .append_message(&setup.session_id, msg)
            .await?;
        // Dual-write: persist as HistoryItems alongside legacy messages
        {
            let turn_id = xiaolin_protocol::TurnId::generate();
            let history_items = xiaolin_core::history_compat::chat_message_to_history(msg, turn_id);
            if let Err(e) = state
                .store
                .session_store
                .append_history_items(&setup.session_id, &history_items)
                .await
            {
                tracing::warn!(session_id = %setup.session_id, error = %e, "failed to dual-write history items");
            }
        }
    }
    if let Some(choice) = result.response.choices.first() {
        after_chat(&state, &setup, &choice.message, false).await?;
    }

    state
        .store
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
        maybe_spawn_smart_title_background(&state, &setup, &assistant_text);
    }

    xiaolin_observe::record_chat_latency(&setup.agent_id, request_start);

    let elapsed_ms = request_start.elapsed().as_millis() as f64;
    state
        .obs
        .metrics_collector
        .record_request(&setup.agent_id, "http");
    state
        .obs
        .metrics_collector
        .record_latency_ms("/api/v1/chat", elapsed_ms);
    if let Some(usage) = result.response.usage.as_ref() {
        let total = usage.total_tokens as u64;
        if total > 0 {
            state
                .obs
                .metrics_collector
                .record_tokens(&result.response.model, total);
        }
    }

    let mut resp_json = serde_json::to_value(&result.response)
        .map_err(|e| anyhow::anyhow!("serialization error: {e}"))?;
    if let Some(obj) = resp_json.as_object_mut() {
        let memory_enabled = {
            let live = state.cfg.config_live.load();
            live.get("memory")
                .and_then(|m| m.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(state.cfg.config.memory.enabled)
        };
        let mut meta = json!({
            "sessionId": &setup.session_id,
            "toolCallsMade": result.tool_calls_made,
            "iterations": result.iterations,
            "memoryInjected": memory_enabled,
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
/// Token deltas may be dropped under backpressure (see `send_stream_event` in `xiaolin-agent`).
async fn handle_stream(
    state: AppState,
    request: ChatRequest,
) -> Result<axum::response::Response, AppError> {
    use xiaolin_protocol::{AgentEvent, SessionId};
    use xiaolin_session_actor::SessionOp;

    let request_start = std::time::Instant::now();
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
    let agent_config = setup.agent_config.clone();

    for msg in &setup.user_messages {
        if let Err(e) = state
            .store
            .session_store
            .append_message(&session_id, msg)
            .await
        {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "stream: failed to persist user message to session"
            );
        } else {
            let turn_id = xiaolin_protocol::TurnId::generate();
            let history_items = xiaolin_core::history_compat::chat_message_to_history(msg, turn_id);
            if let Err(e) = state
                .store
                .session_store
                .append_history_items(&session_id, &history_items)
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "failed to dual-write history items");
            }
        }
    }

    let state_budget = state.clone();

    let stream_context_key = uuid::Uuid::new_v4().to_string();
    let mut op_extra = serde_json::Map::new();
    op_extra.insert(
        "_stream_context_key".into(),
        serde_json::Value::String(stream_context_key),
    );

    let typed_data = xiaolin_core::typed_turn_data::TypedTurnData::wrap(
        setup.enriched_request.clone(),
        agent_config.clone(),
    );

    let session_handle = state
        .svc
        .session_manager
        .get_or_create(SessionId::new(&session_id), &setup.agent_id)
        .await;

    let (_sub_id, mut event_rx) = session_handle
        .submit_and_subscribe(
            SessionOp::UserTurn {
                messages: serde_json::Value::Array(vec![]),
                agent_id: Some(setup.agent_id.clone()),
                model: setup.enriched_request.model.clone(),
                work_dir: setup.enriched_request.work_dir.clone(),
                extra: op_extra,
                typed_data: Some(typed_data),
            },
            128,
        )
        .await
        .map_err(|e| anyhow::anyhow!("session submit error: {e}"))?;

    let session_id_header = session_id.clone();
    let state_for_persist = state.clone();
    let setup_for_persist = setup.clone();

    let sse_stream = async_stream::stream! {
        let mut streamed_chars: usize = 0;
        let mut assistant_content = String::new();
        let mut assistant_reasoning = String::new();
        while let Some(se) = event_rx.recv().await {
            let event = se.msg;
            state_for_persist.store.event_log.append(&session_id, &event);
            if let AgentEvent::ReasoningDelta { ref content, .. } = event {
                assistant_reasoning.push_str(content);
            }
            match event {
                AgentEvent::ContentDelta {
                    delta,
                    raw_bytes,
                    ..
                } => {
                    let has_content = delta
                        .get("choices")
                        .and_then(|c| c.as_array())
                        .map(|choices| {
                            choices.iter().any(|choice| {
                                choice
                                    .get("delta")
                                    .map(|d| {
                                        d.get("role").and_then(|v| v.as_str()).is_some()
                                            || d.get("content")
                                                .and_then(|v| v.as_str())
                                                .map(|s| !s.is_empty())
                                                .unwrap_or(false)
                                            || d.get("tool_calls")
                                                .and_then(|v| v.as_array())
                                                .map(|t| !t.is_empty())
                                                .unwrap_or(false)
                                    })
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    if !has_content {
                        continue;
                    }
                    if let Some(choices) = delta.get("choices").and_then(|c| c.as_array()) {
                        for choice in choices {
                            if let Some(content) = choice
                                .get("delta")
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                assistant_content.push_str(content);
                                streamed_chars = streamed_chars.saturating_add(content.len());
                            }
                        }
                    }
                    let json_str = if let Some(ref raw) = raw_bytes {
                        String::from_utf8_lossy(raw).into_owned()
                    } else {
                        serde_json::to_string(&delta).unwrap_or_default()
                    };
                    yield Ok::<_, std::io::Error>(format!("data: {json_str}\n\n"));
                }
                AgentEvent::TurnEnd {
                    ref session_id,
                    ref summary,
                    ref final_tool_calls,
                    ..
                } => {
                    tracing::info!(
                        elapsed_ms = request_start.elapsed().as_millis() as u64,
                        "perf: http_stream_total"
                    );
                    let usage = summary.usage.as_ref();
                    let elapsed_ms = summary.elapsed_ms;
                    record_chat_budget_stream_estimate(
                        &state_budget,
                        model_for_budget.as_str(),
                        input_estimate,
                        streamed_chars,
                    );
                    state_budget.obs.metrics_collector.record_request(&setup_for_persist.agent_id, "http-stream");
                    state_budget.obs.metrics_collector.record_latency_ms("/api/v1/chat", elapsed_ms as f64);
                    if let Some(u) = usage {
                        let total = u.total_tokens as u64;
                        if total > 0 {
                            state_budget.obs.metrics_collector.record_tokens(model_for_budget.as_str(), total);
                        }
                    }
                    let core_tool_calls = final_tool_calls.as_ref().map(|tcs| {
                        tcs.iter()
                            .map(|tc| xiaolin_core::types::ToolCall {
                                id: tc.id.clone(),
                                call_type: tc.call_type.clone(),
                                function: xiaolin_core::types::FunctionCall {
                                    name: tc.function.name.clone(),
                                    arguments: tc.function.arguments.clone(),
                                },
                                output: tc.output.clone(),
                                success: tc.success,
                                duration_ms: tc.duration_ms,
                            })
                            .collect()
                    });
                    if !assistant_content.is_empty() || core_tool_calls.is_some() {
                        let assistant_msg = xiaolin_core::types::ChatMessage {
                            role: xiaolin_core::types::Role::Assistant,
                            content: if assistant_content.is_empty() {
                                None
                            } else {
                                Some(serde_json::Value::String(assistant_content.clone()))
                            },
                            reasoning_content: if assistant_reasoning.is_empty() {
                                None
                            } else {
                                Some(assistant_reasoning.clone())
                            },
                            tool_calls: core_tool_calls,
                        ..Default::default()
                        };
                        let _ = after_chat(
                            &state_for_persist,
                            &setup_for_persist,
                            &assistant_msg,
                            needs_title_stream,
                        )
                        .await;
                    }
                    if let Some(ref sid) = session_id {
                        let pt = usage.map(|u| u.prompt_tokens).unwrap_or(0);
                        let ct = usage.map(|u| u.completion_tokens).unwrap_or(0);
                        let tt = usage.map(|u| u.total_tokens).unwrap_or(0);
                        if let Err(e) = state_for_persist
                            .store
                            .session_store
                            .accumulate_usage(sid, pt, ct, elapsed_ms)
                            .await
                        {
                            tracing::warn!(error = %e, session_id = %sid, "failed to accumulate session usage");
                        }
                        if let Err(e) = state_for_persist
                            .store
                            .session_store
                            .stamp_last_assistant_usage(sid, pt, ct, tt, elapsed_ms)
                            .await
                        {
                            tracing::warn!(error = %e, session_id = %sid, "failed to stamp last assistant usage");
                        }
                    }
                    let mut val = serde_json::to_value(&event).unwrap_or_default();
                    if let Some(obj) = val.as_object_mut() {
                        if budget_degraded {
                            obj.insert("budgetDegraded".into(), json!(true));
                        }
                        obj.insert("resolvedAgent".into(), json!(&setup_for_persist.agent_id));
                        obj.insert("resolveReason".into(), json!(resolve_reason));
                        if let Some((est_tokens, ctx_window)) = setup_for_persist.context_tokens_estimate {
                            let actual_prompt = usage.map(|u| u.prompt_tokens).unwrap_or(0);
                            obj.insert(
                                "contextTokens".into(),
                                json!(if actual_prompt > 0 {
                                    actual_prompt
                                } else {
                                    est_tokens
                                }),
                            );
                            if ctx_window > 0 {
                                obj.insert("contextWindow".into(), json!(ctx_window));
                            }
                        }
                    }
                    let json_str = serde_json::to_string(&val).unwrap_or_default();
                    yield Ok(format!("event: turn_end\ndata: {json_str}\n\n"));
                    yield Ok("data: [DONE]\n\n".to_string());
                    break;
                }
                AgentEvent::Error { .. } => {
                    tracing::info!(
                        elapsed_ms = request_start.elapsed().as_millis() as u64,
                        "perf: http_stream_total"
                    );
                    if reserved > 0.0 {
                        let _ = state_budget.obs.budget_tracker.release_reservation(reserved);
                    }
                    if !assistant_content.is_empty() {
                        let assistant_msg = xiaolin_core::types::ChatMessage {
                            role: xiaolin_core::types::Role::Assistant,
                            content: Some(serde_json::Value::String(assistant_content.clone())),
                            reasoning_content: if assistant_reasoning.is_empty() {
                                None
                            } else {
                                Some(assistant_reasoning.clone())
                            },
                        ..Default::default()
                        };
                        let _ = after_chat(
                            &state_for_persist,
                            &setup_for_persist,
                            &assistant_msg,
                            false,
                        )
                        .await;
                    }
                    let json_str = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(format!("data: {json_str}\n\n"));
                    yield Ok("data: [DONE]\n\n".to_string());
                    break;
                }
                _ => {
                    let val = serde_json::to_value(&event).unwrap_or_default();
                    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("event");
                    let json_str = serde_json::to_string(&val).unwrap_or_default();
                    yield Ok(format!("event: {event_type}\ndata: {json_str}\n\n"));
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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResolveApprovalRequest {
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub(super) async fn resolve_approval(
    State(state): State<AppState>,
    Json(body): Json<ResolveApprovalRequest>,
) -> impl IntoResponse {
    use xiaolin_protocol::approval::ApprovalDecision;

    let decision = match body.decision.as_str() {
        "approved" => ApprovalDecision::Approved,
        "denied" => ApprovalDecision::Denied,
        "approved_for_session" => ApprovalDecision::ApprovedForSession,
        other => {
            return Json(json!({"ok": false, "error": format!("unknown decision: {other}")}))
                .into_response();
        }
    };

    let mut resolved = false;
    if let Some(ref sid) = body.session_id {
        if let Some(handle) = state
            .svc
            .session_manager
            .get(&xiaolin_protocol::SessionId::new(sid))
            .await
        {
            resolved = handle
                .submit(xiaolin_session_actor::SessionOp::ResolveApproval {
                    interaction_id: body.approval_id.clone(),
                    decision: decision.clone(),
                })
                .await
                .is_ok();
        }
    }

    Json(json!({"ok": resolved})).into_response()
}
