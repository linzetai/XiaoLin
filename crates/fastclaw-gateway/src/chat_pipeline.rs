//! Shared chat setup (resolve → session → context → routing → budget) and post-turn helpers
//! used by HTTP `/api/v1/chat` and the WebSocket `chat` method.

use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

use crate::error::AppError;
use crate::routes::{
    apply_model_router_for_chat, auto_record_episode, filtered_tool_definitions,
    map_router_resolve_err, record_chat_budget_actual, record_chat_budget_stream_estimate,
    resolve_session_context, try_reserve_budget,
};
use crate::state::AppState;

/// Options for [`setup_chat`], preserving small behavioral differences between transports.
pub struct SetupChatOptions {
    /// Second argument to [`fastclaw_observe::record_chat_request`].
    pub chat_stream: bool,
    /// When false, context ingest errors are ignored (WebSocket `chat` legacy behavior).
    pub propagate_context_ingest_errors: bool,
    /// When true, the resolved session id is written to the enriched request (streaming paths).
    pub set_resolved_session_on_request: bool,
    /// When true (HTTP paths), call [`fastclaw_observe::record_chat_request`] after resolve.
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
    pub llm_override: Option<Arc<dyn fastclaw_agent::LlmProvider>>,
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
    let user_messages = request.messages.clone();

    let agent_config = {
        let router = state.router.read().await;
        router
            .resolve(request)
            .map(|c| c.clone())
            .map_err(map_router_resolve_err)?
    };
    let agent_id = agent_config.agent_id.clone();
    let resolve_reason: &'static str = if request.agent_id.is_some() {
        "explicit"
    } else {
        "default"
    };

    if options.record_chat_observe {
        fastclaw_observe::record_chat_request(&agent_id, options.chat_stream);
    }

    let (session_id, mut context_messages) =
        resolve_session_context(state, request.session_id.as_deref(), &agent_id).await?;

    let needs_title = matches!(
        state.session_store.get_session(&session_id).await,
        Ok(Some(s)) if s.title.is_none()
    );
    let user_text_for_title = if needs_title {
        user_messages
            .iter()
            .find(|m| m.role == fastclaw_core::types::Role::User && m.content.is_some())
            .and_then(|m| m.text_content())
    } else {
        None
    };

    let ingest_input = fastclaw_context::IngestInput {
        messages: user_messages.clone(),
        agent_id: agent_id.clone(),
        session_id: session_id.clone(),
        user_id: None,
    };
    if options.propagate_context_ingest_errors {
        state
            .context_engine
            .ingest(&ingest_input, &mut context_messages)
            .await?;
    } else {
        let _ = state
            .context_engine
            .ingest(&ingest_input, &mut context_messages)
            .await;
    }

    context_messages.extend_from_slice(&user_messages);

    let mut enriched_request = request.clone();
    enriched_request.messages = context_messages;
    state
        .context_engine
        .process(&mut enriched_request.messages)
        .await?;
    let slash_meta = inject_slash_intent_context(state, request, &agent_id, &mut enriched_request.messages);
    let prompt_route_meta = apply_prompt_router(state, request, &agent_id, &mut enriched_request.messages);
    inject_skills_prompt(state, &agent_id, &mut enriched_request.messages);
    inject_runtime_paths_prompt(state, &agent_id, request.work_dir.as_deref(), &mut enriched_request.messages);
    inject_mcp_tools_prompt(state, &mut enriched_request.messages);

    // BUG-003: If the caller used an agent ID as the `model` field, clear it so the agent's
    // configured model is used instead of forwarding the alias string to the upstream LLM.
    {
        let router = state.router.read().await;
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
        enriched_request.session_id = Some(session_id.clone());
    }

    let tool_definition_count =
        filtered_tool_definitions(&state.tool_registry, &agent_config).map_or(0, |d| d.len());
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
        let model_ctx_from_config = state
            .config_live
            .read()
            .ok()
            .and_then(|live| {
                live.get("models")
                    .and_then(|m| m.as_object())
                    .and_then(|models| {
                        models.values().find_map(|cfg| {
                            let m = cfg.get("model").and_then(|v| v.as_str())?;
                            if m == model_for_budget {
                                cfg.get("contextWindow").and_then(|v| v.as_u64()).map(|v| v as u32)
                            } else {
                                None
                            }
                        })
                    })
            });
        model_ctx_from_config
            .or(agent_config.model.context_window)
            .or_else(|| {
                state
                    .model_router
                    .as_ref()
                    .and_then(|mr| mr.max_context_for_model(&model_for_budget))
            })
    };
    let context_tokens_estimate = if let Some(cw) = effective_context_window {
        let est = fastclaw_context::ContextEngine::fit_to_context_window(
            &mut enriched_request.messages,
            cw,
            agent_config.model.max_tokens,
        );
        Some((est as u32, cw))
    } else {
        let est = fastclaw_context::estimate_messages_tokens(&enriched_request.messages);
        Some((est as u32, 0))
    };

    let input_estimate = fastclaw_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &enriched_request.messages,
        tool_definition_count,
    );
    let (reserved_cost, budget_degraded) = try_reserve_budget(
        state,
        model_for_budget.as_str(),
        input_estimate,
        tool_definition_count,
    )?;

    Ok(ChatSetup {
        agent_config,
        agent_id,
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
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
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
                name: None,
                tool_calls: None,
                tool_call_id: None,
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
    let cfg = &state.config.prompt_router;
    if !cfg.enabled {
        return None;
    }

    if request.agent_id.is_some() {
        return Some(PromptRouteMeta {
            intent: "explicit-agent".to_string(),
            profile: agent_id.to_string(),
            reason: "explicit-agent",
        });
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
        .or_else(|| cfg.profiles.get(&cfg.default_profile).map(|p| p.role_prompt_id.as_str()))
        .unwrap_or("main");

    if role_prompt_id != agent_id {
        if let Some(role_prompt) = fastclaw_core::workspace::resolve_agent_role_prompt(role_prompt_id) {
            let dynamic = format!(
                "[Dynamic Role Prompt]\nintent: {selected_profile}\nrolePromptId: {role_prompt_id}\n\n{}",
                role_prompt.trim()
            );
            messages.insert(
                0,
                ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(dynamic)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
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
    let skills_prompt = agent_skill_reg.format_for_prompt_mode(&state.config.skills.prompt_mode);
    if skills_prompt.is_empty() {
        return;
    }
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(skills_prompt)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    );
}

fn inject_runtime_paths_prompt(state: &AppState, agent_id: &str, user_work_dir: Option<&str>, messages: &mut Vec<ChatMessage>) {
    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&state.config.paths));
    let agent_workspace = state
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.clone())
        .unwrap_or_else(|| fastclaw_core::workspace::resolve_workspace_root(&state_dir, agent_id, None));
    let process_cwd = std::env::current_dir().unwrap_or_else(|_| state_dir.clone());

    let effective_workdir = user_work_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| agent_workspace.clone());

    let prompt = format!(
        "[Runtime Paths]\n\
FastClaw state directory: {}\n\
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
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    );
}

fn inject_mcp_tools_prompt(state: &AppState, messages: &mut Vec<ChatMessage>) {
    let tool_defs = state.tool_registry.definitions();
    let mcp_tools: Vec<_> = tool_defs
        .iter()
        .filter(|td| td.function.name.starts_with("mcp_"))
        .collect();

    tracing::debug!(
        total_tools = tool_defs.len(),
        mcp_tools = mcp_tools.len(),
        global_mcp_configured = state.config.mcp_servers.len(),
        "inject_mcp_tools_prompt check"
    );

    if mcp_tools.is_empty() {
        if !state.config.mcp_servers.is_empty() {
            tracing::warn!(
                configured = state.config.mcp_servers.len(),
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
        let cfg_match = state.config.mcp_servers.iter().find(|c| c.id == *server_id);
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
            name: None,
            tool_calls: None,
            tool_call_id: None,
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
        .session_store
        .append_message(&setup.session_id, assistant)
        .await?;

    if let Some(content) = assistant.text_content() {
        if !content.is_empty() {
            auto_record_episode(state, &setup.session_id, &setup.agent_id, &content).await;
        }
    }

    if with_smart_title {
        maybe_spawn_smart_title_background(
            state,
            setup,
            assistant.text_content().unwrap_or_default().as_str(),
        );
    }

    Ok(())
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
    use fastclaw_core::types::Role;

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
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(format!(
                "User: {user_preview}\nAssistant: {assistant_preview}"
            ))),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let params = fastclaw_agent::CompletionParams {
        model,
        messages: &messages,
        temperature: 0.3,
        max_tokens: Some(40),
        tools: None,
    };

    let title = match state.runtime.provider().chat_completion(&params).await {
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
            .session_store
            .update_title(session_id, &final_title)
            .await?;
    }
    Ok(())
}
