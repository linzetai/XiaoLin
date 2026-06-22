use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use xiaolin_agent::AgentRuntime;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::bus::MessageBus;
use xiaolin_core::channel::ChannelRegistry;
use xiaolin_core::config::XiaoLinConfig;
use xiaolin_core::skill::SkillRegistry;
use xiaolin_core::tool::{ContributorContext, ContributorRegistry, Tool, ToolRegistry};
use xiaolin_core::workspace::AgentWorkspace;
use xiaolin_core::Router as AgentRouter;
use xiaolin_cron::CronJobStore;
use xiaolin_evolution::{FeedbackStore, PromptDistiller, SkillStore, TrajectoryStore};
use xiaolin_memory::{DreamingPipeline, EmbeddingProvider, EpisodicMemory, SemanticMemory};
use xiaolin_model_router::BudgetTracker;
use xiaolin_session::{EventLog, SearchIndex, SessionStore};

use crate::memory_scope::memory_tool_agent_suffix;
use crate::scoped_tool::RenamedTool;

use super::helpers;
use super::AppState;

// --- Phased initialization for [`AppState::new`] (see [`StateBuilder`]). ---

struct BuildPhase1 {
    agents: Vec<AgentConfig>,
    agent_count: usize,
    db_path: PathBuf,
    pool: sqlx::SqlitePool,
    session_store: Arc<SessionStore>,
    event_log: Arc<EventLog>,
    search_index: Arc<SearchIndex>,
}

struct BuildPhase3 {
    phase1: BuildPhase1,
    runtime: Arc<AgentRuntime>,
    router: AgentRouter,
    tool_registry: Arc<ToolRegistry>,
    base_skill_registry: SkillRegistry,
    agent_skill_registries: std::collections::HashMap<String, Arc<SkillRegistry>>,
    workspaces: std::collections::HashMap<String, AgentWorkspace>,
    llm_plugin_registry: xiaolin_agent::LlmPluginRegistry,
    todo_store: xiaolin_agent::builtin_tools::TodoStore,
}

struct BuildPhase4 {
    phase3: BuildPhase3,
    channel_registry: ChannelRegistry,
    channel_inbound_tx: tokio::sync::mpsc::UnboundedSender<xiaolin_core::channel::InboundMessage>,
    inbound_rx: tokio::sync::mpsc::UnboundedReceiver<xiaolin_core::channel::InboundMessage>,
    base_skill_registry: Arc<SkillRegistry>,
    stream_event_tx:
        Arc<DashMap<String, tokio::sync::mpsc::Sender<xiaolin_protocol::AgentEvent>>>,
    tool_orchestrator: Arc<xiaolin_agent::ToolOrchestrator>,
    mcp_status_init: std::collections::HashMap<String, xiaolin_core::types::McpServerStatus>,
    mcp_handles_init: std::collections::HashMap<String, xiaolin_mcp::SharedMcpClient>,
    session_modes: xiaolin_agent::builtin_tools::SessionModeRegistry,
    goal_store: Arc<xiaolin_agent::builtin_tools::GoalStore>,
    plan_step_store: xiaolin_agent::builtin_tools::PlanStepStore,
}

struct BuildPhase2Memory {
    phase4: BuildPhase4,
    agent_episodic_map: std::collections::HashMap<String, Arc<EpisodicMemory>>,
    agent_semantic_map: std::collections::HashMap<String, Arc<SemanticMemory>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    message_bus: Arc<MessageBus>,
    feedback_store: FeedbackStore,
    prompt_distiller: PromptDistiller,
    trajectory_store: TrajectoryStore,
    skill_store: SkillStore,
    skill_embedding_store: xiaolin_core::skill_embedding::SkillEmbeddingStore,
    skill_usage_store: xiaolin_core::skill_usage::SkillUsageStore,
    context_engine: xiaolin_context::ContextEngine,
    tool_count: usize,
}

struct BuildPhase5 {
    phase2: BuildPhase2Memory,
    cron_store: CronJobStore,
    notification_store: crate::notification_store::NotificationStore,
    budget_tracker: BudgetTracker,
    model_router: Option<Arc<xiaolin_model_router::ModelRouter>>,
    ws_broadcast: tokio::sync::broadcast::Sender<String>,
}

/// Subsystem-grouped initialization phases for [`AppState`].
///
/// [`AppState::new`] chains [`StateBuilder::phase1_config_session`] → phase 3 → 4 → 2 → 5
/// (phase numbers follow dependency order; phase 2 memory/evolution runs after channels/MCP).
pub(crate) struct StateBuilder;

impl StateBuilder {
    /// Builtin default exec-policy embedded at compile time from `config/exec-policy.toml`.
    const BUILTIN_EXEC_POLICY: &'static str =
        include_str!("../../../../config/exec-policy.toml");

    /// Load execution policy from layered sources (project > user > builtin default).
    fn load_exec_policy(engine: &mut xiaolin_execpolicy::PolicyEngine, config: &XiaoLinConfig) -> bool {
        let mut loaded = false;

        // User-specified policy path from config
        if let Some(ref path_str) = config.security.exec_policy_path {
            let path = std::path::Path::new(path_str);
            if path.exists() {
                match engine.load_file(path, "user") {
                    Ok(()) => {
                        tracing::info!(path = %path.display(), "loaded user exec-policy");
                        loaded = true;
                    }
                    Err(e) => tracing::warn!(path = %path.display(), error = %e, "failed to load user exec-policy"),
                }
                return loaded;
            }
        }

        // Project-level: .xiaolin/exec-policy.toml (relative to git root)
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = cwd.as_path();
            loop {
                if dir.join(".git").exists() {
                    let project_policy = dir.join(".xiaolin").join("exec-policy.toml");
                    if project_policy.exists() {
                        match engine.load_file(&project_policy, "project") {
                            Ok(()) => {
                                tracing::info!(path = %project_policy.display(), "loaded project exec-policy");
                                loaded = true;
                            }
                            Err(e) => tracing::warn!(path = %project_policy.display(), error = %e, "failed to load project exec-policy"),
                        }
                    }
                    break;
                }
                match dir.parent() {
                    Some(parent) if parent != dir => dir = parent,
                    _ => break,
                }
            }
        }

        // User-level: ~/.xiaolin/exec-policy.toml
        if let Some(home) = dirs::home_dir() {
            let user_policy = home.join(".xiaolin").join("exec-policy.toml");
            if user_policy.exists() {
                let layer_name = if loaded { "user" } else { "system" };
                match engine.load_file(&user_policy, layer_name) {
                    Ok(()) => {
                        tracing::info!(path = %user_policy.display(), "loaded user-level exec-policy");
                        loaded = true;
                    }
                    Err(e) => tracing::warn!(path = %user_policy.display(), error = %e, "failed to load user-level exec-policy"),
                }
            }
        }

        // Builtin default: embedded at compile time (always available)
        if !loaded {
            match engine.load_str(Self::BUILTIN_EXEC_POLICY, "system") {
                Ok(()) => {
                    tracing::info!("loaded builtin default exec-policy (embedded)");
                    loaded = true;
                }
                Err(e) => tracing::warn!(error = %e, "failed to parse embedded exec-policy"),
            }
        }

        if loaded {
            tracing::info!(rules = engine.rule_count(), "exec-policy engine initialized");
        }
        loaded
    }

    /// Phase 1: config paths, agent list, unified SQLite pool, session store.
    async fn phase1_config_session(config: &XiaoLinConfig) -> anyhow::Result<BuildPhase1> {
        xiaolin_core::paths::ensure_state_dir_from(Some(&config.paths))?;
        let agents = helpers::load_agents(config)?;
        let agent_count = agents.len();
        let db_path = helpers::resolve_db_path(&config.paths)?;
        let pool = helpers::open_unified_pool(&db_path).await?;
        let search_index = Arc::new(SearchIndex::new(pool.clone()));
        search_index.ensure_schema().await?;
        let session_store = Arc::new(
            SessionStore::from_pool_with_search_index(pool.clone(), Some(search_index.clone()))
                .await?,
        );
        let event_log = Arc::new(EventLog::with_search_index(
            pool.clone(),
            Some(search_index.clone()),
        ));
        event_log.ensure_table().await?;
        Ok(BuildPhase1 {
            agents,
            agent_count,
            db_path,
            pool,
            session_store,
            event_log,
            search_index,
        })
    }

    /// Phase 3: LLM runtime, core tools, WASM/plugins, skills, workspaces.
    async fn phase3_agent_runtime_tools(
        config: &XiaoLinConfig,
        mut p1: BuildPhase1,
    ) -> anyhow::Result<BuildPhase3> {
        // Load LLM provider plugins from the plugins directory.
        let llm_plugins_dir =
            xiaolin_core::llm_plugin::resolve_plugins_dir(&config.llm_plugins, &config.paths);
        tracing::info!(
            enabled = config.llm_plugins.enabled,
            dir = %llm_plugins_dir.display(),
            "resolving LLM provider plugins"
        );
        let llm_plugins = if config.llm_plugins.enabled {
            let plugins = xiaolin_core::llm_plugin::load_llm_plugins(&llm_plugins_dir);
            tracing::info!(
                count = plugins.len(),
                dir = %llm_plugins_dir.display(),
                "loaded LLM provider plugins"
            );
            plugins
        } else {
            tracing::info!("LLM provider plugins disabled");
            Vec::new()
        };
        let llm_plugin_registry = xiaolin_agent::LlmPluginRegistry::from_configs(llm_plugins);

        let creds =
            helpers::merge_model_base_urls_into_credentials(&config.credentials, &config.models);
        let plugin_ref = if llm_plugin_registry.is_empty() {
            None
        } else {
            Some(&llm_plugin_registry)
        };
        xiaolin_agent::patch_agent_context_windows(&mut p1.agents, plugin_ref);
        let runtime = super::AppState::build_runtime(&p1.agents, &creds, plugin_ref)?;
        let router = AgentRouter::new(p1.agents.clone());
        let (tool_registry, todo_store) = super::AppState::build_tools_core(config).await?;

        let paths_cfg = &config.paths;

        use xiaolin_core::skill::{load_skills_from_dirs_with_layer, SkillLayer};

        let workspace_root = std::env::current_dir()
            .ok()
            .map(|cwd| xiaolin_core::workspace::detect_workspace_root(&cwd));

        let skills_dir = helpers::resolve_skills_dir(paths_cfg);

        let ext_registry = super::AppState::load_extension_skills(paths_cfg);
        let cross_tool_registry =
            xiaolin_core::skill::load_skills_cross_tool(workspace_root.as_deref());

        let legacy_project_registry = xiaolin_core::skill::load_skills_from_dirs_with_layer(
            &[skills_dir.as_path()],
            SkillLayer::Project,
        );

        let mut base_skill_registry = SkillRegistry::new();
        xiaolin_core::skill::register_builtin_skills(&mut base_skill_registry);
        base_skill_registry.merge_from(ext_registry);
        base_skill_registry.merge_from(legacy_project_registry);
        base_skill_registry.merge_from(cross_tool_registry);

        tracing::info!(
            base_skills = base_skill_registry.count(),
            skills_dir = %skills_dir.display(),
            workspace_root = ?workspace_root.as_deref().map(|p| p.display().to_string()),
            "base skill registry loaded (extension + legacy-project + cross-tool)"
        );

        // Sanitize skills deny list: remove entries for skills that no longer exist on disk.
        if !config.skills.deny.is_empty() {
            let (_, removed) = base_skill_registry.sanitize_deny_list(&config.skills.deny);
            if !removed.is_empty() {
                tracing::info!(
                    removed_count = removed.len(),
                    removed_ids = ?removed,
                    "cleaned stale entries from skills.deny (skills no longer on disk)"
                );
                let (cleaned, _) = base_skill_registry.sanitize_deny_list(&config.skills.deny);
                if let Err(e) = helpers::persist_skills_deny_cleanup(&cleaned) {
                    tracing::warn!(error = %e, "failed to persist cleaned skills.deny list to config file");
                }
            }
        }

        let resolved_agents = config.agents.resolved_list();
        let state_dir = helpers::resolve_state_dir(paths_cfg);
        let mut workspaces = std::collections::HashMap::new();
        for agent_entry in &resolved_agents {
            let ws_root = if let Some(ref ws) = agent_entry.workspace {
                let p = PathBuf::from(ws);
                if p.is_relative() {
                    state_dir.join(p)
                } else {
                    p
                }
            } else {
                xiaolin_core::workspace::resolve_workspace_root(&state_dir, &agent_entry.id, None)
            };
            let workspace = AgentWorkspace::new(&ws_root, &agent_entry.id);
            if let Err(e) = workspace.ensure_workspace() {
                tracing::warn!(
                    agent_id = %agent_entry.id,
                    error = %e,
                    "failed to ensure workspace bootstrap files"
                );
            }
            tracing::info!(
                agent_id = %agent_entry.id,
                workspace = %ws_root.display(),
                "agent workspace initialized"
            );
            workspaces.insert(agent_entry.id.clone(), workspace);
        }
        if workspaces.is_empty() {
            let default_root = config
                .workspace
                .as_deref()
                .map(|ws| {
                    let p = PathBuf::from(ws);
                    if p.is_relative() {
                        state_dir.join(p)
                    } else {
                        p
                    }
                })
                .unwrap_or_else(|| state_dir.join("workspace"));
            let ws = AgentWorkspace::new(&default_root, "main");
            let _ = ws.ensure_workspace();
            workspaces.insert("main".to_string(), ws);
        }

        let mut agent_skill_registries = std::collections::HashMap::new();
        for (agent_id, workspace) in &workspaces {
            let agent_ws_skills_dir = workspace.skills_dir();
            let mut agent_reg = base_skill_registry.clone();
            if agent_ws_skills_dir.exists() {
                let ws_skills = load_skills_from_dirs_with_layer(
                    &[agent_ws_skills_dir.as_path()],
                    SkillLayer::AgentWorkspace,
                );
                let ws_count = ws_skills.count();
                agent_reg.merge_from(ws_skills);
                if ws_count > 0 {
                    tracing::info!(
                        agent_id = %agent_id,
                        workspace_skills = ws_count,
                        total = agent_reg.count(),
                        "agent skill registry built with workspace overlay"
                    );
                }
            }
            let agent_allow = resolved_agents
                .iter()
                .find(|a| a.id == *agent_id)
                .and_then(|a| a.skills.as_deref());
            let before = agent_reg.count();
            agent_reg = agent_reg.filtered(&config.skills.allow, &config.skills.deny, agent_allow);
            let after = agent_reg.count();
            if before != after {
                tracing::info!(
                    agent_id = %agent_id,
                    before,
                    after,
                    "skills filtered by global allow/deny and per-agent config"
                );
            }
            agent_skill_registries.insert(agent_id.clone(), Arc::new(agent_reg));
        }

        Ok(BuildPhase3 {
            phase1: p1,
            runtime,
            router,
            tool_registry: Arc::new(tool_registry),
            base_skill_registry,
            agent_skill_registries,
            workspaces,
            llm_plugin_registry,
            todo_store,
        })
    }

    /// Phase 4: MCP + subagent tools, channel plugins, hub + skill tools.
    async fn phase4_channels_mcp(
        config: &XiaoLinConfig,
        p3: BuildPhase3,
    ) -> anyhow::Result<BuildPhase4> {
        let mut all_mcp_servers = config.mcp_servers.clone();
        let resolution = super::resolve_project_mcp(&mut all_mcp_servers);
        all_mcp_servers.extend(resolution.approved);
        let pending_approval_status = resolution.pending;

        let (mcp_result, channel_result) = tokio::join!(
            super::AppState::register_mcp_and_subagent_tools(
                &p3.phase1.agents,
                &all_mcp_servers,
                p3.runtime.clone(),
                p3.tool_registry.clone(),
            ),
            super::AppState::build_channels(config, &p3.tool_registry),
        );
        let (mut mcp_status_init, mcp_handles_init) = mcp_result?;
        mcp_status_init.extend(pending_approval_status);
        let (channel_registry, channel_inbound_tx, inbound_rx) = channel_result?;

        let base_skill_registry = Arc::new(p3.base_skill_registry.filtered(
            &config.skills.allow,
            &config.skills.deny,
            None,
        ));
        xiaolin_core::workspace::set_skill_prompt_mode(config.skills.prompt_mode.clone());
        let workspace_root_for_skills = std::env::current_dir()
            .ok()
            .map(|cwd| xiaolin_core::workspace::detect_workspace_root(&cwd));
        if let Some((_, ws)) = p3.workspaces.iter().next() {
            xiaolin_agent::builtin_tools::register_skill_tools_full(
                &p3.tool_registry,
                base_skill_registry.clone(),
                Arc::new(ws.clone()),
                workspace_root_for_skills.clone(),
            );
            tracing::info!(
                prompt_mode = ?config.skills.prompt_mode,
                "registered skill tool (list/read/search/write)"
            );
        } else {
            xiaolin_agent::builtin_tools::register_skill_tools(
                &p3.tool_registry,
                base_skill_registry.clone(),
            );
            tracing::info!(
                prompt_mode = ?config.skills.prompt_mode,
                "registered skill tool (list/read/search, no workspace for write)"
            );
        }

        let multi_agent_identity = p3.workspaces.len() > 1;
        for (agent_id, ws) in &p3.workspaces {
            let ws_arc = Arc::new(ws.clone());
            if multi_agent_identity {
                let sfx = memory_tool_agent_suffix(agent_id);
                let get_inner = Arc::new(xiaolin_agent::builtin_tools::GetIdentityTool::new(
                    ws_arc.clone(),
                ));
                let set_inner =
                    Arc::new(xiaolin_agent::builtin_tools::SetIdentityTool::new(ws_arc));
                let get_name = format!("get_identity__{sfx}");
                let set_name = format!("set_identity__{sfx}");
                let get_desc = format!("{} (agent `{}`)", get_inner.description(), agent_id);
                let set_desc = format!("{} (agent `{}`)", set_inner.description(), agent_id);
                p3.tool_registry.register(Arc::new(RenamedTool::new(
                    get_name,
                    get_desc,
                    get_inner as Arc<dyn xiaolin_core::tool::Tool + Send + Sync>,
                )));
                p3.tool_registry.register(Arc::new(RenamedTool::new(
                    set_name,
                    set_desc,
                    set_inner as Arc<dyn xiaolin_core::tool::Tool + Send + Sync>,
                )));
                tracing::info!(agent_id = %agent_id, "registered scoped get_identity / set_identity tools");
            } else {
                xiaolin_agent::builtin_tools::register_identity_tools(&p3.tool_registry, ws_arc);
                tracing::info!(agent_id = %agent_id, "registered get_identity / set_identity tools");
            }
        }

        let stream_event_tx = Arc::new(DashMap::new());
        let ask_question_pending = Arc::new(DashMap::new());
        let tool_orchestrator = {
            let mut policy_engine = xiaolin_execpolicy::PolicyEngine::new();
            let policy_loaded = Self::load_exec_policy(&mut policy_engine, config);
            if policy_loaded {
                Arc::new(xiaolin_agent::ToolOrchestrator::with_policy(policy_engine))
            } else {
                Arc::new(xiaolin_agent::ToolOrchestrator::new())
            }
        };
        p3.tool_registry.register(Arc::new(
            xiaolin_agent::builtin_tools::AskQuestionTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            ),
        ));
        p3.tool_registry
            .register(Arc::new(xiaolin_agent::builtin_tools::ConfirmTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            )));
        xiaolin_agent::builtin_tools::register_brief_tool(
            &p3.tool_registry,
            stream_event_tx.clone(),
        );
        tracing::info!("registered ask_question + confirm + send_user_message tools");

        let session_modes = xiaolin_agent::builtin_tools::SessionModeRegistry::new();
        // Plan tools are registered with a default mode state; at runtime the
        // task-local `CURRENT_SESSION_MODE` provides the per-session state.
        let default_mode = xiaolin_agent::builtin_tools::ExecutionModeState::new();
        xiaolin_agent::builtin_tools::register_plan_mode_tools(
            &p3.tool_registry,
            default_mode,
        );
        tracing::info!("registered plan mode tools (enter/exit_plan_mode)");

        // Structured plan step tracking (update_plan)
        let plan_step_store = xiaolin_agent::builtin_tools::PlanStepStore::new();
        xiaolin_agent::builtin_tools::register_update_plan_tool(
            &p3.tool_registry,
            stream_event_tx.clone(),
            plan_step_store.clone(),
        );
        tracing::info!("registered update_plan tool");

        // Legacy PTY interactive terminal tools (deprecated)
        let legacy_pty_manager = Arc::new(
            xiaolin_agent::builtin_tools::exec_command::PtySessionManager::with_default_timeout(),
        );
        xiaolin_agent::builtin_tools::register_exec_command_tools(
            &p3.tool_registry,
            legacy_pty_manager,
        );

        // Goal management tools (backed by session SQLite)
        let goal_store = Arc::new(xiaolin_agent::builtin_tools::GoalStore::new(
            p3.phase1.session_store.clone(),
        ));
        xiaolin_agent::builtin_tools::register_goal_tools(&p3.tool_registry, goal_store.clone());

        // ContributorRegistry: extension point for external tool contributors.
        // External plugins can register ToolContributor implementations here.
        let contributor_registry = ContributorRegistry::new();
        // (Future: contributors are registered via config or plugin discovery)
        let contributor_ctx = ContributorContext {
            agent_id: p3.phase1.agents.first().map(|a| a.agent_id.to_string()).unwrap_or_default(),
            channel_id: None,
        };
        let contributor_tool_count = contributor_registry.apply_to_registry(&p3.tool_registry, &contributor_ctx);
        if contributor_tool_count > 0 {
            tracing::info!(count = contributor_tool_count, "contributor tools registered");
        }

        Ok(BuildPhase4 {
            phase3: p3,
            channel_registry,
            channel_inbound_tx,
            inbound_rx,
            base_skill_registry,
            stream_event_tx,
            tool_orchestrator,
            mcp_status_init,
            mcp_handles_init,
            session_modes,
            goal_store,
            plan_step_store,
        })
    }

    /// Phase 2: per-agent memory + evolution stores + context engine hooks.
    async fn phase2_memory_evolution(
        config: &XiaoLinConfig,
        p4: BuildPhase4,
        preloaded_embedding: Option<Arc<dyn EmbeddingProvider>>,
    ) -> anyhow::Result<BuildPhase2Memory> {
        let (agent_episodic_map, agent_semantic_map, embedding_provider) =
            super::AppState::build_memory_with_provider(
                config,
                &p4.phase3.workspaces,
                &p4.phase3.tool_registry,
                preloaded_embedding,
            )
            .await?;

        let tool_count = p4.phase3.tool_registry.definitions().len();

        let message_bus = Arc::new(MessageBus::new(1024));
        for agent in &p4.phase3.phase1.agents {
            let aid = agent.agent_id.clone();
            let mut rx = message_bus.register(&aid).await;
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
        }
        xiaolin_agent::builtin_tools::register_session_tools(
            &p4.phase3.tool_registry,
            p4.phase3.phase1.session_store.clone(),
            message_bus.clone(),
        );

        let (feedback_store, trajectory_store, skill_store, prompt_distiller, skill_embedding_store, skill_usage_store) = {
            let shared_pool = p4.phase3.phase1.pool.clone();
            let fs = FeedbackStore::open(shared_pool.clone()).await?;
            let ts = TrajectoryStore::open(shared_pool.clone()).await?;
            let ss = SkillStore::open(shared_pool.clone()).await?;
            let pd = PromptDistiller::open(shared_pool.clone()).await?;
            let ses = xiaolin_core::skill_embedding::SkillEmbeddingStore::open(shared_pool.clone()).await?;
            let sus = xiaolin_core::skill_usage::SkillUsageStore::open(shared_pool).await?;
            (fs, ts, ss, pd, ses, sus)
        };

        let mut context_engine =
            xiaolin_context::ContextEngine::new(xiaolin_context::DEFAULT_COMPACTION_THRESHOLD);
        context_engine.add_hook(Arc::new(xiaolin_context::CompactionHook::new(
            xiaolin_context::CompactionStrategy::default(),
        )));
        context_engine.add_hook(Arc::new(xiaolin_context::ContentFilterHook::default()));
        context_engine.add_hook(Arc::new(xiaolin_context::SystemReminderHook::default()));
        let mut personality_hook = xiaolin_context::AgentPersonalityHook::new();
        for (agent_id, workspace) in &p4.phase3.workspaces {
            personality_hook.add_agent(agent_id, workspace);
        }
        context_engine.add_hook(Arc::new(personality_hook));
        if config.memory.enabled && !agent_episodic_map.is_empty() {
            context_engine.add_hook(Arc::new(xiaolin_context::AgentMemoryIngestHook::new(
                agent_episodic_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                agent_semantic_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                embedding_provider.clone(),
                10,
            )));

            context_engine.add_hook(Arc::new(xiaolin_context::MemoryKeywordInterceptor::new(
                agent_semantic_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                embedding_provider.clone(),
            )));

            let consolidation_model = config
                .memory
                .consolidation_model
                .clone()
                .or_else(|| {
                    p4.phase3
                        .phase1
                        .agents
                        .first()
                        .map(|a| a.model.model.clone())
                })
                .or_else(|| config.models.values().next().map(|m| m.model.clone()))
                .unwrap_or_else(|| "deepseek/deepseek-v4-flash".to_string());

            context_engine.add_hook(Arc::new(
                crate::consolidation::MemoryConsolidationHook::new(
                    p4.phase3.runtime.default_provider_arc(),
                    agent_episodic_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    agent_semantic_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    embedding_provider.clone(),
                    xiaolin_memory::ImportanceScorer::from(config.memory.importance.clone()),
                    config.memory.consolidation_min_messages,
                    consolidation_model,
                ),
            ));
        }
        tracing::info!(
            hooks = context_engine.hook_count(),
            "context engine initialized"
        );

        Ok(BuildPhase2Memory {
            phase4: p4,
            agent_episodic_map,
            agent_semantic_map,
            embedding_provider,
            message_bus,
            feedback_store,
            prompt_distiller,
            trajectory_store,
            skill_store,
            skill_embedding_store,
            skill_usage_store,
            context_engine,
            tool_count,
        })
    }

    /// Phase 5: cron, model router, WebSocket broadcast fanout.
    async fn phase5_cron(
        config: &XiaoLinConfig,
        p2: BuildPhase2Memory,
    ) -> anyhow::Result<BuildPhase5> {
        let shared_pool = p2.phase4.phase3.phase1.pool.clone();
        let cron_store = CronJobStore::open(shared_pool.clone()).await?;
        let notification_store =
            crate::notification_store::NotificationStore::open(shared_pool).await?;

        let budget_tracker = BudgetTracker::new(config.model_router.daily_budget);

        let model_router = if config.model_router.enabled {
            let strategy_raw = config.model_router.strategy.as_str();
            let strategy = match strategy_raw {
                "cost_optimized" => xiaolin_model_router::RoutingStrategy::CostOptimized,
                "fallback" => xiaolin_model_router::RoutingStrategy::Fallback,
                "quality_first" | "latency_optimized" => {
                    tracing::warn!(
                        requested = strategy_raw,
                        "model_router.strategy is deprecated (quality/latency ranking needs live metrics); using `fallback`"
                    );
                    xiaolin_model_router::RoutingStrategy::Fallback
                }
                _ => xiaolin_model_router::RoutingStrategy::Fixed,
            };
            let mut router =
                xiaolin_model_router::ModelRouter::new(strategy, budget_tracker.clone());
            if !config.model_router.fallback_chain.is_empty() {
                router.set_fallback_chain(config.model_router.fallback_chain.clone());
            }
            tracing::info!(strategy = ?strategy, "model router initialized");
            Some(Arc::new(router))
        } else {
            None
        };

        let (ws_broadcast, _) = tokio::sync::broadcast::channel::<String>(256);

        Ok(BuildPhase5 {
            phase2: p2,
            cron_store,
            notification_store,
            budget_tracker,
            model_router,
            ws_broadcast,
        })
    }

    /// Run all phases in dependency order and produce a ready [`AppState`].
    pub(crate) async fn build(config: XiaoLinConfig) -> anyhow::Result<AppState> {
        let mut ssrf_hosts = config.security.ssrf_allowed_hosts.clone();
        // In dev mode, auto-allow localhost so agents can verify local dev servers
        if cfg!(debug_assertions)
            || std::env::var("XIAOLIN_PROFILE").unwrap_or_default() == "dev"
        {
            for host in ["localhost", "127.0.0.1", "[::1]"] {
                let h = host.to_string();
                if !ssrf_hosts.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
                    ssrf_hosts.push(h);
                }
            }
        }
        if !ssrf_hosts.is_empty() {
            tracing::info!(
                hosts = ?ssrf_hosts,
                "SSRF: registering allowed hosts that bypass private-IP checks"
            );
            xiaolin_security::ssrf::set_ssrf_allowed_hosts(ssrf_hosts);
        }

        tracing::info!(
            policy = ?config.security.dangerous_ops_policy,
            pattern_count = config.security.dangerous_patterns.len(),
            "Dangerous-ops: initializing policy"
        );
        if let Err(e) = xiaolin_security::dangerous_ops::set_dangerous_ops_config(
            config.security.dangerous_ops_policy,
            &config.security.dangerous_patterns,
        ) {
            tracing::error!(error = %e, "Dangerous-ops: failed to initialize policy");
        }

        let p1 = Self::phase1_config_session(&config).await?;

        // Start embedding model loading early (parallel with Phase 3 + 4).
        let embedding_handle = {
            let mem_enabled = config.memory.enabled;
            let emb_cfg = config.memory.embedding.clone();
            let creds = config.credentials.clone();
            tokio::spawn(async move {
                if !mem_enabled {
                    return Ok::<_, anyhow::Error>(None);
                }
                match xiaolin_memory::create_embedding_provider(&emb_cfg, Some(&creds)).await {
                    Ok(ep) => {
                        tracing::info!(
                            provider = ep.name(),
                            dims = ep.dimensions(),
                            "embedding provider initialized (preloaded)"
                        );
                        Ok(Some(Arc::from(ep) as Arc<dyn EmbeddingProvider>))
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "embedding provider unavailable, memory will use keyword-only search"
                        );
                        Ok(None)
                    }
                }
            })
        };

        let p3 = Self::phase3_agent_runtime_tools(&config, p1).await?;
        let p4 = Self::phase4_channels_mcp(&config, p3).await?;

        // Await the pre-loaded embedding provider.
        let preloaded_embedding: Option<Arc<dyn EmbeddingProvider>> = embedding_handle
            .await
            .map_err(|e| anyhow::anyhow!("embedding preload task panicked: {e}"))??;

        let p2 = Self::phase2_memory_evolution(&config, p4, preloaded_embedding).await?;
        let p5 = Self::phase5_cron(&config, p2).await?;

        // All stores have created their tables; migrate legacy DBs if present.
        if let Some(parent) = p5.phase2.phase4.phase3.phase1.db_path.parent() {
            helpers::migrate_legacy_databases(
                &p5.phase2.phase4.phase3.phase1.pool,
                parent,
            )
            .await?;
        }

        // Backfill project_id for sessions that have work_dir but no project yet.
        let _ = p5
            .phase2
            .phase4
            .phase3
            .phase1
            .session_store
            .migrate_sessions_to_projects()
            .await;

        let agent_count = p5.phase2.phase4.phase3.phase1.agent_count;
        let tool_count = p5.phase2.tool_count;
        let channel_count = p5.phase2.phase4.channel_registry.channel_count();
        let workspace_count = p5.phase2.phase4.phase3.workspaces.len();
        let db_path = p5.phase2.phase4.phase3.phase1.db_path.clone();
        let initial_agents = p5.phase2.phase4.phase3.phase1.agents.clone();

        tracing::info!(
            agent_count,
            tool_count,
            channel_count,
            workspace_count,
            base_skills = p5.phase2.phase4.base_skill_registry.count(),
            agent_registries = p5.phase2.phase4.phase3.agent_skill_registries.len(),
            db = %db_path.display(),
            "application state initialized (full stack)"
        );

        let inbound_rx = p5.phase2.phase4.inbound_rx;
        let trajectory_store = Arc::new(p5.phase2.trajectory_store);
        let skill_store = Arc::new(p5.phase2.skill_store);
        let skill_usage_store_arc = Arc::new(p5.phase2.skill_usage_store);
        let cost_store = Arc::new(
            xiaolin_session::CostStore::open(p5.phase2.phase4.phase3.phase1.pool.clone())
                .await?,
        );
        p5.phase2
            .phase4
            .phase3
            .runtime
            .attach_evolution_stores(skill_store.clone(), trajectory_store.clone());
        p5.phase2
            .phase4
            .phase3
            .runtime
            .attach_skill_usage_store(skill_usage_store_arc.clone());
        p5.phase2
            .phase4
            .phase3
            .runtime
            .set_skills_deny(config.skills.deny.clone());
        p5.phase2
            .phase4
            .phase3
            .runtime
            .set_skills_allow(config.skills.allow.clone());

        let prompt_injection_enabled = config.security.prompt_injection_detection;
        let config_live_val = serde_json::to_value(&config).unwrap_or_default();
        let auth = Arc::new(xiaolin_security::ApiKeyAuth::new(
            &xiaolin_security::AuthConfig {
                enabled: !config.security.api_keys.is_empty(),
                api_keys: config.security.api_keys.clone(),
            },
        ));
        let runtime_for_subagent = p5.phase2.phase4.phase3.runtime.clone();
        let runtime_for_session = p5.phase2.phase4.phase3.runtime.clone();
        let session_store_for_session = p5.phase2.phase4.phase3.phase1.session_store.clone();
        let event_log_for_svc = p5.phase2.phase4.phase3.phase1.event_log.clone();
        let default_agent_config = initial_agents
            .first()
            .expect("at least one agent must be configured")
            .clone();

        let prompt_guard = {
            let mut pg = xiaolin_security::PromptGuard::new();
            pg.set_enabled(prompt_injection_enabled);
            Arc::new(pg)
        };

        let stream_event_tx_for_executor = p5.phase2.phase4.stream_event_tx.clone();
        let mode_registry_for_executor = p5.phase2.phase4.session_modes.clone();
        let todo_store_for_executor = p5.phase2.phase4.phase3.todo_store.clone();
        let goal_store_for_executor = p5.phase2.phase4.goal_store.clone();
        let plan_file_store =
            xiaolin_agent::builtin_tools::PlanFileStore::default();
        let tool_orchestrator_for_executor = p5.phase2.phase4.tool_orchestrator.clone();

        // Wrap the tool registry in Arc early so the same instance is shared
        // between the session executor and state.rt.tool_registry. ToolRegistry
        // uses interior mutability (RwLock) so later registrations are visible
        // to both holders.
        let shared_tool_registry = {
            let reg = p5.phase2.phase4.phase3.tool_registry;
            xiaolin_agent::builtin_tools::register_tool_search(&reg);
            reg
        };

        let session_behavior_overrides: Arc<dashmap::DashMap<String, xiaolin_core::agent_config::BehaviorConfig>> =
            Arc::new(dashmap::DashMap::new());
        let permission_preset_registry =
            Arc::new(xiaolin_core::agent_config::PermissionPresetRegistry::default());

        let live_agents_swap: Arc<ArcSwap<Vec<AgentConfig>>> =
            Arc::new(ArcSwap::from_pointee(initial_agents.clone()));

        let subagent_manager_shared = Arc::new(xiaolin_agent::SubAgentManager::new(
            runtime_for_subagent,
            initial_agents.clone(),
            xiaolin_core::agent_config::SubAgentPolicy::default(),
            std::sync::Arc::new(xiaolin_agent::SpawnController::new(
                xiaolin_agent::SpawnConfig::from_policy_fallback(
                    xiaolin_core::agent_config::SubAgentPolicy::default().max_parallel,
                ),
            )),
        ));

        let session_manager = Arc::new(xiaolin_session_actor::SessionManager::new(
            Arc::new(xiaolin_agent::RuntimeTurnExecutor {
                runtime: runtime_for_session.clone(),
                config: default_agent_config,
                tool_registry: shared_tool_registry.clone(),
                llm_override: None,
                session_store: Some(session_store_for_session.clone()),
                mode_registry: Some(mode_registry_for_executor),
                todo_store: Some(todo_store_for_executor),
                goal_store: Some(goal_store_for_executor),
                plan_file_store: Some(plan_file_store.clone()),
                stream_event_tx: Some(stream_event_tx_for_executor),
                subagent_manager: Some(subagent_manager_shared.clone()),
                tool_orchestrator: Some(tool_orchestrator_for_executor),
                behavior_overrides: Some(session_behavior_overrides.clone()),
                live_agents: Some(live_agents_swap.clone()),
                cost_store: Some(cost_store.clone()),
            }),
        ));
        let mut state = AppState {
            cfg: super::ConfigState {
                config: Arc::new(config),
                config_live: Arc::new(ArcSwap::new(Arc::new(config_live_val))),
                auth,
                runtime_route_bindings: Arc::new(tokio::sync::RwLock::new(Vec::new())),
                last_good_agents: Arc::new(tokio::sync::RwLock::new(initial_agents.clone())),
                live_agents_swap: live_agents_swap.clone(),
            },
            rt: super::RuntimeState {
                router: Arc::new(tokio::sync::RwLock::new(p5.phase2.phase4.phase3.router)),
                runtime: p5.phase2.phase4.phase3.runtime,
                tool_registry: shared_tool_registry.clone(),
                base_skill_registry: Arc::new(ArcSwap::new(p5.phase2.phase4.base_skill_registry)),
                unfiltered_skill_registry: Arc::new(ArcSwap::new(Arc::new(
                    p5.phase2.phase4.phase3.base_skill_registry,
                ))),
                agent_skill_registries: Arc::new(ArcSwap::new(Arc::new(
                    p5.phase2.phase4.phase3.agent_skill_registries,
                ))),
                mcp_skill_registry: Arc::new(ArcSwap::new(Arc::new(
                    xiaolin_core::skill::SkillRegistry::new(),
                ))),
                workspaces: Arc::new(p5.phase2.phase4.phase3.workspaces),
                prompt_guard: prompt_guard.clone(),
                session_modes: p5.phase2.phase4.session_modes,
                todo_store: p5.phase2.phase4.phase3.todo_store,
                goal_store: p5.phase2.phase4.goal_store,
                plan_file_store,
                plan_step_store: p5.phase2.phase4.plan_step_store,
                permission_preset_registry: permission_preset_registry.clone(),
                embedding_update_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            store: super::StorageState {
                session_store: p5.phase2.phase4.phase3.phase1.session_store,
                event_log: p5.phase2.phase4.phase3.phase1.event_log,
                cron_store: Arc::new(p5.cron_store),
                cron_wake: Arc::new(tokio::sync::Notify::new()),
                notification_store: Arc::new(p5.notification_store),
                feedback_store: Arc::new(p5.phase2.feedback_store),
                prompt_distiller: Arc::new(p5.phase2.prompt_distiller),
                trajectory_store,
                skill_store,
                skill_embedding_store: Arc::new(p5.phase2.skill_embedding_store),
                skill_usage_store: skill_usage_store_arc.clone(),
                context_engine: Arc::new(p5.phase2.context_engine),
                cost_store: cost_store.clone(),
                search_index: p5.phase2.phase4.phase3.phase1.search_index.clone(),
            },
            mem: super::MemoryState {
                agent_episodic: Arc::new(p5.phase2.agent_episodic_map),
                agent_semantic: Arc::new(p5.phase2.agent_semantic_map),
                embedding_provider: p5.phase2.embedding_provider,
            },
            ext: super::ExtensionState {
                channel_registry: Arc::new(tokio::sync::RwLock::new(
                    p5.phase2.phase4.channel_registry,
                )),
                message_bus: p5.phase2.message_bus,
                mcp_status: Arc::new(ArcSwap::new(Arc::new(p5.phase2.phase4.mcp_status_init))),
                mcp_handles: Arc::new(tokio::sync::Mutex::new(p5.phase2.phase4.mcp_handles_init)),
                channel_inbound_tx: p5.phase2.phase4.channel_inbound_tx,
                llm_plugin_registry: Arc::new(tokio::sync::RwLock::new(
                    p5.phase2.phase4.phase3.llm_plugin_registry,
                )),
                chat_locks: Arc::new(dashmap::DashMap::new()),
                chat_cancels: Arc::new(dashmap::DashMap::new()),
                chat_model_overrides: Arc::new(dashmap::DashMap::new()),
                wechat_login_sessions: Arc::new(dashmap::DashMap::new()),
                session_behavior_overrides: session_behavior_overrides.clone(),
                session_preset_ids: Arc::new(dashmap::DashMap::new()),
                hub_client: Arc::new(xiaolin_core::hub::HubClient::with_defaults()),
            },
            obs: super::ObserveState {
                metrics_collector: xiaolin_observe::shared_metrics_collector(),
                budget_tracker: Arc::new(p5.budget_tracker),
                model_router: p5.model_router,
            },
            strm: super::StreamState {
                stream_event_tx: p5.phase2.phase4.stream_event_tx,
                tool_orchestrator: p5.phase2.phase4.tool_orchestrator.clone(),
                git_watcher_manager: Arc::new(crate::git_watcher::GitWatcherManager::new(p5.ws_broadcast.clone())),
                ws_broadcast: p5.ws_broadcast,
                subagent_manager: subagent_manager_shared.clone(),
                session_manager: session_manager.clone(),
                pty_manager: Arc::new(xiaolin_pty::PtySessionManager::new()),
                agent_def_watcher: None,
                skill_watcher: None,
                pending_elicitations: Arc::new(dashmap::DashMap::new()),
            },
            svc: super::SharedServices {
                runtime: runtime_for_session,
                tool_registry: shared_tool_registry,
                session_store: session_store_for_session,
                event_log: event_log_for_svc,
                context_engine: Arc::new(
                    xiaolin_context::ContextEngine::new(
                        xiaolin_context::DEFAULT_COMPACTION_THRESHOLD,
                    ),
                ),
                prompt_guard,
                session_manager,
            },
        };

        // Agent terminal tools (terminal_open, terminal_input, terminal_close)
        xiaolin_agent::builtin_tools::register_terminal_tools(
            &state.rt.tool_registry,
            state.strm.pty_manager.clone(),
        );
        tracing::info!("registered agent terminal tools (terminal_open/input/close)");

        state.strm.pty_manager.start_cleanup_task();
        tracing::info!("PTY idle session cleanup task started (runs every 60s)");

        state
            .rt
            .tool_registry
            .register(Arc::new(crate::mcp_tool::ManageMcpServerTool::new(
                state.cfg.config_live.clone(),
                state.ext.mcp_status.clone(),
                state.ext.mcp_handles.clone(),
                state.rt.tool_registry.clone(),
            )));
        tracing::info!("registered manage_mcp_server tool");

        state.rt.tool_registry.register_deferred(Arc::new(
            crate::mcp_tool::McpListResourcesTool::new(state.ext.mcp_handles.clone()),
        ));
        state.rt.tool_registry.register_deferred(Arc::new(
            crate::mcp_tool::McpReadResourceTool::new(state.ext.mcp_handles.clone()),
        ));
        tracing::info!("registered mcp__list_resources and mcp__read_resource tools (deferred)");

        {
            let handles = state.ext.mcp_handles.lock().await;
            for (sid, handle) in handles.iter() {
                super::AppState::spawn_server_request_watcher(
                    sid,
                    handle,
                    state.strm.ws_broadcast.clone(),
                    state.strm.pending_elicitations.clone(),
                );
            }
            if !handles.is_empty() {
                tracing::info!(count = handles.len(), "spawned server request watchers for MCP clients");
            }
        }

        state
            .rt
            .tool_registry
            .register(Arc::new(crate::cron_tool::ManageCronTool::new(
                state.store.cron_store.clone(),
                state.store.cron_wake.clone(),
            )));
        tracing::info!("registered manage_cron tool");

        state
            .rt
            .tool_registry
            .register(Arc::new(crate::channel_tool::ListChannelsTool::new(
                state.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(crate::channel_tool::AddChannelTool::new(
                state.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(crate::channel_tool::RemoveChannelTool::new(
                state.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(crate::channel_tool::NotifyChannelTool::new(
                state.clone(),
            )));
        tracing::info!("registered channel management tools (incl. notify_channel)");

        {
            let json_dir = PathBuf::from("config/sub-agents");
            let md_dir = PathBuf::from(".xiaolin/agents");
            let mut subagent_defs = xiaolin_core::agent_config::load_all_subagent_defs(
                Some(&json_dir),
                Some(&md_dir),
            );

            // Also load user-level sub-agent definitions from ~/.xiaolin/subagents/
            if let Some(home) = dirs::home_dir() {
                let user_md_dir = home.join(".xiaolin").join("subagents");
                if user_md_dir.exists() {
                    match xiaolin_core::agent_config::load_subagent_defs_markdown(&user_md_dir) {
                        Ok(user_defs) => {
                            for d in user_defs {
                                if let Some(existing) = subagent_defs.iter_mut().find(|e| e.id == d.id) {
                                    tracing::info!(id = %d.id, "user sub-agent def overrides existing");
                                    *existing = d;
                                } else {
                                    subagent_defs.push(d);
                                }
                            }
                        }
                        Err(e) => tracing::warn!(
                            dir = %user_md_dir.display(),
                            error = %e,
                            "failed to load user markdown sub-agent defs"
                        ),
                    }
                }
            }

            let def_count = subagent_defs.len();
            state
                .strm
                .subagent_manager
                .set_subagent_defs(subagent_defs);
            tracing::info!(count = def_count, "loaded sub-agent definitions (builtin + custom + user)");

            // Start hot-reload watcher for agent definition directories
            let mut watch_dirs = vec![json_dir, md_dir];
            if let Some(home) = dirs::home_dir() {
                watch_dirs.push(home.join(".xiaolin").join("subagents"));
            }
            match crate::agent_def_watcher::AgentDefWatcher::start(
                watch_dirs,
                state.strm.subagent_manager.clone(),
            ) {
                Ok(watcher) => {
                    state.strm.agent_def_watcher = Some(Arc::new(watcher));
                    tracing::info!("agent definition hot-reload watcher started");
                }
                Err(e) => tracing::warn!(error = %e, "failed to start agent def watcher"),
            }
        }

        state.rt.tool_registry.register(Arc::new(
            xiaolin_agent::SubAgentTool::new(
                state.strm.subagent_manager.clone(),
                state.rt.tool_registry.clone(),
                xiaolin_core::agent_config::SubAgentPolicy::default(),
            )
            .with_session_store(Arc::clone(&state.store.session_store)),
        ));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::SubAgentGetTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::SubAgentListTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::WaitAgentTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::ListAgentsTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::GetAgentInfoTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::ResumeSubagentTool::new(
                state.strm.subagent_manager.clone(),
                state.rt.tool_registry.clone(),
                xiaolin_core::agent_config::SubAgentPolicy::default(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::SendMessageTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(xiaolin_agent::TaskStopTool::new()));
        tracing::info!("registered sub-agent tools (spawn_subagent, subagent_get, subagent_list, list_agents, get_agent_info, resume_subagent, send_message, task_stop)");

        // Re-register skill tool with hot-reload callback + semantic search.
        {
            let reload_state = state.clone();
            let reload_cb: Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync> =
                Arc::new(move || {
                    reload_state.reload_skills()?;
                    reload_state.spawn_skill_embedding_update();
                    Ok(())
                });
            let ws_root = std::env::current_dir()
                .ok()
                .map(|cwd| xiaolin_core::workspace::detect_workspace_root(&cwd));
            let skill_reg = state.rt.base_skill_registry.load();
            let semantic = state
                .mem
                .embedding_provider
                .clone()
                .map(|ep| (state.store.skill_embedding_store.clone(), ep));
            if let Some((_, ws)) = state.rt.workspaces.iter().next() {
                let mut tool = xiaolin_agent::builtin_tools::skill::UnifiedSkillTool::new(
                    skill_reg.clone(),
                    Some(Arc::new(ws.clone())),
                );
                if let Some(root) = ws_root {
                    tool = tool.with_workspace_root(root);
                }
                if let Some((store, provider)) = semantic {
                    tool = tool.with_semantic(store, provider);
                }
                tool = tool.with_usage_store(state.store.skill_usage_store.clone());
                let tool = tool.with_reload_callback(reload_cb);
                state.rt.tool_registry.register(Arc::new(tool));
                tracing::info!("re-registered skill tool with hot-reload + semantic search");
            }
        }

        state.spawn_skill_evolution_tasks();
        state.spawn_skill_embedding_update();

        // Start filesystem watcher for skill directories
        {
            let paths_cfg = &state.cfg.config.paths;
            let mut watch_dirs = vec![
                helpers::resolve_skills_dir(paths_cfg),
                xiaolin_core::skill::resolve_global_skills_dir(),
            ];
            let ext_dir = xiaolin_core::paths::resolve_extensions_dir_from(Some(paths_cfg));
            if ext_dir.exists() {
                watch_dirs.push(ext_dir);
            }
            if let Ok(cwd) = std::env::current_dir() {
                let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
                let project_skills = ws_root.join(".xiaolin").join("skills");
                if project_skills.exists() {
                    watch_dirs.push(project_skills);
                }
            }
            match crate::skill_watcher::SkillWatcher::start(watch_dirs, state.clone()) {
                Ok(watcher) => {
                    state.strm.skill_watcher = Some(Arc::new(watcher));
                    tracing::info!("skill directory hot-reload watcher started");
                }
                Err(e) => tracing::warn!(error = %e, "failed to start skill watcher"),
            }
        }

        {
            let mcp_state = state.clone();
            tokio::spawn(async move {
                mcp_state.refresh_mcp_skills().await;
            });
        }

        state.spawn_inbound_dispatcher(inbound_rx);

        {
            let search_index = state.store.search_index.clone();
            tokio::spawn(async move {
                match search_index.needs_backfill().await {
                    Ok(true) => {
                        tracing::info!("search index backfill started");
                        if let Err(e) = search_index.bulk_index_history(None).await {
                            tracing::warn!(error = %e, "search index backfill failed");
                        } else {
                            tracing::info!("search index backfill completed");
                        }
                    }
                    Ok(false) => tracing::debug!("search index backfill not needed"),
                    Err(e) => tracing::warn!(error = %e, "search index backfill check failed"),
                }
            });
            tracing::info!("search index background task started");
        }

        // Spawn periodic resource GC loop (60s interval)
        {
            let gc_state = state.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(60));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;
                    gc_state.gc_stale_resources().await;

                    // Memory monitoring
                    if let Some(rss_bytes) = crate::memory_monitor::get_process_rss_bytes() {
                        let rss_mb = rss_bytes / (1024 * 1024);
                        if rss_mb > 4096 {
                            tracing::error!(rss_mb, "CRITICAL: process RSS exceeds 4GB");
                        } else if rss_mb > 1024 {
                            tracing::warn!(rss_mb, "process RSS exceeds 1GB");
                        } else {
                            tracing::debug!(rss_mb, "process RSS");
                        }
                    }
                }
            });
            tracing::info!("resource GC background task started (60s interval)");
        }

        let dream_secs = state.cfg.config.memory.dreaming_interval_secs;
        if state.cfg.config.memory.enabled && dream_secs > 0 && !state.mem.agent_episodic.is_empty()
        {
            let episodic = state.mem.agent_episodic.clone();
            let semantic = state.mem.agent_semantic.clone();
            let dream_embedder = state.mem.embedding_provider.clone();
            let dream_scorer = Some(xiaolin_memory::ImportanceScorer::from(
                state.cfg.config.memory.importance.clone(),
            ));
            let dream_skill_store = state.store.skill_store.clone();
            let dream_llm = Arc::new(super::LlmSkillExtraction {
                provider: state.rt.runtime.default_provider_arc(),
                model: state
                    .cfg
                    .config
                    .agents
                    .list
                    .first()
                    .and_then(|a| a.model.clone())
                    .or_else(|| {
                        state.cfg.config.models.values().next().map(|m| m.model.clone())
                    })
                    .unwrap_or_else(|| "deepseek/deepseek-v4-flash".to_string()),
            });
            tokio::spawn(async move {
                const DREAM_EPISODE_BATCH: i64 = 50;
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(dream_secs));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    for (agent_id, ep) in episodic.iter() {
                        if let Some(sem) = semantic.get(agent_id) {
                            let pipe = DreamingPipeline {
                                episodic: ep.as_ref(),
                                semantic: sem.as_ref(),
                                embedder: dream_embedder.clone(),
                                scorer: dream_scorer.clone(),
                            };
                            match pipe.run_dream_cycle(DREAM_EPISODE_BATCH).await {
                                Ok(report) => {
                                    if report.episodes_considered > 0
                                        || report.embeddings_backfilled > 0
                                        || report.importance_rescored > 0
                                    {
                                        tracing::info!(
                                            agent_id = %agent_id,
                                            considered = report.episodes_considered,
                                            marked = report.episodes_marked,
                                            rels = report.relationships_added,
                                            facts = report.facts_extracted,
                                            embeddings = report.embeddings_backfilled,
                                            rescored = report.importance_rescored,
                                            skill_candidates = report.skill_candidates_found,
                                            "dream cycle completed"
                                        );
                                    }
                                    if report.skill_candidates_found > 0 {
                                        promote_episodes_to_skills(
                                            ep.as_ref(),
                                            &dream_skill_store,
                                            dream_llm.as_ref(),
                                            agent_id,
                                        )
                                        .await;
                                    }
                                }
                                Err(e) => tracing::warn!(
                                    agent_id = %agent_id,
                                    error = %e,
                                    "dream cycle failed"
                                ),
                            }
                        }
                    }
                }
            });
            tracing::info!(
                interval_secs = dream_secs,
                "dreaming background task started"
            );
        }

        if let Some(ttl_hours) = state.cfg.config.session.ttl_hours {
            let store = state.store.session_store.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(e) = store.cleanup_expired(ttl_hours).await {
                        tracing::warn!(error = %e, "session cleanup failed");
                    }
                }
            });
            tracing::info!(ttl_hours, "session TTL cleanup task started (runs hourly)");
        }

        {
            let usage_store = state.store.skill_usage_store.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(86400));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match usage_store.purge_old(90).await {
                        Ok(purged) if purged > 0 => {
                            tracing::info!(purged, "skill usage data cleanup: removed old entries (>90 days)");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "skill usage data cleanup failed");
                        }
                        _ => {}
                    }
                }
            });
            tracing::info!("skill usage cleanup task started (runs daily, 90-day retention)");
        }

        Ok(state)
    }
}

async fn promote_episodes_to_skills(
    episodic: &EpisodicMemory,
    skill_store: &SkillStore,
    llm: &dyn xiaolin_evolution::LlmExtractionCallback,
    agent_id: &str,
) {
    use xiaolin_evolution::{ExtractedSkill, SkillStatus};

    let episodes = match episodic.high_importance(0.8, 10).await {
        Ok(eps) => eps,
        Err(e) => {
            tracing::warn!(agent_id, error = %e, "failed to query high-importance episodes");
            return;
        }
    };

    for ep in &episodes {
        let summary = format!(
            "Agent '{}' completed the following high-value task:\n\n{}",
            agent_id, ep.summary
        );
        let pattern = match llm.extract_pattern(&summary).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(episode_id = %ep.id, error = %e, "LLM skill extraction from episode skipped");
                continue;
            }
        };
        if pattern.strategy_template.split_whitespace().count() < 10 {
            continue;
        }

        let needle = format!("{} {}", pattern.name, pattern.task_pattern);
        let is_dup = skill_store
            .find_similar(&needle, 10)
            .await
            .map(|existing| !existing.is_empty())
            .unwrap_or(false);
        if is_dup {
            tracing::debug!(episode_id = %ep.id, name = %pattern.name, "skill already exists, skipping promotion");
            continue;
        }

        let skill = ExtractedSkill {
            id: format!("ep_promote_{}", &ep.id),
            name: pattern.name.clone(),
            task_pattern: pattern.task_pattern,
            strategy_template: pattern.strategy_template,
            parameters: pattern.parameters,
            source_trajectory_ids: vec![ep.id.clone()],
            success_rate: 0.0,
            usage_count: 0,
            status: SkillStatus::Candidate,
            created_at: chrono::Utc::now().to_rfc3339(),
            version: 1,
            parent_id: None,
        };
        match skill_store.save_skill(&skill).await {
            Ok(()) => tracing::info!(
                skill_id = %skill.id,
                name = %skill.name,
                episode_id = %ep.id,
                "promoted episode to candidate skill"
            ),
            Err(e) => tracing::warn!(
                episode_id = %ep.id,
                error = %e,
                "failed to save promoted skill"
            ),
        }
    }
}
