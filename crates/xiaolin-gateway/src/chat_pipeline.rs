//! Shared chat setup (resolve → session → context → routing → budget) and post-turn helpers
//! used by HTTP `/api/v1/chat` and the WebSocket `chat` method.

use std::sync::Arc;

use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::types::{ChatMessage, ChatRequest, Role};

use crate::error::AppError;
use crate::routes::{
    apply_model_router_for_chat, auto_record_episode, filtered_tool_definitions,
    map_router_resolve_err, record_chat_budget_actual, record_chat_budget_stream_estimate,
    resolve_session_context, try_reserve_budget, ResolvedSession,
};
use crate::state::AppState;

/// Options for [`setup_chat`], preserving small behavioral differences between transports.
pub struct SetupChatOptions {
    /// Second argument to [`xiaolin_observe::record_chat_request`].
    pub chat_stream: bool,
    /// When false, context ingest errors are ignored (WebSocket `chat` legacy behavior).
    pub propagate_context_ingest_errors: bool,
    /// When true, the resolved session id is written to the enriched request (streaming paths).
    pub set_resolved_session_on_request: bool,
    /// When true (HTTP paths), call [`xiaolin_observe::record_chat_request`] after resolve.
    /// WebSocket `chat` historically did not record this metric.
    pub record_chat_observe: bool,
}

impl Default for SetupChatOptions {
    fn default() -> Self {
        Self {
            chat_stream: false,
            propagate_context_ingest_errors: true,
            set_resolved_session_on_request: false,
            record_chat_observe: true,
        }
    }
}

/// Result of the chat pipeline setup phase.
#[derive(Clone)]
pub struct ChatSetup {
    pub agent_config: AgentConfig,
    pub agent_id: String,
    pub session_id: String,
    pub enriched_request: ChatRequest,
    pub resolve_reason: &'static str,
    pub llm_override: Option<Arc<dyn xiaolin_agent::LlmProvider>>,
    pub reserved_cost: f64,
    pub budget_degraded: bool,
    pub model_for_budget: String,
    pub input_estimate: u32,
    #[allow(dead_code)]
    pub tool_definition_count: usize,
    pub needs_title: bool,
    pub user_text_for_title: Option<String>,
    /// Original inbound user/turn messages (before context enrichment).
    pub user_messages: Vec<ChatMessage>,
    pub prompt_intent: Option<String>,
    pub prompt_profile: Option<String>,
    pub prompt_route_reason: Option<&'static str>,
    pub slash_intent_type: Option<String>,
    pub slash_intent_value: Option<String>,
    pub slash_exact_match: Option<bool>,
    pub slash_skill_loaded: Option<bool>,
    /// (estimated_context_tokens, effective_context_window). Window is 0 when unconfigured.
    pub context_tokens_estimate: Option<(u32, u32)>,
}

/// Run the setup phase: resolve agent, session, context, model routing, budget.
pub async fn setup_chat(
    state: &AppState,
    request: &ChatRequest,
    options: SetupChatOptions,
) -> Result<ChatSetup, AppError> {
    let setup_t0 = std::time::Instant::now();
    let user_messages = request.messages.clone();

    let t0 = std::time::Instant::now();
    if let Some(ref req_agent_id) = request.agent_id {
        if req_agent_id != "main" {
            tracing::warn!(
                requested_agent_id = %req_agent_id,
                "agentId parameter is deprecated; all requests now route to the main agent"
            );
        }
    }
    let agent_config = {
        let router = state.rt.router.read().await;
        router
            .agent_by_id("main")
            .or_else(|| router.list_agents().into_iter().next())
            .cloned()
            .ok_or_else(|| {
                map_router_resolve_err(anyhow::anyhow!("no main agent configured"))
            })?
    };
    let agent_id = agent_config.agent_id.clone();
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: resolve_agent (fixed main)"
    );

    let resolve_reason: &'static str = "main";

    if options.record_chat_observe {
        xiaolin_observe::record_chat_request(&agent_id, options.chat_stream);
    }

    let t0 = std::time::Instant::now();
    let ResolvedSession {
        session_id,
        messages: mut context_messages,
        needs_title,
    } = resolve_session_context(state, request.session_id.as_deref(), &agent_id).await?;
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: resolve_session_context"
    );
    let user_text_for_title = if needs_title {
        user_messages
            .iter()
            .find(|m| m.role == xiaolin_core::types::Role::User && m.content.is_some())
            .and_then(|m| m.text_content())
            .map(|c| c.into_owned())
    } else {
        None
    };

    if let Some(ref wd) = request.work_dir {
        let _ = state
            .store
            .session_store
            .update_work_dir(&session_id, Some(wd))
            .await;
        if let Ok(project) = state.store.session_store.find_or_create_project(wd).await {
            let _ = state
                .store
                .session_store
                .update_session_project_id(&session_id, Some(&project.id))
                .await;
            let _ = state.strm.ws_broadcast.send(
                serde_json::json!({"type":"event","event":"projects.changed","data":{"projectId": &project.id, "action": "session_bound"}}).to_string(),
            );
            let _ = state.strm.ws_broadcast.send(
                serde_json::json!({"type":"event","event":"sessions.changed","data":{"sessionId": &session_id}}).to_string(),
            );
        }
    } else {
        auto_detect_work_dir(state, &session_id, &user_messages).await;
    }

    for msg in &user_messages {
        if msg.role == Role::User {
            if let Some(text) = msg.text_content() {
                let result = state.rt.prompt_guard.is_suspicious(&text);
                if result.is_suspicious {
                    tracing::warn!(
                        agent_id = %agent_id,
                        risk = ?result.risk_level,
                        patterns = ?result.matched_patterns,
                        "prompt injection detected in user message"
                    );
                    if result.risk_level == xiaolin_security::RiskLevel::High {
                        return Err(AppError::BadRequest(format!(
                            "Message rejected: high-risk prompt injection patterns detected ({})",
                            result.matched_patterns.join(", ")
                        )));
                    }
                }
            }
        }
    }

    let ingest_input = xiaolin_context::IngestInput {
        messages: user_messages.clone(),
        agent_id: agent_id.to_string(),
        session_id: session_id.clone(),
        user_id: None,
    };
    let t0 = std::time::Instant::now();
    if options.propagate_context_ingest_errors {
        state
            .store
            .context_engine
            .ingest(&ingest_input, &mut context_messages)
            .await?;
    } else {
        let _ = state
            .store
            .context_engine
            .ingest(&ingest_input, &mut context_messages)
            .await;
    }
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: context_ingest"
    );

    context_messages.extend_from_slice(&user_messages);

    // Build enriched_request without cloning request.messages (which would be
    // immediately discarded). This avoids a large allocation for sessions with
    // extensive message histories.
    let mut enriched_request = ChatRequest {
        messages: context_messages,
        model: request.model.clone(),
        agent_id: request.agent_id.clone(),
        session_id: request.session_id.clone(),
        stream: request.stream,
        temperature: request.temperature,
        max_tokens: request.max_tokens,
        tools: request.tools.clone(),
        slash_intent: request.slash_intent.clone(),
        work_dir: request.work_dir.clone(),
    };
    let t0 = std::time::Instant::now();
    state
        .store
        .context_engine
        .process(&mut enriched_request.messages)
        .await?;
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: context_process"
    );

    // Detect /compact in user message text (frontend sends it via the slash command).
    let is_compact_request = user_messages
        .iter()
        .any(|m| m.role == Role::User && m.text_content().is_some_and(|t| t.trim() == "/compact"));
    if is_compact_request {
        // Replace the raw "/compact" user message with a nicer prompt and inject
        // a system marker that the agent loop's compression pipeline can detect.
        if let Some(last_user) = enriched_request.messages.iter_mut().rev().find(|m| {
            m.role == Role::User && m.text_content().is_some_and(|t| t.trim() == "/compact")
        }) {
            last_user.content = Some(serde_json::Value::String(
                "请压缩上下文并简要确认压缩结果。".to_string(),
            ));
        }
        enriched_request.messages.push(ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(
                "[COMPACT_REQUESTED] The user explicitly requested context compression. \
                 The compression pipeline will force-compress regardless of threshold."
                    .to_string(),
            )),
        ..Default::default()
        });
    }

    let t0 = std::time::Instant::now();
    let slash_meta =
        inject_slash_intent_context(state, request, &agent_id, &mut enriched_request.messages);
    let prompt_route_meta =
        apply_prompt_router(state, request, &agent_id, &mut enriched_request.messages);
    inject_skills_prompt(state, &agent_id, &mut enriched_request.messages);
    inject_runtime_paths_prompt(
        state,
        &agent_id,
        request.work_dir.as_deref(),
        &mut enriched_request.messages,
    );
    inject_mcp_tools_prompt(state, &mut enriched_request.messages);
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: prompt_injections"
    );

    // BUG-003: If the caller used an agent ID as the `model` field, clear it so the agent's
    // configured model is used instead of forwarding the alias string to the upstream LLM.
    {
        let router = state.rt.router.read().await;
        if enriched_request
            .model
            .as_deref()
            .map(|m| router.has_agent(m))
            .unwrap_or(false)
        {
            enriched_request.model = None;
        }
    }

    if options.set_resolved_session_on_request {
        enriched_request.session_id = Some(session_id.clone().into());
    }

    let t0 = std::time::Instant::now();
    let tool_definition_count =
        filtered_tool_definitions(&state.rt.tool_registry, &agent_config).map_or(0, |d| d.len());
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        count = tool_definition_count,
        "perf: filtered_tool_definitions"
    );
    let llm_override = apply_model_router_for_chat(
        state,
        &agent_config,
        &mut enriched_request,
        tool_definition_count,
    );

    let model_for_budget = enriched_request
        .model
        .clone()
        .unwrap_or_else(|| agent_config.model.model.clone());

    // Resolve effective context window: model config (live) > model router > agent fallback
    let effective_context_window: Option<u32> = {
        let model_ctx_from_config = {
            let live = state.cfg.config_live.load();
            live.get("models")
                .and_then(|m| m.as_object())
                .and_then(|models| {
                    models.values().find_map(|cfg| {
                        let m = cfg
                            .get("model")
                            .or_else(|| cfg.get("defaultModel"))
                            .and_then(|v| v.as_str())?;
                        if m == model_for_budget {
                            cfg.get("contextWindow")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32)
                        } else {
                            None
                        }
                    })
                })
        };
        model_ctx_from_config
            .or(agent_config.model.context_window)
            .or_else(|| {
                state
                    .obs
                    .model_router
                    .as_ref()
                    .and_then(|mr| mr.max_context_for_model(&model_for_budget))
            })
    };
    let context_tokens_estimate = if let Some(cw) = effective_context_window {
        let est = xiaolin_context::ContextEngine::fit_to_context_window(
            &mut enriched_request.messages,
            cw,
            agent_config.model.max_tokens,
        );
        Some((est as u32, cw))
    } else {
        let est = xiaolin_context::estimate_messages_tokens(&enriched_request.messages);
        Some((est as u32, 0))
    };

    let input_estimate = xiaolin_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &enriched_request.messages,
        tool_definition_count,
    );
    let (reserved_cost, budget_degraded) = try_reserve_budget(
        state,
        model_for_budget.as_str(),
        input_estimate,
        tool_definition_count,
    )?;

    {
        let msgs = &enriched_request.messages;
        let system_count = msgs.iter().filter(|m| m.role == Role::System).count();
        let user_count = msgs.iter().filter(|m| m.role == Role::User).count();
        let assistant_count = msgs.iter().filter(|m| m.role == Role::Assistant).count();
        let tool_msg_count = msgs.iter().filter(|m| m.role == Role::Tool).count();
        let system_tokens: usize = msgs
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| xiaolin_context::estimate_messages_tokens(std::slice::from_ref(m)))
            .sum();
        let history_tokens: usize = msgs
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| xiaolin_context::estimate_messages_tokens(std::slice::from_ref(m)))
            .sum();
        let est_tokens = context_tokens_estimate.map(|(t, _)| t).unwrap_or(0);
        tracing::info!(
            elapsed_ms = setup_t0.elapsed().as_millis() as u64,
            agent_id = %agent_id,
            tool_count = tool_definition_count,
            total_messages = msgs.len(),
            system_msgs = system_count,
            user_msgs = user_count,
            assistant_msgs = assistant_count,
            tool_msgs = tool_msg_count,
            system_tokens,
            history_tokens,
            est_total_tokens = est_tokens,
            "perf: setup_chat total"
        );
    }

    Ok(ChatSetup {
        agent_config,
        agent_id: agent_id.to_string(),
        session_id,
        enriched_request,
        resolve_reason,
        llm_override,
        reserved_cost,
        budget_degraded,
        model_for_budget,
        input_estimate,
        tool_definition_count,
        needs_title,
        user_text_for_title,
        user_messages,
        prompt_intent: prompt_route_meta.as_ref().map(|m| m.intent.clone()),
        prompt_profile: prompt_route_meta.as_ref().map(|m| m.profile.clone()),
        prompt_route_reason: prompt_route_meta.as_ref().map(|m| m.reason),
        slash_intent_type: slash_meta.as_ref().map(|m| m.intent_type.clone()),
        slash_intent_value: slash_meta.as_ref().map(|m| m.value.clone()),
        slash_exact_match: slash_meta.as_ref().map(|m| m.exact_match),
        slash_skill_loaded: slash_meta.as_ref().map(|m| m.skill_loaded),
        context_tokens_estimate,
    })
}

#[derive(Clone)]
struct SlashIntentMeta {
    intent_type: String,
    value: String,
    exact_match: bool,
    skill_loaded: bool,
}

async fn auto_detect_work_dir(
    state: &AppState,
    session_id: &str,
    user_messages: &[ChatMessage],
) {
    use std::path::Path;

    if let Ok(Some(session)) = state.store.session_store.get_session(session_id).await {
        if session.work_dir.is_some() {
            return;
        }
    }

    for msg in user_messages {
        if msg.role != Role::User {
            continue;
        }
        let Some(text) = msg.text_content() else {
            continue;
        };
        for path_str in extract_absolute_paths(&text) {
            let path = Path::new(path_str);
            if !path.exists() {
                let parent = path.parent();
                if parent.is_none_or(|p| !p.exists()) {
                    continue;
                }
            }
            let target = if path.is_dir() { path.to_path_buf() } else {
                path.parent().unwrap_or(path).to_path_buf()
            };
            let ws_root = xiaolin_core::workspace::detect_workspace_root(&target);
            let ws_str = ws_root.display().to_string();
            if state
                .store
                .session_store
                .update_work_dir(session_id, Some(&ws_str))
                .await
                .is_ok()
            {
                let _ = state.strm.ws_broadcast.send(
                    serde_json::json!({
                        "type": "event",
                        "event": "sessions.changed",
                        "data": { "sessionId": session_id }
                    })
                    .to_string(),
                );
                tracing::info!(
                    session_id = session_id,
                    detected_path = path_str,
                    workspace_root = %ws_str,
                    "auto-detected workspace from user message"
                );
            }
            return;
        }
    }
}

fn extract_absolute_paths(text: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let delimiters: &[char] = &[' ', '\t', '\n', '`', '"', '\'', '(', ')', '[', ']', '{', '}', '<', '>'];
    for token in text.split(delimiters) {
        let token = token.trim();
        if token.starts_with('/') && token.len() >= 4 && token.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic()) {
            paths.push(token);
        }
    }
    paths
}

fn inject_slash_intent_context(
    state: &AppState,
    request: &ChatRequest,
    agent_id: &str,
    messages: &mut Vec<ChatMessage>,
) -> Option<SlashIntentMeta> {
    let slash = request.slash_intent.as_ref()?;
    let intent_type = slash.intent_type.trim().to_lowercase();
    let value = slash.value.trim().to_string();
    let exact_match = slash.exact_match;
    let mut skill_loaded = false;

    if intent_type == "skill" && exact_match {
        let normalized = value.trim_start_matches('/').trim();
        if !normalized.is_empty() {
            let registry = state.skill_registry_for(agent_id);
            if let Some(skill) = registry.get(normalized) {
                let injected = format!(
                    "[Slash Skill Activation]\nskill_id: {}\nname: {}\n\n{}",
                    skill.id, skill.name, skill.content
                );
                messages.insert(
                    0,
                    ChatMessage {
                        role: Role::System,
                        content: Some(serde_json::Value::String(injected)),
                    ..Default::default()
                    },
                );
                skill_loaded = true;
            }
        }
    } else {
        let injected = format!(
            "[Slash Command Hint]\ntype: {}\nvalue: {}\nexact_match: {}",
            intent_type, value, exact_match
        );
        messages.insert(
            0,
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(injected)),
                ..Default::default()
            },
        );
    }

    Some(SlashIntentMeta {
        intent_type,
        value,
        exact_match,
        skill_loaded,
    })
}

#[derive(Clone)]
struct PromptRouteMeta {
    intent: String,
    profile: String,
    reason: &'static str,
}

fn apply_prompt_router(
    state: &AppState,
    request: &ChatRequest,
    agent_id: &str,
    messages: &mut Vec<ChatMessage>,
) -> Option<PromptRouteMeta> {
    let cfg = &state.cfg.config.prompt_router;
    if !cfg.enabled {
        return None;
    }

    let last_user_text = request
        .messages
        .iter()
        .rev()
        .filter(|m| m.role == Role::User)
        .find_map(|m| m.text_content())
        .unwrap_or_default();
    let haystack = last_user_text.to_lowercase();

    let mut selected_profile = cfg.default_profile.clone();
    let mut route_reason = "default-fallback";
    for rule in &cfg.rules {
        let hit = rule
            .keywords
            .iter()
            .map(|kw| kw.trim())
            .filter(|kw| !kw.is_empty())
            .any(|kw| haystack.contains(&kw.to_lowercase()));
        if hit {
            selected_profile = rule.profile.clone();
            route_reason = "rule-hit";
            break;
        }
    }

    let role_prompt_id = cfg
        .profiles
        .get(&selected_profile)
        .map(|p| p.role_prompt_id.as_str())
        .or_else(|| {
            cfg.profiles
                .get(&cfg.default_profile)
                .map(|p| p.role_prompt_id.as_str())
        })
        .unwrap_or("main");

    if role_prompt_id != agent_id {
        if let Some(role_prompt) =
            xiaolin_core::workspace::resolve_agent_role_prompt(role_prompt_id)
        {
            let dynamic = format!(
                "[Dynamic Role Prompt]\nintent: {selected_profile}\nrolePromptId: {role_prompt_id}\n\n{}",
                role_prompt.trim()
            );
            messages.insert(
                0,
                ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(dynamic)),
                ..Default::default()
                },
            );
        } else {
            route_reason = "missing-role-prompt-fallback";
        }
    }

    Some(PromptRouteMeta {
        intent: selected_profile.clone(),
        profile: selected_profile,
        reason: route_reason,
    })
}

fn inject_skills_prompt(state: &AppState, agent_id: &str, messages: &mut Vec<ChatMessage>) {
    let agent_skill_reg = state.skill_registry_for(agent_id);
    let skills_prompt =
        agent_skill_reg.format_for_prompt_mode(&state.cfg.config.skills.prompt_mode);
    if skills_prompt.is_empty() {
        return;
    }
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(skills_prompt)),
        ..Default::default()
        },
    );
}

fn inject_runtime_paths_prompt(
    state: &AppState,
    agent_id: &str,
    user_work_dir: Option<&str>,
    messages: &mut Vec<ChatMessage>,
) {
    let state_dir = xiaolin_core::paths::resolve_state_dir_from(Some(&state.cfg.config.paths));
    let agent_workspace = state
        .rt
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.clone())
        .unwrap_or_else(|| {
            xiaolin_core::workspace::resolve_workspace_root(&state_dir, agent_id, None)
        });
    let process_cwd = std::env::current_dir().unwrap_or_else(|_| state_dir.clone());

    let effective_workdir = user_work_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| agent_workspace.clone());

    let prompt = format!(
        "[Runtime Paths]\n\
XiaoLin state directory: {}\n\
Agent default workspace: {}\n\
Current working directory: {}\n\
Gateway process cwd: {}\n\
\n\
Guidance:\n\
- Use the \"Current working directory\" as your primary working directory for this conversation.\n\
- Use absolute paths rooted at the current working directory when calling read_file/write_file/list_directory/shell_exec.\n\
- The agent default workspace is a fallback if no specific directory was set.",
        state_dir.display(),
        agent_workspace.display(),
        effective_workdir.display(),
        process_cwd.display()
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

fn inject_mcp_tools_prompt(state: &AppState, messages: &mut Vec<ChatMessage>) {
    let mcp_tools = state.rt.tool_registry.mcp_definitions();

    tracing::debug!(
        mcp_tools = mcp_tools.len(),
        global_mcp_configured = state.cfg.config.mcp_servers.len(),
        "inject_mcp_tools_prompt check"
    );

    if mcp_tools.is_empty() {
        if !state.cfg.config.mcp_servers.is_empty() {
            tracing::warn!(
                configured = state.cfg.config.mcp_servers.len(),
                "MCP servers configured but no mcp_* tools in registry (connection may have failed at startup)"
            );
        }
        return;
    }

    let mut servers: std::collections::BTreeMap<String, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for td in &mcp_tools {
        let name = &td.function.name;
        let after_prefix = &name[4..]; // strip "mcp_"
        let server_id = if let Some(idx) = after_prefix.find('_') {
            &after_prefix[..idx]
        } else {
            after_prefix
        };
        servers
            .entry(server_id.to_string())
            .or_default()
            .push((name.as_str(), td.function.description.as_str()));
    }

    let mut prompt = String::from("[MCP Extensions]\nThe following MCP (Model Context Protocol) servers are connected, providing additional tools.\n\n");

    for (server_id, tools) in &servers {
        let cfg_match = state
            .cfg
            .config
            .mcp_servers
            .iter()
            .find(|c| c.id == *server_id);
        let cmd_info = cfg_match
            .map(|c| format!("{} {}", c.command, c.args.join(" ")))
            .unwrap_or_default();

        prompt.push_str(&format!("### MCP Server: {server_id}"));
        if !cmd_info.is_empty() {
            prompt.push_str(&format!(" ({cmd_info})"));
        }
        prompt.push('\n');
        prompt.push_str(&format!("Tools ({} available):\n", tools.len()));
        for (tool_name, desc) in tools {
            if desc.is_empty() {
                prompt.push_str(&format!("- `{tool_name}`\n"));
            } else {
                let short_desc: String = desc.chars().take(120).collect();
                prompt.push_str(&format!("- `{tool_name}`: {short_desc}\n"));
            }
        }
        prompt.push('\n');
    }

    prompt.push_str(
        "Use these MCP tools just like built-in tools — call them by their full prefixed name (e.g. `mcp_serverId_toolName`). \
         MCP tools extend your capabilities with external integrations."
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

/// Append the assistant message and record an episode when textual content is present.
/// When `with_smart_title` is true, schedules smart title generation (HTTP SSE `done` path).
pub async fn after_chat(
    state: &AppState,
    setup: &ChatSetup,
    assistant: &ChatMessage,
    with_smart_title: bool,
) -> Result<(), AppError> {
    state
        .store
        .session_store
        .append_message(&setup.session_id, assistant)
        .await?;

    // Dual-write: persist as HistoryItems alongside legacy messages
    {
        let turn_id = xiaolin_protocol::TurnId::generate();
        let history_items =
            xiaolin_core::history_compat::chat_message_to_history(assistant, turn_id);
        if let Err(e) = state
            .store
            .session_store
            .append_history_items(&setup.session_id, &history_items)
            .await
        {
            tracing::warn!(session_id = %setup.session_id, error = %e, "failed to dual-write history items");
        }
    }

    if let Some(content) = assistant.text_content() {
        if !content.is_empty() {
            auto_record_episode(state, &setup.session_id, &setup.agent_id, &content).await;
        }
    }

    if with_smart_title {
        maybe_spawn_smart_title_background(
            state,
            setup,
            &assistant.text_content().unwrap_or_default(),
        );
    }

    if state.cfg.config.tracing.conversation_trace {
        spawn_trace_write(state, setup, assistant);
    }

    Ok(())
}

fn spawn_trace_write(state: &AppState, setup: &ChatSetup, assistant: &ChatMessage) {
    use xiaolin_core::types::{ConversationTrace, TraceLlmRequest, TraceLlmResponse, TraceTurn};

    let store = state.store.session_store.clone();
    let session_id = setup.session_id.clone();
    let agent_id = setup.agent_id.clone();
    let model = setup.model_for_budget.clone();
    let (ctx_tokens, ctx_window) = setup.context_tokens_estimate.unwrap_or((0, 0));

    let user_msg = setup.user_messages.last().cloned().unwrap_or(ChatMessage {
        role: Role::User,
        content: None,
    ..Default::default()
    });
    let assistant_msg = assistant.clone();

    tokio::spawn(async move {
        let now = chrono::Utc::now().to_rfc3339();
        let trace_id = format!("tr-{}", uuid::Uuid::new_v4());
        let turn = TraceTurn {
            turn_index: 0,
            user_message: user_msg,
            assistant_message: assistant_msg,
            tool_calls: Vec::new(),
            llm_request: TraceLlmRequest {
                model: model.clone(),
                message_count: 1,
                estimated_tokens: ctx_tokens,
            },
            llm_response: TraceLlmResponse {
                model: model.clone(),
                usage: None,
                finish_reason: Some("stop".to_string()),
                latency_ms: 0,
            },
            context_tokens: ctx_tokens,
            latency_ms: 0,
            compaction_applied: false,
        };

        let trace = ConversationTrace {
            trace_id,
            session_id,
            agent_id,
            model,
            context_window: if ctx_window > 0 {
                Some(ctx_window)
            } else {
                None
            },
            started_at: now.clone(),
            finished_at: Some(now),
            turns: vec![turn],
            metadata: serde_json::Map::new(),
        };

        if let Err(e) = store.upsert_trace(&trace).await {
            tracing::warn!(error = %e, "failed to write conversation trace");
        }
    });
}

/// Schedule smart session title generation when [`ChatSetup::needs_title`] is set and the
/// assistant produced non-empty text. Used by non-stream HTTP (after `after_turn`) and WS chat.
pub fn maybe_spawn_smart_title_background(
    state: &AppState,
    setup: &ChatSetup,
    assistant_text: &str,
) {
    if !setup.needs_title {
        return;
    }
    let state2 = state.clone();
    let sid2 = setup.session_id.clone();
    let model2 = setup.agent_config.model.model.clone();
    let ut = setup.user_text_for_title.clone().unwrap_or_default();
    let at = assistant_text.to_string();
    tokio::spawn(async move {
        if let Err(e) = generate_smart_title(&state2, &sid2, &model2, &ut, &at).await {
            tracing::warn!(error = %e, "failed to generate smart title");
        }
    });
}

/// Use the LLM to generate a concise session title from the conversation.
/// Falls back to a truncated user message if the LLM call fails.
pub async fn generate_smart_title(
    state: &AppState,
    session_id: &str,
    model: &str,
    user_text: &str,
    assistant_text: &str,
) -> anyhow::Result<()> {
    use xiaolin_core::types::Role;

    let user_preview: String = user_text.chars().take(300).collect();
    let assistant_preview: String = assistant_text.chars().take(300).collect();

    let messages = vec![
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(
                "Generate a concise title (max 30 chars) summarizing this conversation. \
                 Reply with ONLY the title, no quotes, no explanation."
                    .to_string(),
            )),
        ..Default::default()
        },
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(format!(
                "User: {user_preview}\nAssistant: {assistant_preview}"
            ))),
        ..Default::default()
        },
    ];

    let params = xiaolin_agent::CompletionParams {
        model,
        messages: &messages,
        temperature: 0.3,
        max_tokens: Some(40),
        tools: None,
    };

    let title = match state.rt.runtime.provider().chat_completion(&params).await {
        Ok(resp) => {
            record_chat_budget_actual(state, model, resp.usage.as_ref());
            if resp.usage.is_none() {
                record_chat_budget_stream_estimate(state, model, 100, 80);
            }
            resp.choices
                .first()
                .and_then(|c| c.message.text_content())
                .map(|t| t.trim().trim_matches('"').trim().to_string())
                .filter(|t| !t.is_empty())
        }
        Err(e) => {
            eprintln!("[title] LLM title generation failed: {e}, using fallback");
            None
        }
    };

    let final_title = match title {
        Some(t) => t.chars().take(50).collect::<String>(),
        None => {
            let cleaned = user_text.lines().next().unwrap_or(user_text).trim();
            if cleaned.chars().count() <= 50 {
                cleaned.to_string()
            } else {
                let truncated: String = cleaned.chars().take(47).collect();
                format!("{truncated}...")
            }
        }
    };

    if !final_title.is_empty() {
        state
            .store
            .session_store
            .update_title(session_id, &final_title)
            .await?;
    }
    Ok(())
}
