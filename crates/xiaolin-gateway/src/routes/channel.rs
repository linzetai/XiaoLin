use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use xiaolin_core::types::{ChatMessage, ChatRequest, Role};

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
    use xiaolin_core::channel::WebhookResult;

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
            AppError::Unauthorized("webhook verification failed".into())
        })?;

    let payload: serde_json::Value = channel.parse_webhook_payload(&body).map_err(|e| {
        tracing::warn!(channel = %channel_id, error = %e, "invalid webhook payload");
        AppError::BadRequest("invalid webhook payload".into())
    })?;

    match channel.handle_webhook(payload).await {
        Ok(WebhookResult::Challenge(v)) => Ok(Json(v).into_response()),
        Ok(WebhookResult::Messages(messages)) => {
            for msg in messages {
                let state_clone = state.clone();
                let channel_clone = channel.clone();
                let account_id = msg.account_id.clone();
                let chat_type = msg.chat_type.clone();
                let attachments = msg.attachments.clone();
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
                        attachments,
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
        let agent_id = "main";
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
                xiaolin_core::skill::SkillLayer::Extension => "extension",
                xiaolin_core::skill::SkillLayer::Project
                | xiaolin_core::skill::SkillLayer::ProjectCursor
                | xiaolin_core::skill::SkillLayer::ProjectFastclaw => "project",
                xiaolin_core::skill::SkillLayer::Global => "global",
                xiaolin_core::skill::SkillLayer::AgentWorkspace => "workspace",
                xiaolin_core::skill::SkillLayer::SharedAgents => "shared",
                xiaolin_core::skill::SkillLayer::UserCodex
                | xiaolin_core::skill::SkillLayer::UserCursor => "user",
            };
            buf.push_str(&format!(
                "• **{}** (`{}`) [{}]\n  {}\n",
                skill.name, skill.id, layer, first_line
            ));
        }

        return Some(buf);
    }

    if trimmed == "/new" || trimmed == "/new session" || trimmed == "/reset" {
        let agent_id = "main";
        let dm_scope = state
            .cfg
            .config
            .session
            .dm_scope
            .clone()
            .unwrap_or(xiaolin_core::config::DmScope::PerChannelPeer);
        let session_key = xiaolin_core::routing::build_session_key(
            &dm_scope, agent_id, channel_id, account_id, chat_id, chat_type,
        );

        let deleted = state
            .store
            .session_store
            .delete_session(&session_key)
            .await
            .unwrap_or(false);

        if deleted {
            state.cleanup_session_resources(&session_key).await;
            return Some("🔄 已开启新对话，之前的上下文已清除。".to_string());
        } else {
            return Some("🔄 已就绪，当前没有历史上下文。".to_string());
        }
    }

    if trimmed == "/stop" {
        return Some("⏹ 已停止当前处理。".to_string());
    }

    if trimmed == "/model" || trimmed == "/model list" {
        let current_override = state.ext.chat_model_overrides.get(chat_id).map(|v| v.clone());
        let agent_id_local = "main";
        let default_model = state
            .cfg
            .config
            .agents
            .list
            .iter()
            .find(|a| a.id == agent_id_local)
            .and_then(|a| a.model.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let live = state.cfg.config_live.load();
        let available: Vec<String> = live
            .get("models")
            .and_then(|v| v.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_else(|| state.cfg.config.models.keys().cloned().collect());

        let mut buf = format!("🤖 当前模型: **{}**\n", current_override.as_deref().unwrap_or(&default_model));
        if current_override.is_some() {
            buf.push_str(&format!("   (默认: {})\n", default_model));
        }
        buf.push_str(&format!("\n可用模型 ({} 个):\n", available.len()));
        for name in &available {
            let marker = if Some(name.as_str()) == current_override.as_deref()
                || (current_override.is_none() && name == &default_model)
            {
                " ← 当前"
            } else {
                ""
            };
            buf.push_str(&format!("• `{name}`{marker}\n"));
        }
        buf.push_str("\n用法: `/model <模型名>` 切换模型\n用法: `/model reset` 恢复默认");
        return Some(buf);
    }

    if let Some(rest) = trimmed.strip_prefix("/model ") {
        let model_name = rest.trim();
        if model_name == "reset" || model_name == "default" {
            state.ext.chat_model_overrides.remove(chat_id);
            let default_model = state
                .cfg
                .config
                .agents
                .list
                .iter()
                .find(|a| a.id == "main")
                .and_then(|a| a.model.clone())
                .unwrap_or_else(|| "unknown".to_string());
            return Some(format!("🤖 已恢复默认模型: **{default_model}**"));
        }
        let live = state.cfg.config_live.load();
        let model_exists = live
            .get("models")
            .and_then(|v| v.as_object())
            .map(|m| m.contains_key(model_name))
            .unwrap_or_else(|| state.cfg.config.models.contains_key(model_name));
        if !model_exists {
            let available: Vec<String> = live
                .get("models")
                .and_then(|v| v.as_object())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_else(|| state.cfg.config.models.keys().cloned().collect());
            return Some(format!(
                "❌ 未知模型 `{model_name}`\n\n可用模型: {}",
                available.join(", ")
            ));
        }
        state
            .ext
            .chat_model_overrides
            .insert(chat_id.to_string(), model_name.to_string());
        return Some(format!("🤖 已切换到模型: **{model_name}**"));
    }

    if trimmed == "/workspace" {
        let agent_id = "main";
        let dm_scope = state
            .cfg
            .config
            .session
            .dm_scope
            .clone()
            .unwrap_or(xiaolin_core::config::DmScope::PerChannelPeer);
        let session_key = xiaolin_core::routing::build_session_key(
            &dm_scope, agent_id, channel_id, account_id, chat_id, chat_type,
        );
        if let Ok(Some(session)) = state.store.session_store.get_session(&session_key).await {
            if let Some(wd) = session.work_dir.as_deref() {
                return Some(format!("📂 当前工作区: `{wd}`"));
            }
        }
        return Some("📂 当前没有设置工作区。\n\n用法: `/workspace /path/to/project` 设置工作区".to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("/workspace ") {
        let path = rest.trim();
        if path.is_empty() {
            return Some("❌ 请提供路径。用法: `/workspace /path/to/project`".to_string());
        }
        let target = std::path::Path::new(path);
        if !target.exists() {
            return Some(format!("❌ 路径不存在: `{path}`"));
        }
        let ws_root = xiaolin_core::workspace::detect_workspace_root(target);
        let ws_str = ws_root.display().to_string();

        let agent_id = "main";
        let dm_scope = state
            .cfg
            .config
            .session
            .dm_scope
            .clone()
            .unwrap_or(xiaolin_core::config::DmScope::PerChannelPeer);
        let session_key = xiaolin_core::routing::build_session_key(
            &dm_scope, agent_id, channel_id, account_id, chat_id, chat_type,
        );
        let _ = state.store.session_store.update_work_dir(&session_key, Some(&ws_str)).await;
        let _ = state.strm.ws_broadcast.send(
            serde_json::json!({"type":"event","event":"sessions.changed","data":{"sessionId": &session_key}}).to_string(),
        );
        return Some(format!("📂 工作区已设置为: `{ws_str}`"));
    }

    if trimmed == "/init" || trimmed.starts_with("/init ") {
        let path = trimmed.strip_prefix("/init").unwrap().trim();
        let target = if path.is_empty() {
            std::env::current_dir().ok()
        } else {
            let p = std::path::PathBuf::from(path);
            if p.exists() { Some(p) } else { None }
        };
        let Some(target) = target else {
            return Some(format!("❌ 路径不存在: `{path}`"));
        };
        let ws_root = xiaolin_core::workspace::detect_workspace_root(&target);
        let xiaolin_dir = ws_root.join(".xiaolin");
        if xiaolin_dir.exists() {
            return Some(format!("✅ `.xiaolin/` 已存在于 `{}`", ws_root.display()));
        }
        let result = (|| -> anyhow::Result<Vec<String>> {
            let mut created = Vec::new();
            std::fs::create_dir_all(xiaolin_dir.join("skills"))?;
            created.push(".xiaolin/skills/".into());
            std::fs::create_dir_all(xiaolin_dir.join("rules"))?;
            created.push(".xiaolin/rules/".into());
            let cfg = serde_json::json!({
                "// XiaoLin project-level configuration": "Override user/global settings for this project.",
            });
            std::fs::write(xiaolin_dir.join("config.json"), serde_json::to_string_pretty(&cfg)? + "\n")?;
            created.push(".xiaolin/config.json".into());
            let mcp = serde_json::json!({ "mcpServers": {} });
            std::fs::write(xiaolin_dir.join("mcp.json"), serde_json::to_string_pretty(&mcp)? + "\n")?;
            created.push(".xiaolin/mcp.json".into());
            Ok(created)
        })();
        return match result {
            Ok(created) => Some(format!(
                "✅ 已在 `{}` 初始化 .xiaolin/\n\n创建:\n{}",
                ws_root.display(),
                created.iter().map(|f| format!("• `{f}`")).collect::<Vec<_>>().join("\n")
            )),
            Err(e) => Some(format!("❌ 初始化失败: {e}")),
        };
    }

    if trimmed == "/skillify" {
        return Some(
            "💡 `/skillify` 需要在聊天中使用——它会分析当前会话上下文，从中提取可复用的模式并生成新的 Skill。\n\n\
             请在对话中直接发送 `/skillify`，AI 会自动：\n\
             1. 分析会话中的多步骤操作模式\n\
             2. 生成 SKILL.md 草稿供你审查\n\
             3. 确认后保存到项目 `.xiaolin/skills/` 目录"
                .to_string(),
        );
    }

    if trimmed == "/help" {
        return Some(
            "可用命令:\n\
             • `/new` — 开启新对话（清除上下文）\n\
             • `/stop` — 停止当前正在处理的回复\n\
             • `/model` — 查看/切换模型\n\
             • `/workspace` — 查看/设置工作区\n\
             • `/init` — 初始化 XiaoLin 项目配置\n\
             • `/skills` — 列出当前 agent 的所有 Skill\n\
             • `/skillify` — 从当前会话提取可复用模式生成 Skill\n\
             • `/help` — 显示帮助"
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
///
/// TODO: Unify inbound message handling with `ws/chat.rs` — both paths should share
/// the same session setup, slash-command routing, attachment normalization, and
/// streaming reply logic to avoid behavioral drift between channel webhooks and WS chat.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_channel_message(
    state: AppState,
    channel: Arc<dyn xiaolin_core::channel::ChannelPlugin>,
    channel_id: &str,
    chat_id: &str,
    message_id: &str,
    text: &str,
    account_id: Option<&str>,
    chat_type: &str,
    attachments: Vec<xiaolin_core::channel::Attachment>,
) -> anyhow::Result<()> {
    use xiaolin_core::config::DmScope;
    use xiaolin_core::routing::build_session_key;

    if let Some(response) =
        handle_slash_command(&state, channel_id, chat_id, text, account_id, chat_type).await
    {
        channel.reply_message(message_id, &response).await?;
        return Ok(());
    }

    let agent_id = "main";

    if let Some(agent_entry) = state
        .cfg
        .config
        .agents
        .list
        .iter()
        .find(|a| a.id == agent_id)
    {
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
    let session_key = build_session_key(
        &dm_scope, agent_id, channel_id, account_id, chat_id, chat_type,
    );

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

    let user_content = if attachments.is_empty() {
        serde_json::Value::String(text.to_string())
    } else {
        let mut parts = Vec::new();
        for att in &attachments {
            if att
                .mime_type
                .as_deref()
                .is_some_and(|m| m.starts_with("image/"))
            {
                if let Ok(bytes) = tokio::fs::read(&att.file_path).await {
                    let mime = att.mime_type.as_deref().unwrap_or("image/png");
                    use base64::Engine as _;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{mime};base64,{b64}")
                        }
                    }));
                }
            } else {
                let name = att.file_name.as_deref().unwrap_or("file");
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": format!("[Attachment: {} ({})]", name, att.mime_type.as_deref().unwrap_or("unknown"))
                }));
            }
        }
        if !text.is_empty() {
            parts.push(serde_json::json!({
                "type": "text",
                "text": text
            }));
        }
        serde_json::Value::Array(parts)
    };
    let user_msg = ChatMessage {
        role: Role::User,
        content: Some(user_content),
    ..Default::default()
    };
    state
        .store
        .session_store
        .append_message(&session_key, &user_msg)
        .await?;
    // Dual-write: persist as HistoryItems alongside legacy messages
    {
        let turn_id = xiaolin_protocol::TurnId::generate();
        let history_items =
            xiaolin_core::history_compat::chat_message_to_history(&user_msg, turn_id);
        if let Err(e) = state
            .store
            .session_store
            .append_history_items(&session_key, &history_items)
            .await
        {
            tracing::warn!(session_key = %session_key, error = %e, "failed to dual-write history items in channel");
        }
    }

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
    let model_override = state
        .ext
        .chat_model_overrides
        .get(chat_id)
        .map(|v| v.clone());

    let request = ChatRequest {
        messages: vec![user_msg],
        stream: use_streaming,
        model: model_override,
        temperature: None,
        max_tokens: None,
        agent_id: Some(agent_id.into()),
        session_id: Some(session_key.clone().into()),
        tools: None,
        slash_intent: None,
        work_dir,
        response_language: None,
        goal_mode: None,
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
    let resolved_model = enriched_request
        .model
        .clone()
        .unwrap_or_else(|| setup.agent_config.model.model.clone());
    inject_channel_context(
        &mut enriched_request.messages,
        channel_id,
        chat_id,
        &resolved_model,
    );

    // Inject channel-scoped tool definitions so the LLM can see them
    let ch_tools = state.rt.tool_registry.channel_scoped_definitions();
    if !ch_tools.is_empty() {
        enriched_request.tools = Some(ch_tools);
    }

    channel.on_processing_start(chat_id, message_id).await;

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
        ..Default::default()
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
            .map(|c| c.into_owned())
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
            ..Default::default()
            });
        chat_pipeline::after_chat(&state, &setup, &assistant_msg, true)
            .await
            .map_err(|e| anyhow::anyhow!("channel after_chat failed: {e}"))?;
    }

    channel.on_processing_end(chat_id, message_id).await;

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
fn inject_channel_context(
    messages: &mut Vec<ChatMessage>,
    channel_id: &str,
    chat_id: &str,
    model_name: &str,
) {
    let prompt = format!(
        "[Channel Context]\n\
         This conversation is happening through an IM channel (not a terminal/IDE).\n\
         Channel: {channel_id}\n\
         Chat: {chat_id}\n\
         Model: {model_name}\n\n\
         Guidance:\n\
         - You are powered by the model: {model_name}. When asked about your model, report this name.\n\
         - You have FULL access to the coding toolchain (read/write files, shell, grep, etc.).\n\
         - The user may ask you to fix code, run builds, check tests, etc. — use your tools.\n\
         - Keep responses concise and IM-friendly. Use short summaries instead of full file dumps.\n\
         - When you perform file edits or run commands, report a brief summary of what changed.\n\
         - For compile/test results, report pass/fail status and key metrics.\n\n\
         Media Capabilities:\n\
         - You can SEND images and files to the user via the notify_channel tool.\n\
         - To send a file/image: first save it to disk, then call notify_channel with \
           channel_id=\"{channel_id}\", target_id=\"{chat_id}\", and attachments=[{{\"file_path\": \"/absolute/path/to/file\"}}].\n\
         - The user may also send you images or files. When they do, the images will appear inline \
           in the message content. Describe what you see or process the file as requested.\n\
         - Supported media: images (PNG, JPEG, GIF), documents, and other files."
    );
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(prompt)),
        ..Default::default()
        },
    );
}

// ---------------------------------------------------------------------------
// Channel Segment types — ordered streaming output
// ---------------------------------------------------------------------------

/// A segment of channel streaming output, preserving the time-ordered sequence
/// of text, thinking, and tool calls as they arrive from the agentic loop.
#[derive(Debug, Clone)]
enum ChannelSegment {
    Text(String),
    Thinking(String),
    ToolCall {
        tool_name: String,
        call_id: String,
        args: Option<String>,
        result: Option<String>,
        success: Option<bool>,
        duration_ms: Option<u64>,
        is_interactive: bool,
        question_text: Option<String>,
        user_answer: Option<String>,
    },
}

/// Controls how segments are rendered into lark_md / markdown.
#[derive(Debug, Clone)]
struct ChannelStreamFormat {
    show_thinking: bool,
    thinking_max_chars: usize,
}

impl Default for ChannelStreamFormat {
    fn default() -> Self {
        Self {
            show_thinking: true,
            thinking_max_chars: 200,
        }
    }
}

/// Render an ordered list of segments into a lark_md / markdown string.
///
/// When `streaming` is true, the output includes cursor indicators and
/// "executing..." placeholders for in-flight tool calls.
fn render_segments(
    segments: &[ChannelSegment],
    streaming: bool,
    fmt: &ChannelStreamFormat,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        match seg {
            ChannelSegment::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                if streaming && is_last {
                    parts.push(format!("{text}\u{258d}"));
                } else {
                    parts.push(text.clone());
                }
            }
            ChannelSegment::Thinking(text) => {
                if !fmt.show_thinking || text.is_empty() {
                    continue;
                }
                let display = if streaming {
                    text.clone()
                } else if text.len() > fmt.thinking_max_chars {
                    let truncated: String = text.chars().take(fmt.thinking_max_chars).collect();
                    format!("{truncated}... (思考过程共 {} 字)", text.chars().count())
                } else {
                    text.clone()
                };
                let quoted = display
                    .lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(format!("💭 **思考中**\n{quoted}"));
            }
            ChannelSegment::ToolCall {
                tool_name,
                args,
                result,
                success,
                duration_ms,
                is_interactive,
                question_text,
                user_answer,
                ..
            } => {
                if *is_interactive {
                    parts.push(render_interactive_segment(
                        tool_name,
                        question_text.as_deref(),
                        user_answer.as_deref(),
                        *duration_ms,
                        streaming && result.is_none(),
                    ));
                } else {
                    parts.push(render_tool_segment(
                        tool_name,
                        args.as_deref(),
                        result.as_deref(),
                        *success,
                        *duration_ms,
                        streaming && result.is_none(),
                        fmt,
                    ));
                }
            }
        }
    }

    parts.join("\n\n")
}

fn friendly_tool_name(tool_name: &str) -> &'static str {
    match tool_name {
        "web_search" => "网络搜索",
        "web_fetch" => "网页获取",
        "http_fetch" => "HTTP 请求",
        "read_file" => "读取文件",
        "write_file" => "写入文件",
        "edit_file" => "编辑文件",
        "multi_edit" => "批量编辑",
        "apply_patch" => "应用补丁",
        "search_in_files" => "文件搜索",
        "list_directory" => "列出目录",
        "glob" => "文件匹配",
        "shell_exec" => "执行命令",
        "git" => "Git 操作",
        "browser" => "浏览器",
        "screenshot" => "截图",
        "memory_search" => "记忆搜索",
        "memory_store" | "memory" => "记忆存储",
        "image_generate" => "生成图片",
        "text_to_speech" => "语音合成",
        "calculator" => "计算器",
        "get_current_time" => "获取时间",
        "todo_write" | "todo_read" => "待办事项",
        "task_create" | "task_list" | "task_get" | "background_task_stop" | "task_update" => "任务管理",
        "task_stop" => "编排协调",
        "confirm" => "确认操作",
        "workspace_symbols" => "符号搜索",
        "go_to_definition" => "跳转定义",
        "find_references" => "查找引用",
        "file_outline" => "文件大纲",
        "code_sections" => "代码分段",
        "lsp" => "语言服务",
        "notebook_edit" => "笔记本编辑",
        "snip" => "代码片段",
        "workflow" => "工作流",
        "skill" | "list_skills" | "read_skill" | "search_skills" => "技能",
        "tool_search" => "工具搜索",
        "terminal_capture" => "终端截取",
        "identity" | "get_identity" | "set_identity" => "身份设置",
        "enter_plan_mode" | "exit_plan_mode" => "规划模式",
        "send_user_message" => "消息发送",
        _ => "",
    }
}

/// JSON argument keys for [`tool_args_summary`]. Update when adding tools with new param names.
const TOOL_ARG_KEY_QUERY: &str = "query";
const TOOL_ARG_KEY_URL: &str = "url";
const TOOL_ARG_KEY_PATH: &str = "path";
const TOOL_ARG_KEY_PATTERN: &str = "pattern";
const TOOL_ARG_KEY_GLOB: &str = "glob";
const TOOL_ARG_KEY_COMMAND: &str = "command";
const TOOL_ARG_KEY_ACTION: &str = "action";

/// Extract a concise argument summary from tool args JSON for display.
fn tool_args_summary(tool_name: &str, args: Option<&str>) -> String {
    let args_str = match args {
        Some(a) if !a.is_empty() => a,
        _ => return String::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(args_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let summary = match tool_name {
        "web_search" => parsed
            .get(TOOL_ARG_KEY_QUERY)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "web_fetch" | "http_fetch" => parsed
            .get(TOOL_ARG_KEY_URL)
            .and_then(|v| v.as_str())
            .map(|u| {
                if u.len() > 50 {
                    let end = u.floor_char_boundary(47);
                    format!("{}...", &u[..end])
                } else {
                    u.to_string()
                }
            }),
        "read_file" => parsed
            .get(TOOL_ARG_KEY_PATH)
            .and_then(|v| v.as_str())
            .map(short_path),
        "write_file" | "edit_file" | "multi_edit" | "apply_patch" => parsed
            .get(TOOL_ARG_KEY_PATH)
            .and_then(|v| v.as_str())
            .map(short_path),
        "search_in_files" | "glob" => parsed
            .get(TOOL_ARG_KEY_PATTERN)
            .or_else(|| parsed.get(TOOL_ARG_KEY_QUERY))
            .or_else(|| parsed.get(TOOL_ARG_KEY_GLOB))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "list_directory" => parsed
            .get(TOOL_ARG_KEY_PATH)
            .and_then(|v| v.as_str())
            .map(short_path),
        "shell_exec" => parsed
            .get(TOOL_ARG_KEY_COMMAND)
            .and_then(|v| v.as_str())
            .map(|c| {
                if c.len() > 60 {
                    let end = c.floor_char_boundary(57);
                    format!("{}...", &c[..end])
                } else {
                    c.to_string()
                }
            }),
        "git" => parsed
            .get(TOOL_ARG_KEY_COMMAND)
            .or_else(|| parsed.get(TOOL_ARG_KEY_ACTION))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "memory_search" => parsed
            .get(TOOL_ARG_KEY_QUERY)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "browser" => parsed
            .get(TOOL_ARG_KEY_ACTION)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    };

    summary.unwrap_or_default()
}

fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() <= 3 {
        p.to_string()
    } else {
        format!(".../{}", parts[parts.len() - 2..].join("/"))
    }
}

fn render_tool_segment(
    tool_name: &str,
    args: Option<&str>,
    _result: Option<&str>,
    success: Option<bool>,
    duration_ms: Option<u64>,
    is_running: bool,
    _fmt: &ChannelStreamFormat,
) -> String {
    let friendly = friendly_tool_name(tool_name);
    let display_name = if friendly.is_empty() {
        tool_name.to_string()
    } else {
        friendly.to_string()
    };

    let args_hint = tool_args_summary(tool_name, args);

    if is_running {
        if args_hint.is_empty() {
            return format!("🔧 **{display_name}**...");
        } else {
            return format!("🔧 **{display_name}** {args_hint}...");
        }
    }

    let status_icon = match success {
        Some(true) => "✅",
        Some(false) => "❌",
        None => "✅",
    };

    let duration_str = duration_ms
        .map(|ms| {
            if ms >= 1000 {
                format!(" {:.1}s", ms as f64 / 1000.0)
            } else {
                format!(" {}ms", ms)
            }
        })
        .unwrap_or_default();

    if args_hint.is_empty() {
        format!("{status_icon} **{display_name}**{duration_str}")
    } else {
        format!("{status_icon} **{display_name}** {args_hint}{duration_str}")
    }
}

fn render_interactive_segment(
    _tool_name: &str,
    question_text: Option<&str>,
    user_answer: Option<&str>,
    duration_ms: Option<u64>,
    is_waiting: bool,
) -> String {
    let q = question_text.unwrap_or("等待确认");

    if is_waiting {
        return format!("❓ **等待确认**: {q}\n⏳ 请在下方卡片中选择...");
    }

    let duration_str = duration_ms
        .map(|ms| {
            if ms >= 1000 {
                format!(" ({:.1}s)", ms as f64 / 1000.0)
            } else {
                format!(" ({}ms)", ms)
            }
        })
        .unwrap_or_default();

    match user_answer {
        Some(ans) => format!("❓ **确认** → 用户选择: \"{ans}\"{duration_str}"),
        None => format!("❓ **确认**: {q} → (未回答){duration_str}"),
    }
}

/// Extract the plain text content from segments (for session history storage).
fn segments_plain_text(segments: &[ChannelSegment]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in segments {
        if let ChannelSegment::Text(t) = seg {
            if !t.is_empty() {
                parts.push(t.as_str());
            }
        }
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Streaming handler
// ---------------------------------------------------------------------------

/// Streaming handler for channels that support message editing (e.g. Feishu).
/// Supports ask_question: when the agent emits an AskQuestion event, an interactive
/// card is sent to the chat and the handler waits for the user's button click.
async fn handle_channel_streaming(
    state: &AppState,
    channel: &Arc<dyn xiaolin_core::channel::ChannelPlugin>,
    agent_config: &xiaolin_core::agent_config::AgentConfig,
    request: &xiaolin_core::types::ChatRequest,
    original_message_id: &str,
    chat_id: &str,
    llm_override: Option<Arc<dyn xiaolin_agent::LlmProvider>>,
) -> anyhow::Result<String> {
    use crate::ask_question_card::{AskQuestionCardBuilder, QuestionOption};
    use xiaolin_protocol::AgentEvent;

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
            .map(|c| c.into_owned())
            .unwrap_or_else(|| "(no response)".to_string());
        return Ok(text);
    }

    tracing::debug!(reply_msg_id = %reply_msg_id, "streaming: placeholder sent, starting LLM stream");

    let tool_definition_count = request.tools.as_ref().map_or(0, |t| t.len());
    let input_estimate = xiaolin_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &request.messages,
        tool_definition_count,
    );
    let model_for_budget = request
        .model
        .clone()
        .unwrap_or_else(|| agent_config.model.model.clone());

    let state_budget = state.clone();
    let stream_context_key = uuid::Uuid::new_v4().to_string();

    let mut op_extra = serde_json::Map::new();
    if let Ok(enriched_val) = serde_json::to_value(request) {
        op_extra.insert("_enriched_request".into(), enriched_val);
    }
    if let Ok(cfg_val) = serde_json::to_value(agent_config) {
        op_extra.insert("_agent_config".into(), cfg_val);
    }
    op_extra.insert(
        "_stream_context_key".into(),
        serde_json::Value::String(stream_context_key.clone()),
    );

    let session_id = request
        .session_id
        .clone()
        .unwrap_or_else(|| xiaolin_protocol::SessionId::new(uuid::Uuid::new_v4().to_string()));

    let session_handle = state
        .svc
        .session_manager
        .get_or_create(session_id.clone(), &agent_config.agent_id.to_string())
        .await;

    let (_sub_id, mut event_rx) = match session_handle
        .submit_and_subscribe(
            xiaolin_session_actor::SessionOp::UserTurn {
                messages: serde_json::to_value(&request.messages).unwrap_or_default(),
                agent_id: Some(agent_config.agent_id.to_string()),
                model: request.model.clone(),
                work_dir: request.work_dir.clone(),
                extra: op_extra,
                typed_data: None,
            },
            128,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!("session submit error: {e}"));
        }
    };

    let mut segments: Vec<ChannelSegment> = Vec::new();
    let stream_fmt = ChannelStreamFormat::default();
    let mut last_update = std::time::Instant::now();
    let update_interval = std::time::Duration::from_millis(800);
    let channel_for_update = channel.clone();
    let supports_cards = channel.supports_interactive_questions();

    let mut tool_start_times: std::collections::HashMap<String, std::time::Instant> =
        std::collections::HashMap::new();

    let mut segments_dirty = false;

    while let Some(se) = event_rx.recv().await {
        let event = se.msg;
        state.store.event_log.append(&session_id, &event);
        match event {
            AgentEvent::ContentDelta { delta, .. } => {
                if let Some(choices) = delta.get("choices").and_then(|c| c.as_array()) {
                    for choice in choices {
                        if let Some(reasoning) = choice
                            .get("delta")
                            .and_then(|d| d.get("reasoning_content"))
                            .and_then(|c| c.as_str())
                        {
                            if !reasoning.is_empty() {
                                match segments.last_mut() {
                                    Some(ChannelSegment::Thinking(ref mut t)) => {
                                        t.push_str(reasoning)
                                    }
                                    _ => segments.push(ChannelSegment::Thinking(reasoning.to_string())),
                                }
                                segments_dirty = true;
                            }
                        }
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !content.is_empty() {
                                match segments.last_mut() {
                                    Some(ChannelSegment::Text(ref mut t)) => t.push_str(content),
                                    _ => segments.push(ChannelSegment::Text(content.to_string())),
                                }
                                segments_dirty = true;
                            }
                        }
                    }
                }

                if segments_dirty && last_update.elapsed() >= update_interval {
                    let rendered = render_segments(&segments, true, &stream_fmt);
                    if !rendered.is_empty() {
                        if let Err(e) = channel_for_update
                            .update_message(&reply_msg_id, &rendered)
                            .await
                        {
                            tracing::debug!(error = %e, "streaming: update_message failed (will retry)");
                        }
                    }
                    last_update = std::time::Instant::now();
                    segments_dirty = false;
                }
            }
            AgentEvent::ToolExecuting {
                tool_name,
                call_id,
                args,
                ..
            } => {
                tool_start_times.insert(call_id.clone(), std::time::Instant::now());
                segments.push(ChannelSegment::ToolCall {
                    tool_name,
                    call_id,
                    args,
                    result: None,
                    success: None,
                    duration_ms: None,
                    is_interactive: false,
                    question_text: None,
                    user_answer: None,
                });
                let rendered = render_segments(&segments, true, &stream_fmt);
                let _ = channel_for_update
                    .update_message(&reply_msg_id, &rendered)
                    .await;
                last_update = std::time::Instant::now();
                segments_dirty = false;
            }
            AgentEvent::ToolResult {
                call_id,
                output,
                display_output,
                success,
                ..
            } => {
                let elapsed = tool_start_times
                    .remove(&call_id)
                    .map(|t| t.elapsed().as_millis() as u64);
                let display = display_output.unwrap_or(output);
                for seg in segments.iter_mut().rev() {
                    if let ChannelSegment::ToolCall {
                        call_id: ref cid,
                        result: ref mut r,
                        success: ref mut s,
                        duration_ms: ref mut d,
                        is_interactive,
                        user_answer: ref mut ua,
                        ..
                    } = seg
                    {
                        if cid == &call_id {
                            *r = Some(display.clone());
                            *s = Some(success);
                            *d = elapsed;
                            if *is_interactive {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(&display)
                                {
                                    if let Some(ans) = parsed.get("answer").and_then(|v| v.as_str())
                                    {
                                        *ua = Some(ans.to_string());
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                let rendered = render_segments(&segments, true, &stream_fmt);
                let _ = channel_for_update
                    .update_message(&reply_msg_id, &rendered)
                    .await;
                last_update = std::time::Instant::now();
                segments_dirty = false;
            }
            AgentEvent::ToolProgress {
                call_id, message, ..
            } => {
                for seg in segments.iter_mut().rev() {
                    if let ChannelSegment::ToolCall {
                        call_id: ref cid,
                        result: ref mut r,
                        ..
                    } = seg
                    {
                        if cid == &call_id && r.is_none() {
                            *r = Some(message.clone());
                            break;
                        }
                    }
                }
                if last_update.elapsed() >= update_interval {
                    let rendered = render_segments(&segments, true, &stream_fmt);
                    let _ = channel_for_update
                        .update_message(&reply_msg_id, &rendered)
                        .await;
                    last_update = std::time::Instant::now();
                }
            }
            AgentEvent::AskQuestion {
                request_id,
                question,
                options,
                timeout_secs,
                allow_multiple,
                ..
            } => {
                for seg in segments.iter_mut().rev() {
                    if let ChannelSegment::ToolCall {
                        tool_name: ref tn,
                        is_interactive: ref mut ii,
                        question_text: ref mut qt,
                        ..
                    } = seg
                    {
                        if tn == "ask_question" {
                            *ii = true;
                            *qt = Some(question.clone());
                            break;
                        }
                    }
                }

                let rendered = render_segments(&segments, true, &stream_fmt);
                let _ = channel_for_update
                    .update_message(&reply_msg_id, &rendered)
                    .await;
                last_update = std::time::Instant::now();
                segments_dirty = false;

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
                        session_id.to_string(),
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
                            let fallback_answer = options.first().map(|o| o.id.clone()).unwrap_or_default();
                            let _ = session_handle
                                .submit(xiaolin_session_actor::SessionOp::ResolveAnswer {
                                    interaction_id: request_id.clone(),
                                    answer: fallback_answer.clone(),
                                })
                                .await;
                        }
                    }
                } else {
                    tracing::debug!("streaming: channel does not support interactive cards, auto-selecting first option");
                    let fallback_answer = options.first().map(|o| o.id.clone()).unwrap_or_default();
                    let _ = session_handle
                        .submit(xiaolin_session_actor::SessionOp::ResolveAnswer {
                            interaction_id: request_id.clone(),
                            answer: fallback_answer.clone(),
                        })
                        .await;
                }
            }
            AgentEvent::TurnStart { session_id: Some(sid), .. } => {
                tracing::debug!(session_id = %sid, "channel streaming: turn started");
            }
            AgentEvent::TurnStart { .. } => {}
            AgentEvent::TurnEnd { .. } => {
                let plain = segments_plain_text(&segments);
                record_chat_budget_stream_estimate(
                    &state_budget,
                    model_for_budget.as_str(),
                    input_estimate,
                    plain.len(),
                );
                break;
            }
            AgentEvent::StreamError { message, retry_attempt, .. } => {
                if retry_attempt > 0 {
                    tracing::warn!(retry = retry_attempt, error = %message, "streaming: stream error (retrying)");
                } else {
                    tracing::error!(error = %message, "streaming: stream error");
                    segments.push(ChannelSegment::Text(format!("⚠ 流式错误: {message}")));
                    segments_dirty = true;
                }
            }
            AgentEvent::Warning { message, .. } => {
                tracing::info!(warning = %message, "streaming: warning");
                segments.push(ChannelSegment::Text(format!("⚠ {message}")));
                segments_dirty = true;
            }
            AgentEvent::ApprovalRequired {
                approval_id,
                action,
                reason,
                available_decisions,
                ..
            } => {
                tracing::info!(
                    approval_id = %approval_id,
                    reason = %reason,
                    "streaming: approval required — auto-denying for IM channel"
                );
                let action_type = serde_json::to_value(&action)
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
                    .unwrap_or_else(|| "action".to_string());
                segments.push(ChannelSegment::Text(format!(
                    "⚠ 需要审批 ({action_type}): {reason} — 在 IM 通道中自动拒绝"
                )));
                segments_dirty = true;
                let deny = available_decisions
                    .iter()
                    .find(|d| matches!(d, xiaolin_protocol::approval::ApprovalDecision::Denied))
                    .cloned()
                    .unwrap_or(xiaolin_protocol::approval::ApprovalDecision::Denied);
                let _ = session_handle
                    .submit(xiaolin_session_actor::SessionOp::ResolveApproval {
                        interaction_id: approval_id.clone(),
                        decision: deny.clone(),
                    })
                    .await;
            }
            AgentEvent::Error { message, .. } => {
                tracing::error!(error = %message, "streaming: LLM error");
                if segments.is_empty() {
                    segments.push(ChannelSegment::Text(format!("(错误: {message})")));
                }
                break;
            }
            _ => {}
        }
    }

    let final_rendered = render_segments(&segments, false, &stream_fmt);
    let final_text = if final_rendered.is_empty() {
        "(no response)".to_string()
    } else {
        final_rendered
    };

    if let Err(e) = channel.update_message(&reply_msg_id, &final_text).await {
        tracing::warn!(error = %e, "streaming: final update_message failed");
    }

    let plain_text = segments_plain_text(&segments);
    let reply = if plain_text.is_empty() {
        final_text.clone()
    } else {
        plain_text
    };

    tracing::info!(
        reply_msg_id = %reply_msg_id,
        segment_count = segments.len(),
        content_len = final_text.len(),
        "streaming: completed"
    );

    Ok(reply)
}

pub(super) async fn list_channels(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.ext.channel_registry.read().await;
    let mut channels: Vec<Value> = Vec::new();

    for ch in registry.all_plugins() {
        let meta = ch.meta();
        let caps = ch.capabilities();
        let mode = ch.connection_mode();
        let healthy = ch.probe().await.unwrap_or(false);

        channels.push(json!({
            "id": meta.id,
            "name": meta.name,
            "description": meta.description,
            "aliases": meta.aliases,
            "status": if healthy { "connected" } else { "disconnected" },
            "connectionMode": mode,
            "capabilities": {
                "directMessage": caps.direct_message,
                "groupChat": caps.group_chat,
                "media": caps.media,
                "streaming": caps.streaming,
                "reactions": caps.reactions,
                "threads": caps.threads,
            },
        }));
    }

    Json(json!({ "channels": channels, "count": channels.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_fmt() -> ChannelStreamFormat {
        ChannelStreamFormat::default()
    }

    fn text(s: &str) -> ChannelSegment {
        ChannelSegment::Text(s.to_string())
    }

    fn thinking(s: &str) -> ChannelSegment {
        ChannelSegment::Thinking(s.to_string())
    }

    fn tool(
        name: &str,
        result: Option<&str>,
        success: Option<bool>,
        ms: Option<u64>,
    ) -> ChannelSegment {
        tool_with_args(name, None, result, success, ms)
    }

    fn tool_with_args(
        name: &str,
        args: Option<&str>,
        result: Option<&str>,
        success: Option<bool>,
        ms: Option<u64>,
    ) -> ChannelSegment {
        ChannelSegment::ToolCall {
            tool_name: name.to_string(),
            call_id: "c1".to_string(),
            args: args.map(String::from),
            result: result.map(String::from),
            success,
            duration_ms: ms,
            is_interactive: false,
            question_text: None,
            user_answer: None,
        }
    }

    fn interactive(question: &str, answer: Option<&str>, ms: Option<u64>) -> ChannelSegment {
        ChannelSegment::ToolCall {
            tool_name: "ask_question".to_string(),
            call_id: "c2".to_string(),
            args: None,
            result: answer.map(|a| format!("{{\"answer\":\"{a}\"}}")),
            success: answer.map(|_| true),
            duration_ms: ms,
            is_interactive: true,
            question_text: Some(question.to_string()),
            user_answer: answer.map(String::from),
        }
    }

    #[test]
    fn render_text_only_streaming() {
        let segs = vec![text("Hello world")];
        let out = render_segments(&segs, true, &default_fmt());
        assert!(
            out.contains("Hello world\u{258d}"),
            "streaming cursor missing: {out}"
        );
    }

    #[test]
    fn render_text_only_final() {
        let segs = vec![text("Hello world")];
        let out = render_segments(&segs, false, &default_fmt());
        assert_eq!(out, "Hello world");
    }

    #[test]
    fn render_thinking_truncated_in_final() {
        let long = "a".repeat(400);
        let segs = vec![thinking(&long)];
        let fmt = default_fmt();
        let out = render_segments(&segs, false, &fmt);
        assert!(
            out.contains("思考过程共 400 字"),
            "truncation note missing: {out}"
        );
        assert!(out.contains("💭"), "thinking icon missing: {out}");
    }

    #[test]
    fn render_thinking_full_in_streaming() {
        let long = "a".repeat(400);
        let segs = vec![thinking(&long)];
        let out = render_segments(&segs, true, &default_fmt());
        assert!(
            !out.contains("思考过程共"),
            "should not truncate in streaming: {out}"
        );
    }

    #[test]
    fn render_tool_running() {
        let segs = vec![tool("read_file", None, None, None)];
        let out = render_segments(&segs, true, &default_fmt());
        assert_eq!(out, "🔧 **读取文件**...");
    }

    #[test]
    fn render_tool_running_with_args() {
        let segs = vec![tool_with_args(
            "web_search",
            Some(r#"{"query":"珠海天气"}"#),
            None,
            None,
            None,
        )];
        let out = render_segments(&segs, true, &default_fmt());
        assert!(
            out.contains("网络搜索") && out.contains("珠海天气"),
            "should show friendly name + query: {out}"
        );
    }

    #[test]
    fn render_tool_completed() {
        let segs = vec![tool(
            "read_file",
            Some("found 3 files"),
            Some(true),
            Some(350),
        )];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(out.contains("✅"), "success icon: {out}");
        assert!(out.contains("350ms"), "duration: {out}");
        assert!(out.contains("读取文件"), "friendly name: {out}");
    }

    #[test]
    fn render_tool_failed() {
        let segs = vec![tool(
            "shell_exec",
            Some("exit code 1"),
            Some(false),
            Some(1200),
        )];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(out.contains("❌"), "failure icon: {out}");
        assert!(out.contains("1.2s"), "duration: {out}");
        assert!(out.contains("执行命令"), "friendly name: {out}");
    }

    #[test]
    fn render_mixed_segments_ordered() {
        let segs = vec![
            text("Let me analyze the code..."),
            tool(
                "read_file",
                Some("src/main.rs (120 lines)"),
                Some(true),
                Some(200),
            ),
            text("I found two approaches."),
            tool(
                "shell_exec",
                Some("npm install OK"),
                Some(true),
                Some(1500),
            ),
            text("Done!"),
        ];
        let out = render_segments(&segs, false, &default_fmt());
        let lines: Vec<&str> = out.lines().collect();
        let analyze_pos = lines.iter().position(|l| l.contains("analyze")).unwrap();
        let read_pos = lines.iter().position(|l| l.contains("读取文件")).unwrap();
        let found_pos = lines
            .iter()
            .position(|l| l.contains("two approaches"))
            .unwrap();
        let shell_pos = lines.iter().position(|l| l.contains("执行命令")).unwrap();
        let done_pos = lines.iter().position(|l| l.contains("Done!")).unwrap();
        assert!(analyze_pos < read_pos, "text before tool");
        assert!(read_pos < found_pos, "tool before next text");
        assert!(found_pos < shell_pos, "second text before second tool");
        assert!(shell_pos < done_pos, "second tool before final text");
    }

    #[test]
    fn render_interactive_waiting() {
        let segs = vec![
            text("Found two options."),
            interactive("Which approach?", None, None),
        ];
        let out = render_segments(&segs, true, &default_fmt());
        assert!(
            out.contains("❓ **等待确认**: Which approach?"),
            "question: {out}"
        );
        assert!(out.contains("请在下方卡片中选择"), "wait prompt: {out}");
    }

    #[test]
    fn render_interactive_answered() {
        let segs = vec![
            text("Found two options."),
            interactive("Which approach?", Some("Option A"), Some(5200)),
        ];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(out.contains("❓ **确认**"), "confirmed: {out}");
        assert!(out.contains("Option A"), "answer: {out}");
        assert!(out.contains("5.2s"), "duration: {out}");
    }

    #[test]
    fn render_empty_segments() {
        let segs: Vec<ChannelSegment> = vec![];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(out.is_empty());
    }

    #[test]
    fn segments_plain_text_extracts_text_only() {
        let segs = vec![
            text("Hello"),
            thinking("deep thought"),
            tool("read_file", Some("ok"), Some(true), Some(100)),
            text("World"),
        ];
        let plain = segments_plain_text(&segs);
        assert_eq!(plain, "Hello\n\nWorld");
    }

    #[test]
    fn render_thinking_hidden_when_disabled() {
        let segs = vec![thinking("some thought"), text("answer")];
        let mut fmt = default_fmt();
        fmt.show_thinking = false;
        let out = render_segments(&segs, false, &fmt);
        assert!(!out.contains("思考"), "thinking should be hidden: {out}");
        assert!(out.contains("answer"));
    }

    #[test]
    fn render_tool_compact_no_result() {
        let segs = vec![tool(
            "search_in_files",
            Some("found 10 matches"),
            Some(true),
            Some(100),
        )];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(
            !out.contains("found 10 matches"),
            "result should not be shown in compact format: {out}"
        );
        assert!(out.contains("文件搜索"), "friendly name: {out}");
    }

    #[test]
    fn render_tool_with_args_summary() {
        let segs = vec![tool_with_args(
            "web_search",
            Some(r#"{"query":"rust async"}"#),
            Some("results..."),
            Some(true),
            Some(800),
        )];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(out.contains("网络搜索"), "friendly name: {out}");
        assert!(out.contains("rust async"), "args summary: {out}");
        assert!(out.contains("800ms"), "duration: {out}");
    }

    #[test]
    fn render_unknown_tool_uses_raw_name() {
        let segs = vec![tool(
            "custom_tool_xyz",
            Some("ok"),
            Some(true),
            Some(50),
        )];
        let out = render_segments(&segs, false, &default_fmt());
        assert!(
            out.contains("custom_tool_xyz"),
            "unknown tool should use raw name: {out}"
        );
    }

    #[test]
    fn render_thinking_then_tool_then_text() {
        let segs = vec![
            thinking("Let me think about this problem..."),
            tool("read_file", Some("found it"), Some(true), Some(200)),
            text("Here is the answer."),
        ];
        let out = render_segments(&segs, false, &default_fmt());
        let lines: Vec<&str> = out.lines().collect();
        let think_pos = lines.iter().position(|l| l.contains("💭")).unwrap();
        let tool_pos = lines.iter().position(|l| l.contains("读取文件")).unwrap();
        let answer_pos = lines.iter().position(|l| l.contains("answer")).unwrap();
        assert!(think_pos < tool_pos);
        assert!(tool_pos < answer_pos);
    }
}
