use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

use crate::chat_pipeline::{self, SetupChatOptions};
use crate::state::AppState;

use super::common::{record_chat_budget_actual, record_chat_budget_stream_estimate};
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

    let registry = state.ext.channel_registry.read().await;
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
                let account_id = msg.account_id.clone();
                let chat_type = msg.chat_type.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_channel_message(
                        state_clone,
                        channel_clone,
                        &msg.channel_id,
                        &msg.chat_id,
                        &msg.message_id,
                        &msg.text,
                        account_id.as_deref(),
                        &chat_type,
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
    account_id: Option<&str>,
    chat_type: &str,
) -> Option<String> {
    let trimmed = text.trim();

    if trimmed == "/skills" || trimmed == "/skills list" {
        let bindings = state.merged_route_bindings().await;
        let route = fastclaw_core::routing::resolve_route(
            &bindings,
            &state.cfg.config.agents,
            channel_id,
            account_id,
            Some(chat_type),
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

    if trimmed == "/new" || trimmed == "/new session" || trimmed == "/reset" {
        let bindings = state.merged_route_bindings().await;
        let route = fastclaw_core::routing::resolve_route(
            &bindings,
            &state.cfg.config.agents,
            channel_id,
            account_id,
            Some(chat_type),
            Some(chat_id),
        );
        let agent_id = route.agent_id.as_str();

        let dm_scope = state
            .cfg
            .config
            .session
            .dm_scope
            .clone()
            .unwrap_or(fastclaw_core::config::DmScope::PerChannelPeer);
        let session_key =
            fastclaw_core::routing::build_session_key(&dm_scope, agent_id, channel_id, account_id, chat_id, chat_type);

        let deleted = state
            .store
            .session_store
            .delete_session(&session_key)
            .await
            .unwrap_or(false);

        if deleted {
            return Some("🔄 已开启新对话，之前的上下文已清除。".to_string());
        } else {
            return Some("🔄 已就绪，当前没有历史上下文。".to_string());
        }
    }

    if trimmed == "/help" {
        return Some(
            "可用命令:\n• `/new` — 开启新对话（清除上下文）\n• `/skills` — 列出当前 agent 的所有 Skill\n• `/help` — 显示帮助"
                .to_string(),
        );
    }

    None
}

/// Generic handler: process inbound messages from any channel plugin.
/// If the channel supports streaming, sends a placeholder and progressively updates it.
///
/// Uses the shared `setup_chat()` pipeline so IM channels get the same capabilities
/// as HTTP/WS sessions: workspace paths, context engine, model routing, skills,
/// prompt routing, budget tracking, and the full coding toolchain.
pub(crate) async fn handle_channel_message(
    state: AppState,
    channel: Arc<dyn fastclaw_core::channel::ChannelPlugin>,
    channel_id: &str,
    chat_id: &str,
    message_id: &str,
    text: &str,
    account_id: Option<&str>,
    chat_type: &str,
) -> anyhow::Result<()> {
    use fastclaw_core::config::DmScope;
    use fastclaw_core::routing::{build_session_key, resolve_route};

    if let Some(response) = handle_slash_command(&state, channel_id, chat_id, text, account_id, chat_type).await {
        channel.reply_message(message_id, &response).await?;
        return Ok(());
    }

    let bindings = state.merged_route_bindings().await;
    let route = resolve_route(
        &bindings,
        &state.cfg.config.agents,
        channel_id,
        account_id,
        Some(chat_type),
        Some(chat_id),
    );
    let agent_id = route.agent_id.as_str();

    if let Some(agent_entry) = state.cfg.config.agents.list.iter().find(|a| a.id == agent_id) {
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
        .cfg
        .config
        .session
        .dm_scope
        .clone()
        .unwrap_or(DmScope::PerChannelPeer);
    let session_key = build_session_key(&dm_scope, agent_id, channel_id, account_id, chat_id, chat_type);

    if state
        .store
        .session_store
        .get_session(&session_key)
        .await?
        .is_none()
    {
        state
            .store
            .session_store
            .create_session_full(&session_key, agent_id, None, None, Some(channel_id))
            .await?;
    }

    state
        .store
        .session_store
        .append_message(
            &session_key,
            &ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String(text.to_string())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        )
        .await?;

    // Resolve the agent's workspace root so file tools target the correct directory.
    let work_dir = state
        .rt
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.to_string_lossy().to_string());

    let use_streaming = channel.capabilities().streaming;

    // Build a ChatRequest and run through the shared setup_chat() pipeline.
    // This gives IM channels the same enrichment as HTTP/WS: Runtime Paths,
    // context engine (memory/RAG), model routing, prompt routing, skills, and budget.
    let user_msg = ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(text.to_string())),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };

    let request = ChatRequest {
        messages: vec![user_msg],
        stream: use_streaming,
        model: None,
        temperature: None,
        max_tokens: None,
        agent_id: Some(agent_id.into()),
        session_id: Some(session_key.clone()),
        tools: None,
        slash_intent: None,
        work_dir,
    };

    let options = SetupChatOptions {
        chat_stream: use_streaming,
        propagate_context_ingest_errors: false,
        set_resolved_session_on_request: true,
        record_chat_observe: true,
    };

    let setup = chat_pipeline::setup_chat(&state, &request, options)
        .await
        .map_err(|e| anyhow::anyhow!("channel setup_chat failed: {e}"))?;

    // Inject channel context so the agent knows it is operating from IM
    let mut enriched_request = setup.enriched_request.clone();
    inject_channel_context(&mut enriched_request.messages, channel_id, chat_id);

    // Inject channel-scoped tool definitions so the LLM can see them
    let ch_tools = state.rt.tool_registry.channel_scoped_definitions();
    if !ch_tools.is_empty() {
        enriched_request.tools = Some(ch_tools);
    }

    if use_streaming {
        let reply_text = handle_channel_streaming(
            &state,
            &channel,
            &setup.agent_config,
            &enriched_request,
            message_id,
            chat_id,
            setup.llm_override.clone(),
        )
        .await?;

        let assistant_msg = ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(reply_text.clone())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        chat_pipeline::after_chat(&state, &setup, &assistant_msg, true)
            .await
            .map_err(|e| anyhow::anyhow!("channel after_chat failed: {e}"))?;
    } else {
        let result = state
            .rt
            .runtime
            .execute(
                &setup.agent_config,
                &enriched_request,
                &state.rt.tool_registry,
                setup.llm_override.clone(),
            )
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

        let assistant_msg = result
            .response
            .choices
            .first()
            .map(|c| c.message.clone())
            .unwrap_or(ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String(reply_text.clone())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        chat_pipeline::after_chat(&state, &setup, &assistant_msg, true)
            .await
            .map_err(|e| anyhow::anyhow!("channel after_chat failed: {e}"))?;
    }

    tracing::info!(
        channel_id,
        chat_id,
        message_id,
        session = %setup.session_id,
        streaming = use_streaming,
        agent = %setup.agent_id,
        "channel message processed via unified pipeline"
    );

    Ok(())
}

/// Inject a `[Channel Context]` system message so the agent knows it is operating
/// from an IM channel and can tailor its response format (concise, no code fences
/// for simple answers, etc.).
fn inject_channel_context(messages: &mut Vec<ChatMessage>, channel_id: &str, chat_id: &str) {
    let prompt = format!(
        "[Channel Context]\n\
         This conversation is happening through an IM channel (not a terminal/IDE).\n\
         Channel: {channel_id}\n\
         Chat: {chat_id}\n\n\
         Guidance:\n\
         - You have FULL access to the coding toolchain (read/write files, shell, grep, etc.).\n\
         - The user may ask you to fix code, run builds, check tests, etc. — use your tools.\n\
         - Keep responses concise and IM-friendly. Use short summaries instead of full file dumps.\n\
         - When you perform file edits or run commands, report a brief summary of what changed.\n\
         - For compile/test results, report pass/fail status and key metrics."
    );
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(prompt)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    );
}

/// Streaming handler for channels that support message editing (e.g. Feishu).
/// Supports ask_question: when the agent emits an AskQuestion event, an interactive
/// card is sent to the chat and the handler waits for the user's button click.
async fn handle_channel_streaming(
    state: &AppState,
    channel: &Arc<dyn fastclaw_core::channel::ChannelPlugin>,
    agent_config: &fastclaw_core::agent_config::AgentConfig,
    request: &fastclaw_core::types::ChatRequest,
    original_message_id: &str,
    chat_id: &str,
    llm_override: Option<Arc<dyn fastclaw_agent::LlmProvider>>,
) -> anyhow::Result<String> {
    use crate::ask_question_card::{AskQuestionCardBuilder, QuestionOption};
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
            .rt
            .runtime
            .execute(agent_config, request, &state.rt.tool_registry, llm_override)
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

    let runtime = state.rt.runtime.clone();
    let tool_reg = state.rt.tool_registry.clone();
    let config = agent_config.clone();
    let req = request.clone();
    let llm_spawn = llm_override.clone();
    let state_budget = state.clone();
    let confirm_pending = state.strm.ask_question_pending.clone();
    let stream_event_tx_map = state.strm.stream_event_tx.clone();
    let stream_context_key = uuid::Uuid::new_v4().to_string();

    stream_event_tx_map.insert(stream_context_key.clone(), tx.clone());

    let stream_key_for_task = stream_context_key.clone();
    let confirm_pending_for_task = confirm_pending.clone();
    tokio::spawn(async move {
        let result = fastclaw_agent::builtin_tools::with_stream_context(
            stream_key_for_task,
            runtime.execute_stream_with_confirm(
                &config,
                &req,
                &tool_reg,
                tx.clone(),
                llm_spawn,
                confirm_pending_for_task,
                None,
                None,
                None,
                None,
            ),
        )
        .await;
        if let Err(e) = result {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
        }
    });

    let mut accumulated = String::new();
    let mut last_update = std::time::Instant::now();
    let update_interval = std::time::Duration::from_millis(800);
    let channel_for_update = channel.clone();
    let supports_cards = channel.supports_interactive_questions();
    let session_id = request
        .session_id
        .clone()
        .unwrap_or_default();

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
            StreamEvent::AskQuestion {
                request_id,
                question,
                options,
                timeout_secs,
                allow_multiple,
            } => {
                if supports_cards {
                    let card_options: Vec<QuestionOption> = options
                        .iter()
                        .map(|o| QuestionOption {
                            id: o.id.clone(),
                            label: o.label.clone(),
                        })
                        .collect();
                    let mut builder = AskQuestionCardBuilder::new(
                        question.clone(),
                        card_options,
                        session_id.clone(),
                        request_id.clone(),
                    );
                    builder = builder.allow_multiple(allow_multiple);
                    if timeout_secs > 0 {
                        builder = builder.timeout_secs(timeout_secs);
                    }
                    let card = builder.build();
                    match channel_for_update
                        .send_interactive_card(chat_id, "chat_id", &card)
                        .await
                    {
                        Ok(card_msg_id) => {
                            tracing::info!(
                                request_id = %request_id,
                                card_msg_id = %card_msg_id,
                                "streaming: sent ask_question card"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "streaming: failed to send ask_question card, answering with fallback");
                            if let Some((_, tx)) = confirm_pending.remove(&request_id) {
                                let _ = tx.send(options.first().map(|o| o.id.clone()).unwrap_or_default());
                            }
                        }
                    }
                } else {
                    tracing::debug!("streaming: channel does not support interactive cards, auto-selecting first option");
                    if let Some((_, tx)) = confirm_pending.remove(&request_id) {
                        let _ = tx.send(options.first().map(|o| o.id.clone()).unwrap_or_default());
                    }
                }
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

    stream_event_tx_map.remove(&stream_context_key);

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
    let registry = state.ext.channel_registry.read().await;
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
