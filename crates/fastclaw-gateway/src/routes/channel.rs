use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

use crate::state::AppState;

use super::common::{
    apply_model_router_for_chat, filtered_tool_definitions, record_chat_budget_actual,
    record_chat_budget_stream_estimate,
};
use super::error::AppError;

fn headers_to_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            map.entry(name.as_str().to_string())
                .or_insert_with(|| v.to_string());
        }
    }
    map
}

/// Dynamic channel webhook dispatcher — routes to the appropriate ChannelPlugin.
pub(super) async fn channel_webhook(
    State(state): State<AppState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, AppError> {
    use fastclaw_core::channel::WebhookResult;

    let registry = state.channel_registry.read().await;
    let channel = registry
        .get(&channel_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("channel '{channel_id}' not registered")))?;
    drop(registry);

    let header_map = headers_to_map(&headers);
    channel
        .verify_webhook(&header_map, &body)
        .await
        .map_err(|e| {
            tracing::warn!(channel = %channel_id, error = %e, "webhook signature verification failed");
            AppError::Unauthorized(format!("webhook verification failed: {e}"))
        })?;

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;

    match channel.handle_webhook(payload).await {
        Ok(WebhookResult::Challenge(v)) => Ok(Json(v).into_response()),
        Ok(WebhookResult::Messages(messages)) => {
            for msg in messages {
                let state_clone = state.clone();
                let channel_clone = channel.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_channel_message(
                        state_clone,
                        channel_clone,
                        &msg.channel_id,
                        &msg.chat_id,
                        &msg.message_id,
                        &msg.text,
                    )
                    .await
                    {
                        tracing::error!(
                            error = %e,
                            channel = %msg.channel_id,
                            chat_id = %msg.chat_id,
                            "channel message handling failed"
                        );
                    }
                });
            }
            Ok(Json(json!({"code": 0})).into_response())
        }
        Ok(WebhookResult::Ignored) => Ok(Json(json!({"code": 0})).into_response()),
        Err(e) => {
            tracing::error!(error = %e, channel = %channel_id, "webhook handler error");
            Err(AppError::Internal(e))
        }
    }
}

/// Handle slash commands (e.g. `/skills`) locally without calling the LLM.
/// Returns `Some(response)` if the message is a recognized command, `None` otherwise.
async fn handle_slash_command(
    state: &AppState,
    channel_id: &str,
    chat_id: &str,
    text: &str,
) -> Option<String> {
    let trimmed = text.trim();

    if trimmed == "/skills" || trimmed == "/skills list" {
        let bindings = state.merged_route_bindings().await;
        let route = fastclaw_core::routing::resolve_route(
            &bindings,
            &state.config.agents,
            channel_id,
            None,
            None,
            Some(chat_id),
        );
        let agent_id = route.agent_id.as_str();

        let registry = state.skill_registry_for(agent_id);

        let skills = registry.list();
        let enabled: Vec<_> = skills
            .iter()
            .filter(|s| s.frontmatter.enabled.unwrap_or(true))
            .collect();

        if enabled.is_empty() {
            return Some(format!("📋 Agent `{agent_id}` 当前没有可用的 Skill。"));
        }

        let mut buf = format!(
            "📋 Agent `{agent_id}` 的 Skills ({} 个):\n\n",
            enabled.len()
        );
        for skill in &enabled {
            let desc = skill.description.as_deref().unwrap_or("(无描述)");
            let first_line = desc.lines().next().unwrap_or(desc);
            let layer = match skill.layer {
                fastclaw_core::skill::SkillLayer::Extension => "extension",
                fastclaw_core::skill::SkillLayer::Project => "project",
                fastclaw_core::skill::SkillLayer::Global => "global",
                fastclaw_core::skill::SkillLayer::AgentWorkspace => "workspace",
            };
            buf.push_str(&format!(
                "• **{}** (`{}`) [{}]\n  {}\n",
                skill.name, skill.id, layer, first_line
            ));
        }

        return Some(buf);
    }

    if trimmed == "/help" {
        return Some(
            "可用命令:\n• `/skills` — 列出当前 agent 的所有 Skill\n• `/help` — 显示帮助"
                .to_string(),
        );
    }

    None
}

/// Generic handler: process inbound messages from any channel plugin.
/// If the channel supports streaming, sends a placeholder and progressively updates it.
/// Uses multi-agent routing (bindings) and injects workspace bootstrap + skills.
pub(crate) async fn handle_channel_message(
    state: AppState,
    channel: Arc<dyn fastclaw_core::channel::ChannelPlugin>,
    channel_id: &str,
    chat_id: &str,
    message_id: &str,
    text: &str,
) -> anyhow::Result<()> {
    use fastclaw_core::config::DmScope;
    use fastclaw_core::routing::{build_session_key, resolve_route};

    if let Some(response) = handle_slash_command(&state, channel_id, chat_id, text).await {
        channel.reply_message(message_id, &response).await?;
        return Ok(());
    }

    let bindings = state.merged_route_bindings().await;
    let route = resolve_route(
        &bindings,
        &state.config.agents,
        channel_id,
        None,
        None,
        Some(chat_id),
    );
    let agent_id = route.agent_id.as_str();

    if let Some(agent_entry) = state.config.agents.list.iter().find(|a| a.id == agent_id) {
        if let Some(ref gc) = agent_entry.group_chat {
            if gc.require_mention == Some(true) {
                let mentioned = gc
                    .mention_patterns
                    .iter()
                    .any(|pat| text.contains(pat.as_str()));
                if !mentioned {
                    tracing::debug!(agent_id, "skipping non-mentioned group message");
                    return Ok(());
                }
            }
        }
    }

    let dm_scope = state
        .config
        .session
        .dm_scope
        .clone()
        .unwrap_or(DmScope::PerChannelPeer);
    let session_key = build_session_key(&dm_scope, agent_id, channel_id, None, chat_id, "p2p");

    if state
        .session_store
        .get_session(&session_key)
        .await?
        .is_none()
    {
        state
            .session_store
            .create_session(&session_key, agent_id, None)
            .await?;
    }
    let session = state
        .session_store
        .get_session(&session_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to create session"))?;

    state
        .session_store
        .append_message(
            &session.id,
            &ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String(text.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        )
        .await?;

    let mut messages = state.session_store.load_chat_messages(&session.id).await?;

    let mut system_context = String::new();
    if let Some(workspace) = state.workspaces.get(agent_id) {
        let bootstrap = workspace.load_bootstrap();
        let bs_prompt = bootstrap.format_for_prompt();
        if !bs_prompt.is_empty() {
            system_context.push_str(&bs_prompt);
        }
    }
    let agent_skill_reg = state.skill_registry_for(agent_id);
    let skills_prompt = agent_skill_reg.format_for_prompt_mode(&state.config.skills.prompt_mode);
    if !skills_prompt.is_empty() {
        system_context.push_str(&skills_prompt);
    }
    if !system_context.is_empty() {
        messages.insert(
            0,
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(system_context)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    let use_streaming = channel.capabilities().streaming;

    let router = state.router.read().await;
    let agent_config = router
        .resolve(&ChatRequest {
            messages: messages.clone(),
            stream: use_streaming,
            model: None,
            temperature: None,
            max_tokens: None,
            agent_id: Some(agent_id.into()),
            session_id: Some(session.id.clone()),
            tools: None,
            slash_intent: None,
            work_dir: None,
        })
        .map(|c| c.clone())
        .map_err(|e| anyhow::anyhow!("agent resolve: {}", e))?;
    drop(router);

    let tools = filtered_tool_definitions(&state.tool_registry, &agent_config);
    let tool_definition_count = tools.as_ref().map_or(0, |t| t.len());

    let mut request = ChatRequest {
        messages,
        stream: use_streaming,
        model: None,
        temperature: None,
        max_tokens: None,
        agent_id: Some(agent_id.into()),
        session_id: Some(session.id.clone()),
        tools,
        slash_intent: None,
        work_dir: None,
    };

    let llm_override =
        apply_model_router_for_chat(&state, &agent_config, &mut request, tool_definition_count);

    if use_streaming {
        let text = handle_channel_streaming(
            &state,
            &channel,
            &agent_config,
            &request,
            message_id,
            llm_override,
        )
        .await?;
        state
            .session_store
            .append_message(
                &session.id,
                &ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(text.clone())),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            )
            .await?;
    } else {
        let result = state
            .runtime
            .execute(&agent_config, &request, &state.tool_registry, llm_override)
            .await?;
        let charged_model = result.response.model.clone();
        record_chat_budget_actual(
            &state,
            charged_model.as_str(),
            result.response.usage.as_ref(),
        );
        let reply_text = result
            .response
            .choices
            .first()
            .and_then(|c| c.message.text_content())
            .unwrap_or_else(|| "(no response)".to_string());

        channel.reply_message(message_id, &reply_text).await?;

        if let Some(choice) = result.response.choices.first() {
            state
                .session_store
                .append_message(&session.id, &choice.message)
                .await?;
        } else {
            state
                .session_store
                .append_message(
                    &session.id,
                    &ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(reply_text.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                )
                .await?;
        }
    }

    tracing::info!(
        channel_id,
        chat_id,
        message_id,
        session = %session.id,
        streaming = use_streaming,
        "channel message processed and replied"
    );

    Ok(())
}

/// Streaming handler for channels that support message editing (e.g. Feishu).
async fn handle_channel_streaming(
    state: &AppState,
    channel: &Arc<dyn fastclaw_core::channel::ChannelPlugin>,
    agent_config: &fastclaw_core::agent_config::AgentConfig,
    request: &fastclaw_core::types::ChatRequest,
    original_message_id: &str,
    llm_override: Option<Arc<dyn fastclaw_agent::LlmProvider>>,
) -> anyhow::Result<String> {
    use fastclaw_core::types::StreamEvent;

    let placeholder_resp = channel
        .reply_streaming_placeholder(original_message_id, "思考中...")
        .await?;

    let reply_msg_id = placeholder_resp
        .get("message_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            placeholder_resp
                .get("data")
                .and_then(|d| d.get("message_id"))
                .and_then(|v| v.as_str())
        })
        .map(String::from)
        .unwrap_or_default();

    if reply_msg_id.is_empty() {
        tracing::warn!("streaming: could not extract reply message_id from placeholder response, falling back to non-streaming");
        let result = state
            .runtime
            .execute(agent_config, request, &state.tool_registry, llm_override)
            .await?;
        let charged_model = result.response.model.clone();
        record_chat_budget_actual(
            state,
            charged_model.as_str(),
            result.response.usage.as_ref(),
        );
        let text = result
            .response
            .choices
            .first()
            .and_then(|c| c.message.text_content())
            .unwrap_or_else(|| "(no response)".to_string());
        return Ok(text);
    }

    tracing::debug!(reply_msg_id = %reply_msg_id, "streaming: placeholder sent, starting LLM stream");

    let tool_definition_count = request.tools.as_ref().map_or(0, |t| t.len());
    let input_estimate = fastclaw_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &request.messages,
        tool_definition_count,
    );
    let model_for_budget = request
        .model
        .clone()
        .unwrap_or_else(|| agent_config.model.model.clone());

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(1024);

    let runtime = state.runtime.clone();
    let tool_reg = state.tool_registry.clone();
    let config = agent_config.clone();
    let req = request.clone();
    let llm_spawn = llm_override.clone();
    let state_budget = state.clone();

    tokio::spawn(async move {
        if let Err(e) = runtime
            .execute_stream(&config, &req, &tool_reg, tx.clone(), llm_spawn)
            .await
        {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
        }
    });

    let mut accumulated = String::new();
    let mut last_update = std::time::Instant::now();
    let update_interval = std::time::Duration::from_millis(800);
    let channel_for_update = channel.clone();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Delta(delta) => {
                for choice in &delta.choices {
                    if let Some(ref content) = choice.delta.content {
                        accumulated.push_str(content);
                    }
                }

                if last_update.elapsed() >= update_interval && !accumulated.is_empty() {
                    let text = format!("{}▍", accumulated);
                    if let Err(e) = channel_for_update
                        .update_message(&reply_msg_id, &text)
                        .await
                    {
                        tracing::debug!(error = %e, "streaming: update_message failed (will retry)");
                    }
                    last_update = std::time::Instant::now();
                }
            }
            StreamEvent::ToolExecuting { tool_name, .. } => {
                let prefix = if accumulated.is_empty() {
                    String::new()
                } else {
                    format!("{accumulated}\n\n")
                };
                let status = format!("{prefix}🔧 调用工具: {tool_name}...");
                let _ = channel_for_update
                    .update_message(&reply_msg_id, &status)
                    .await;
                last_update = std::time::Instant::now();
            }
            StreamEvent::Done { .. } => {
                record_chat_budget_stream_estimate(
                    &state_budget,
                    model_for_budget.as_str(),
                    input_estimate,
                    accumulated.len(),
                );
                break;
            }
            StreamEvent::Error(e) => {
                tracing::error!(error = %e, "streaming: LLM error");
                if accumulated.is_empty() {
                    accumulated = format!("(错误: {e})");
                }
                break;
            }
            _ => {}
        }
    }

    if accumulated.is_empty() {
        accumulated = "(no response)".to_string();
    }

    if let Err(e) = channel.update_message(&reply_msg_id, &accumulated).await {
        tracing::warn!(error = %e, "streaming: final update_message failed");
    }

    tracing::info!(
        reply_msg_id = %reply_msg_id,
        content_len = accumulated.len(),
        "streaming: completed"
    );

    Ok(accumulated)
}

pub(super) async fn list_channels(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.channel_registry.read().await;
    let channels: Vec<_> = registry
        .list()
        .into_iter()
        .map(|m| -> Value {
            json!({
                "id": m.id,
                "name": m.name,
                "description": m.description,
                "aliases": m.aliases,
            })
        })
        .collect();
    Json(json!({ "channels": channels, "count": channels.len() }))
}
