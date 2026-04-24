use std::path::{Path, PathBuf};
use std::sync::Arc;

use fastclaw_agent::{
    create_provider, create_provider_chain, create_provider_with_credentials, AgentRuntime,
    FallbackProvider,
};
use fastclaw_core::agent_config::{self, AgentConfig, AgentModelConfig};
use fastclaw_core::bus::MessageBus;
use fastclaw_core::channel::{ChannelPlugin, ChannelRegistry};
use fastclaw_core::config::FastClawConfig;
use fastclaw_core::skill::SkillRegistry;
use fastclaw_core::tool::Tool;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::workspace::AgentWorkspace;
use fastclaw_core::routing::RuntimeRouteBinding;
use fastclaw_core::Router as AgentRouter;
use fastclaw_cron::CronJobStore;
use fastclaw_dag::CheckpointStore;
use fastclaw_evolution::{FeedbackStore, PromptDistiller, SkillExtractor, SkillStore, TrajectoryStore};
use fastclaw_memory::{DreamingPipeline, EmbeddingProvider, EpisodicMemory, SemanticMemory};
use fastclaw_model_router::BudgetTracker;
use fastclaw_plugin::PluginRegistry;
use fastclaw_session::SessionStore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::memory_scope::memory_tool_agent_suffix;
use crate::scoped_tool::RenamedTool;

pub type SharedRouter = Arc<tokio::sync::RwLock<AgentRouter>>;

/// Reject agent sets that would leave the gateway without a usable routing table.
pub fn validate_agents_for_reload(agents: &[AgentConfig]) -> anyhow::Result<()> {
    if agents.is_empty() {
        anyhow::bail!(
            "agent reload rejected: no agents loaded (config directory empty or all JSON files failed to parse)"
        );
    }
    let mut seen = std::collections::HashSet::new();
    for a in agents {
        if a.agent_id.trim().is_empty() {
            anyhow::bail!("agent reload rejected: empty agent_id in one of the config files");
        }
        if a.model.provider.trim().is_empty() || a.model.model.trim().is_empty() {
            anyhow::bail!(
                "agent reload rejected: agent `{}` has empty model provider or model id",
                a.agent_id
            );
        }
        if !seen.insert(a.agent_id.as_str()) {
            anyhow::bail!(
                "agent reload rejected: duplicate agent_id `{}`",
                a.agent_id
            );
        }
    }
    Ok(())
}

// --- Phased initialization for [`AppState::new`] (see [`StateBuilder`]). ---

struct BuildPhase1 {
    agents: Vec<AgentConfig>,
    agent_count: usize,
    db_path: PathBuf,
    session_store: Arc<SessionStore>,
}

struct BuildPhase3 {
    phase1: BuildPhase1,
    runtime: Arc<AgentRuntime>,
    router: AgentRouter,
    tool_registry: ToolRegistry,
    plugin_registry: PluginRegistry,
    base_skill_registry: SkillRegistry,
    agent_skill_registries: std::collections::HashMap<String, Arc<SkillRegistry>>,
    workspaces: std::collections::HashMap<String, AgentWorkspace>,
}

struct BuildPhase4 {
    phase3: BuildPhase3,
    channel_registry: ChannelRegistry,
    inbound_rx: tokio::sync::mpsc::UnboundedReceiver<fastclaw_core::channel::InboundMessage>,
    base_skill_registry: Arc<SkillRegistry>,
    stream_event_tx: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>>>,
    ask_question_pending: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    mcp_status_init: std::collections::HashMap<String, fastclaw_core::types::McpServerStatus>,
    mcp_handles_init: std::collections::HashMap<String, fastclaw_collab::mcp::SharedMcpClient>,
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
    context_engine: fastclaw_context::ContextEngine,
    tool_count: usize,
}

struct BuildPhase5 {
    phase2: BuildPhase2Memory,
    dag_checkpoint_store: Arc<dyn CheckpointStore>,
    cron_store: CronJobStore,
    budget_tracker: BudgetTracker,
    model_router: Option<Arc<fastclaw_model_router::ModelRouter>>,
    ws_broadcast: tokio::sync::broadcast::Sender<String>,
}

/// Subsystem-grouped initialization phases for [`AppState`].
///
/// [`AppState::new`] chains [`StateBuilder::phase1_config_session`] → phase 3 → 4 → 2 → 5
/// (phase numbers follow dependency order; phase 2 memory/evolution runs after channels/MCP).
pub(crate) struct StateBuilder;

impl StateBuilder {
    /// Phase 1: config paths, agent list, SQLite session store.
    async fn phase1_config_session(config: &FastClawConfig) -> anyhow::Result<BuildPhase1> {
        fastclaw_core::paths::ensure_state_dir_from(Some(&config.paths))?;
        let agents = load_agents(config)?;
        let agent_count = agents.len();
        let db_path = resolve_db_path(&config.paths)?;
        let session_store = Arc::new(SessionStore::open(&db_path).await?);
        Ok(BuildPhase1 {
            agents,
            agent_count,
            db_path,
            session_store,
        })
    }

    /// Phase 3: LLM runtime, core tools, WASM/plugins, skills, workspaces.
    async fn phase3_agent_runtime_tools(
        config: &FastClawConfig,
        p1: BuildPhase1,
    ) -> anyhow::Result<BuildPhase3> {
        let creds = merge_model_base_urls_into_credentials(&config.credentials, &config.models);
        let runtime = AppState::build_runtime(&p1.agents, &creds)?;
        let router = AgentRouter::new(p1.agents.clone());
        let tool_registry = AppState::build_tools_core(config).await?;

        let paths_cfg = &config.paths;
        let wasm_host = fastclaw_plugin::WasmHost::new(Default::default())?;
        let mut plugin_registry = PluginRegistry::new(wasm_host);
        let plugins_dir = resolve_plugins_dir(paths_cfg);
        if plugins_dir.exists() {
            load_plugins_from_dir(&mut plugin_registry, &plugins_dir);
        }

        let extensions_dir = resolve_extensions_dir(paths_cfg);
        let discovered_plugins = fastclaw_plugin::discover_plugins(&extensions_dir);
        tracing::info!(
            extensions_dir = %extensions_dir.display(),
            count = discovered_plugins.len(),
            "discovered extension plugins"
        );

        let plugin_arc = Arc::new(plugin_registry);
        fastclaw_plugin::bridge::bridge_plugins(&plugin_arc, &tool_registry);
        let bridged_count = plugin_arc.plugin_count();
        if bridged_count > 0 {
            tracing::info!(
                plugins = bridged_count,
                "bridged WASM plugin capabilities into LLM tool registry"
            );
        }
        let plugin_registry = match Arc::try_unwrap(plugin_arc) {
            Ok(r) => r,
            Err(_) => unreachable!("plugin_registry Arc should have no other owners at build time"),
        };

        use fastclaw_core::skill::{load_skills_from_dirs_with_layer, SkillLayer};

        let skills_dir = resolve_skills_dir(paths_cfg);
        let global_skills_dir = fastclaw_core::skill::resolve_global_skills_dir();

        let mut ext_skill_dirs: Vec<PathBuf> = Vec::new();
        for plugin in &discovered_plugins {
            for sd in plugin.skill_dirs() {
                ext_skill_dirs.push(sd);
            }
        }
        let ext_refs: Vec<&Path> = ext_skill_dirs.iter().map(|p| p.as_path()).collect();
        let ext_registry = load_skills_from_dirs_with_layer(&ext_refs, SkillLayer::Extension);
        let project_registry =
            load_skills_from_dirs_with_layer(&[skills_dir.as_path()], SkillLayer::Project);
        let global_registry =
            load_skills_from_dirs_with_layer(&[global_skills_dir.as_path()], SkillLayer::Global);

        let mut base_skill_registry = SkillRegistry::new();
        base_skill_registry.merge_from(ext_registry);
        base_skill_registry.merge_from(project_registry);
        base_skill_registry.merge_from(global_registry);

        tracing::info!(
            base_skills = base_skill_registry.count(),
            skills_dir = %skills_dir.display(),
            global_dir = %global_skills_dir.display(),
            "base skill registry loaded (extension + project + global)"
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
                if let Err(e) = persist_skills_deny_cleanup(&cleaned) {
                    tracing::warn!(error = %e, "failed to persist cleaned skills.deny list to config file");
                }
            }
        }

        let resolved_agents = config.agents.resolved_list();
        let state_dir = resolve_state_dir(paths_cfg);
        let mut workspaces = std::collections::HashMap::new();
        for agent_entry in &resolved_agents {
            let ws_root = if let Some(ref ws) = agent_entry.workspace {
                let p = PathBuf::from(ws);
                if p.is_relative() { state_dir.join(p) } else { p }
            } else {
                fastclaw_core::workspace::resolve_workspace_root(&state_dir, &agent_entry.id, None)
            };
            let workspace = AgentWorkspace::new(&ws_root, &agent_entry.id);
            if let Err(e) = workspace.ensure_bootstrap() {
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
                    if p.is_relative() { state_dir.join(p) } else { p }
                })
                .unwrap_or_else(|| state_dir.join("workspace"));
            let ws = AgentWorkspace::new(&default_root, "main");
            let _ = ws.ensure_bootstrap();
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
            tool_registry,
            plugin_registry,
            base_skill_registry,
            agent_skill_registries,
            workspaces,
        })
    }

    /// Phase 4: MCP + subagent tools, channel plugins, hub + skill tools.
    async fn phase4_channels_mcp(
        config: &FastClawConfig,
        p3: BuildPhase3,
    ) -> anyhow::Result<BuildPhase4> {
        let (mcp_status_init, mcp_handles_init) = AppState::register_mcp_and_subagent_tools(
            &p3.phase1.agents,
            &config.mcp_servers,
            p3.runtime.clone(),
            &p3.tool_registry,
        )
        .await?;

        let (channel_registry, _inbound_tx, inbound_rx) =
            AppState::build_channels(config, &p3.tool_registry).await?;

        let hub_client = fastclaw_core::hub::HubClient::with_defaults();
        let hub = Arc::new(tokio::sync::Mutex::new(hub_client));
        fastclaw_agent::builtin_tools::register_hub_tools(&p3.tool_registry, hub);
        tracing::info!("registered hub_search and hub_install tools");

        let base_skill_registry = Arc::new(p3.base_skill_registry.filtered(
            &config.skills.allow,
            &config.skills.deny,
            None,
        ));
        if matches!(
            config.skills.prompt_mode,
            fastclaw_core::config::SkillPromptMode::Compact
                | fastclaw_core::config::SkillPromptMode::Lazy
        ) {
            if let Some((_, ws)) = p3.workspaces.iter().next() {
                fastclaw_agent::builtin_tools::register_skill_tools_full(
                    &p3.tool_registry,
                    base_skill_registry.clone(),
                    Arc::new(ws.clone()),
                );
                tracing::info!(
                    prompt_mode = ?config.skills.prompt_mode,
                    "registered list_skills / read_skill / write_skill tools"
                );
            } else {
                fastclaw_agent::builtin_tools::register_skill_tools(
                    &p3.tool_registry,
                    base_skill_registry.clone(),
                );
                tracing::info!(
                    prompt_mode = ?config.skills.prompt_mode,
                    "registered list_skills / read_skill tools (no workspace for write_skill)"
                );
            }
        }

        for (agent_id, ws) in &p3.workspaces {
            fastclaw_agent::builtin_tools::register_identity_tools(
                &p3.tool_registry,
                Arc::new(ws.clone()),
            );
            tracing::info!(agent_id = %agent_id, "registered get_identity / set_identity tools");
            break;
        }

        let stream_event_tx = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let ask_question_pending = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        p3.tool_registry.register(Arc::new(
            fastclaw_agent::builtin_tools::AskQuestionTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            ),
        ));
        p3.tool_registry.register(Arc::new(
            fastclaw_agent::builtin_tools::ConfirmTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            ),
        ));
        tracing::info!("registered ask_question + confirm tools");

        Ok(BuildPhase4 {
            phase3: p3,
            channel_registry,
            inbound_rx,
            base_skill_registry,
            stream_event_tx,
            ask_question_pending,
            mcp_status_init,
            mcp_handles_init,
        })
    }

    /// Phase 2: per-agent memory + evolution stores + context engine hooks.
    async fn phase2_memory_evolution(
        config: &FastClawConfig,
        p4: BuildPhase4,
    ) -> anyhow::Result<BuildPhase2Memory> {
        let creds = &config.credentials;
        let (agent_episodic_map, agent_semantic_map, embedding_provider) = AppState::build_memory(
            config,
            creds,
            &p4.phase3.workspaces,
            &p4.phase3.tool_registry,
        )
        .await?;

        let tool_count = p4.phase3.tool_registry.definitions().len();

        let message_bus = Arc::new(MessageBus::new(1024));
        for agent in &p4.phase3.phase1.agents {
            let aid = agent.agent_id.clone();
            let mut rx = message_bus.register(&aid).await;
            tokio::spawn(async move {
                while rx.recv().await.is_some() {}
            });
        }
        fastclaw_agent::builtin_tools::register_session_tools(
            &p4.phase3.tool_registry,
            p4.phase3.phase1.session_store.clone(),
            message_bus.clone(),
        );

        let evo_pool = open_memory_pool_named(&p4.phase3.phase1.db_path, "evolution.db").await?;
        let feedback_store = FeedbackStore::open(evo_pool.clone()).await?;
        let trajectory_store = TrajectoryStore::open(evo_pool.clone()).await?;
        let skill_store = SkillStore::open(evo_pool.clone()).await?;
        let prompt_distiller = PromptDistiller::open(evo_pool).await?;

        let mut context_engine =
            fastclaw_context::ContextEngine::new(fastclaw_context::DEFAULT_COMPACTION_THRESHOLD);
        context_engine.add_hook(Arc::new(fastclaw_context::CompactionHook::new(
            fastclaw_context::CompactionStrategy::default(),
        )));
        context_engine.add_hook(Arc::new(fastclaw_context::SystemReminderHook::default()));
        let mut personality_hook = fastclaw_context::AgentPersonalityHook::new();
        for (agent_id, workspace) in &p4.phase3.workspaces {
            personality_hook.add_agent(agent_id, workspace);
        }
        context_engine.add_hook(Arc::new(personality_hook));
        if config.memory.enabled && !agent_episodic_map.is_empty() {
            context_engine.add_hook(Arc::new(fastclaw_context::AgentMemoryIngestHook::new(
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
            context_engine,
            tool_count,
        })
    }

    /// Phase 5: DAG checkpoints, cron, model router, WebSocket broadcast fanout.
    async fn phase5_cron_dag(
        config: &FastClawConfig,
        p2: BuildPhase2Memory,
    ) -> anyhow::Result<BuildPhase5> {
        let dag_checkpoint_store: Arc<dyn CheckpointStore> = Arc::new(
            fastclaw_dag::SqliteCheckpointStore::open(p2.phase4.phase3.phase1.session_store.pool())
                .await?,
        );

        let cron_pool = open_memory_pool_named(&p2.phase4.phase3.phase1.db_path, "cron.db").await?;
        let cron_store = CronJobStore::open(cron_pool).await?;

        let budget_tracker = BudgetTracker::new(config.model_router.daily_budget);

        let model_router = if config.model_router.enabled {
            let strategy_raw = config.model_router.strategy.as_str();
            let strategy = match strategy_raw {
                "cost_optimized" => fastclaw_model_router::RoutingStrategy::CostOptimized,
                "fallback" => fastclaw_model_router::RoutingStrategy::Fallback,
                "quality_first" | "latency_optimized" => {
                    tracing::warn!(
                        requested = strategy_raw,
                        "model_router.strategy is deprecated (quality/latency ranking needs live metrics); using `fallback`"
                    );
                    fastclaw_model_router::RoutingStrategy::Fallback
                }
                _ => fastclaw_model_router::RoutingStrategy::Fixed,
            };
            let mut router =
                fastclaw_model_router::ModelRouter::new(strategy, budget_tracker.clone());
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
            dag_checkpoint_store,
            cron_store,
            budget_tracker,
            model_router,
            ws_broadcast,
        })
    }

    /// Run all phases in dependency order and produce a ready [`AppState`].
    async fn build(config: FastClawConfig) -> anyhow::Result<AppState> {
        if !config.security.ssrf_allowed_hosts.is_empty() {
            tracing::info!(
                hosts = ?config.security.ssrf_allowed_hosts,
                "SSRF: registering allowed hosts that bypass private-IP checks"
            );
            fastclaw_security::ssrf::set_ssrf_allowed_hosts(
                config.security.ssrf_allowed_hosts.clone(),
            );
        }

        tracing::info!(
            policy = ?config.security.dangerous_ops_policy,
            pattern_count = config.security.dangerous_patterns.len(),
            "Dangerous-ops: initializing policy"
        );
        fastclaw_security::dangerous_ops::set_dangerous_ops_config(
            config.security.dangerous_ops_policy,
            &config.security.dangerous_patterns,
        );

        let p1 = Self::phase1_config_session(&config).await?;
        let p3 = Self::phase3_agent_runtime_tools(&config, p1).await?;
        let p4 = Self::phase4_channels_mcp(&config, p3).await?;
        let p2 = Self::phase2_memory_evolution(&config, p4).await?;
        let p5 = Self::phase5_cron_dag(&config, p2).await?;

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
        p5.phase2
            .phase4
            .phase3
            .runtime
            .attach_evolution_stores(skill_store.clone(), trajectory_store.clone());

        let config_live_val = serde_json::to_value(&config).unwrap_or_default();
        let state = AppState {
            config: Arc::new(config),
            config_live: Arc::new(std::sync::RwLock::new(config_live_val)),
            router: Arc::new(tokio::sync::RwLock::new(p5.phase2.phase4.phase3.router)),
            runtime: p5.phase2.phase4.phase3.runtime,
            tool_registry: Arc::new(p5.phase2.phase4.phase3.tool_registry),
            base_skill_registry: Arc::new(std::sync::RwLock::new(p5.phase2.phase4.base_skill_registry)),
            agent_skill_registries: Arc::new(std::sync::RwLock::new(Arc::new(p5.phase2.phase4.phase3.agent_skill_registries))),
            session_store: p5.phase2.phase4.phase3.phase1.session_store,
            dag_checkpoint_store: p5.dag_checkpoint_store,
            agent_episodic: Arc::new(p5.phase2.agent_episodic_map),
            agent_semantic: Arc::new(p5.phase2.agent_semantic_map),
            embedding_provider: p5.phase2.embedding_provider,
            message_bus: p5.phase2.message_bus,
            feedback_store: Arc::new(p5.phase2.feedback_store),
            prompt_distiller: Arc::new(p5.phase2.prompt_distiller),
            trajectory_store,
            skill_store,
            plugin_registry: Arc::new(tokio::sync::RwLock::new(
                p5.phase2.phase4.phase3.plugin_registry,
            )),
            channel_registry: Arc::new(tokio::sync::RwLock::new(p5.phase2.phase4.channel_registry)),
            context_engine: Arc::new(p5.phase2.context_engine),
            cron_store: Arc::new(p5.cron_store),
            cron_wake: Arc::new(tokio::sync::Notify::new()),
            budget_tracker: Arc::new(p5.budget_tracker),
            model_router: p5.model_router,
            ws_broadcast: p5.ws_broadcast,
            workspaces: Arc::new(p5.phase2.phase4.phase3.workspaces),
            runtime_route_bindings: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            last_good_agents: Arc::new(tokio::sync::RwLock::new(initial_agents)),
            stream_event_tx: p5.phase2.phase4.stream_event_tx,
            ask_question_pending: p5.phase2.phase4.ask_question_pending,
            mcp_status: Arc::new(std::sync::RwLock::new(p5.phase2.phase4.mcp_status_init)),
            mcp_handles: Arc::new(tokio::sync::Mutex::new(p5.phase2.phase4.mcp_handles_init)),
        };

        state.tool_registry.register(Arc::new(crate::mcp_tool::ManageMcpServerTool::new(
            state.config_live.clone(),
            state.mcp_status.clone(),
            state.mcp_handles.clone(),
            state.tool_registry.clone(),
        )));
        tracing::info!("registered manage_mcp_server tool");

        state.tool_registry.register(Arc::new(crate::cron_tool::ManageCronTool::new(
            state.cron_store.clone(),
            state.cron_wake.clone(),
        )));
        tracing::info!("registered manage_cron tool");

        state.spawn_skill_evolution_tasks();

        state.spawn_inbound_dispatcher(inbound_rx);

        let dream_secs = state.config.memory.dreaming_interval_secs;
        if state.config.memory.enabled && dream_secs > 0 && !state.agent_episodic.is_empty() {
            let episodic = state.agent_episodic.clone();
            let semantic = state.agent_semantic.clone();
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
                            };
                            match pipe.run_dream_cycle(DREAM_EPISODE_BATCH).await {
                                Ok(report) => {
                                    if report.episodes_considered > 0 {
                                        tracing::info!(
                                            agent_id = %agent_id,
                                            considered = report.episodes_considered,
                                            marked = report.episodes_marked,
                                            rels = report.relationships_added,
                                            "dream cycle completed"
                                        );
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

        if let Some(ref plugins_path) = state.config.paths.plugins_dir {
            let dir = PathBuf::from(plugins_path);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to create plugins directory for hot-reload watcher"
                );
            } else {
                match fastclaw_plugin::start_watching(state.plugin_registry.clone(), dir.clone()) {
                    Ok(()) => tracing::info!(
                        dir = %dir.display(),
                        "plugin hot-reload watcher started"
                    ),
                    Err(e) => tracing::warn!(
                        dir = %dir.display(),
                        error = %e,
                        "plugin hot-reload watcher failed to start"
                    ),
                }
            }
        }

        if let Some(ttl_hours) = state.config.session.ttl_hours {
            let store = state.session_store.clone();
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

        Ok(state)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<FastClawConfig>,
    /// Mutable JSON snapshot of config, updated by `set_config` IPC so reads
    /// reflect runtime changes without restarting the gateway.
    pub config_live: Arc<std::sync::RwLock<serde_json::Value>>,
    pub router: SharedRouter,
    pub runtime: Arc<AgentRuntime>,
    pub tool_registry: Arc<ToolRegistry>,
    pub base_skill_registry: Arc<std::sync::RwLock<Arc<SkillRegistry>>>,
    pub agent_skill_registries: Arc<std::sync::RwLock<Arc<std::collections::HashMap<String, Arc<SkillRegistry>>>>>,
    pub session_store: Arc<SessionStore>,
    pub dag_checkpoint_store: Arc<dyn CheckpointStore>,
    pub agent_episodic: Arc<std::collections::HashMap<String, Arc<EpisodicMemory>>>,
    pub agent_semantic: Arc<std::collections::HashMap<String, Arc<SemanticMemory>>>,
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub message_bus: Arc<MessageBus>,
    pub feedback_store: Arc<FeedbackStore>,
    pub prompt_distiller: Arc<PromptDistiller>,
    pub trajectory_store: Arc<TrajectoryStore>,
    pub skill_store: Arc<SkillStore>,
    pub plugin_registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
    pub channel_registry: Arc<tokio::sync::RwLock<ChannelRegistry>>,
    pub context_engine: Arc<fastclaw_context::ContextEngine>,
    pub cron_store: Arc<CronJobStore>,
    pub cron_wake: Arc<tokio::sync::Notify>,
    pub budget_tracker: Arc<BudgetTracker>,
    pub model_router: Option<Arc<fastclaw_model_router::ModelRouter>>,
    pub ws_broadcast: tokio::sync::broadcast::Sender<String>,
    pub workspaces: Arc<std::collections::HashMap<String, AgentWorkspace>>,
    /// Runtime-only route rows (API-managed); prepended before config file bindings.
    pub runtime_route_bindings: Arc<tokio::sync::RwLock<Vec<RuntimeRouteBinding>>>,
    /// Snapshot of the last agent list successfully applied to [`Self::router`].
    pub last_good_agents: Arc<tokio::sync::RwLock<Vec<AgentConfig>>>,
    /// Shared slot for the current streaming channel's tx (set before each execute_stream).
    pub stream_event_tx: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>>>,
    /// Pending ask_question requests waiting for user answers.
    pub ask_question_pending: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    /// Runtime status of each MCP server (id -> status).
    pub mcp_status: Arc<std::sync::RwLock<std::collections::HashMap<String, fastclaw_core::types::McpServerStatus>>>,
    /// Handles to running MCP client processes, keyed by server id.
    pub mcp_handles: Arc<tokio::sync::Mutex<std::collections::HashMap<String, fastclaw_collab::mcp::SharedMcpClient>>>,
}

impl AppState {
    /// Hot-reload all MCP servers: compare `config_live` with running handles,
    /// stop removed/changed servers, start new/changed ones, update status map.
    pub async fn reload_mcp_servers(&self) -> anyhow::Result<()> {
        use fastclaw_core::agent_config::McpServerConfig;
        use fastclaw_core::types::{McpServerStatus, McpStatus};

        let desired: Vec<McpServerConfig> = {
            let live = self.config_live.read().map_err(|e| anyhow::anyhow!("{e}"))?;
            let mcp_val = live.get("mcpServers").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::from_value(mcp_val).unwrap_or_default()
        };

        let desired_map: std::collections::HashMap<String, &McpServerConfig> =
            desired.iter().map(|c| (c.id.clone(), c)).collect();

        let mut handles = self.mcp_handles.lock().await;

        let current_ids: std::collections::HashSet<String> = handles.keys().cloned().collect();
        let desired_ids: std::collections::HashSet<String> = desired_map.keys().cloned().collect();

        let to_remove: Vec<String> = current_ids.difference(&desired_ids).cloned().collect();
        for id in &to_remove {
            let prefix = format!("mcp_{}_", id);
            let removed = self.tool_registry.unregister_by_prefix(&prefix);
            tracing::info!(mcp_id = %id, tools_removed = removed, "stopped MCP server (removed from config)");
            handles.remove(id);
        }

        let mut new_status: std::collections::HashMap<String, McpServerStatus> =
            std::collections::HashMap::new();

        for cfg in &desired {
            if cfg.enabled == Some(false) {
                if handles.contains_key(&cfg.id) {
                    let prefix = format!("mcp_{}_", cfg.id);
                    self.tool_registry.unregister_by_prefix(&prefix);
                    handles.remove(&cfg.id);
                    tracing::info!(mcp_id = %cfg.id, "stopped MCP server (disabled)");
                }
                new_status.insert(
                    cfg.id.clone(),
                    McpServerStatus {
                        id: cfg.id.clone(),
                        status: McpStatus::Disabled,
                        error: None,
                        tool_count: 0,
                        connected_at: None,
                    },
                );
                continue;
            }

            if handles.contains_key(&cfg.id) {
                let prefix = format!("mcp_{}_", cfg.id);
                self.tool_registry.unregister_by_prefix(&prefix);
                handles.remove(&cfg.id);
                tracing::info!(mcp_id = %cfg.id, "restarting MCP server");
            }

            let args_ref: Vec<&str> = cfg.args.iter().map(|s| s.as_str()).collect();
            let prefix = format!("mcp_{}_", cfg.id);
            let tool_count_before = self.tool_registry.len();
            match fastclaw_collab::mcp::register_mcp_tools(
                &cfg.command,
                &args_ref,
                &self.tool_registry,
                &prefix,
                &cfg.env,
            )
            .await
            {
                Ok(handle) => {
                    let tool_count = self.tool_registry.len() - tool_count_before;
                    let now = chrono::Utc::now().to_rfc3339();
                    tracing::info!(mcp_id = %cfg.id, tool_count, "MCP server connected (hot reload)");
                    new_status.insert(
                        cfg.id.clone(),
                        McpServerStatus {
                            id: cfg.id.clone(),
                            status: McpStatus::Connected,
                            error: None,
                            tool_count,
                            connected_at: Some(now),
                        },
                    );
                    handles.insert(cfg.id.clone(), handle);
                }
                Err(e) => {
                    tracing::warn!(mcp_id = %cfg.id, error = %e, "MCP server failed to connect (hot reload)");
                    new_status.insert(
                        cfg.id.clone(),
                        McpServerStatus {
                            id: cfg.id.clone(),
                            status: McpStatus::Failed,
                            error: Some(e.to_string()),
                            tool_count: 0,
                            connected_at: None,
                        },
                    );
                }
            }
        }

        for id in &to_remove {
            new_status.remove(id);
        }

        {
            let mut status = self.mcp_status.write().map_err(|e| anyhow::anyhow!("{e}"))?;
            *status = new_status;
        }

        Ok(())
    }

    /// Periodic skill maintenance and background extraction from trajectories.
    fn spawn_skill_evolution_tasks(&self) {
        let skill_store = self.skill_store.clone();
        let maintenance_secs = self.config.evolution.skill_maintenance_interval_secs;
        if maintenance_secs > 0 {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    maintenance_secs.max(1),
                ));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match skill_store.maintenance().await {
                        Ok(rep) => {
                            if rep.promoted > 0 || rep.retired_active > 0 {
                                tracing::info!(
                                    promoted = rep.promoted,
                                    retired_active = rep.retired_active,
                                    "skill store maintenance completed"
                                );
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "skill store maintenance failed"),
                    }
                }
            });
            tracing::info!(
                interval_secs = maintenance_secs,
                "skill maintenance background task started"
            );
        }

        let skill_store_ex = self.skill_store.clone();
        let trajectory_store_ex = self.trajectory_store.clone();
        let extraction_secs = self.config.evolution.skill_extraction_interval_secs;
        if extraction_secs > 0 {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    extraction_secs.max(1),
                ));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match trajectory_store_ex.get_recent_successful_global(200).await {
                        Ok(trajs) if !trajs.is_empty() => {
                            let extractor = SkillExtractor::default();
                            let extracted = extractor.extract_skills(&trajs);
                            tracing::info!(
                                trajectories = trajs.len(),
                                candidates = extracted.len(),
                                "skill extraction pass (rule-based)"
                            );
                            for ext in extracted {
                                let needle = format!("{} {}", ext.name, ext.task_pattern);
                                let similar = match skill_store_ex.find_similar(&needle, 18).await {
                                    Ok(s) => s,
                                    Err(e) => {
                                        tracing::warn!(error = %e, "find_similar during extraction failed");
                                        continue;
                                    }
                                };
                                let duplicate = similar
                                    .iter()
                                    .any(|s| s.task_pattern == ext.task_pattern);
                                if duplicate {
                                    continue;
                                }
                                match skill_store_ex.save_skill(&ext).await {
                                    Ok(()) => tracing::info!(
                                        skill_id = %ext.id,
                                        name = %ext.name,
                                        "saved extracted candidate skill"
                                    ),
                                    Err(e) => tracing::warn!(
                                        skill_id = %ext.id,
                                        error = %e,
                                        "failed to save extracted skill"
                                    ),
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "load trajectories for extraction failed"),
                    }
                }
            });
            tracing::info!(
                interval_secs = extraction_secs,
                "skill extraction background task started"
            );
        }
    }

    /// Default LLM runtime + per-agent provider registration.
    fn build_runtime(
        agents: &[AgentConfig],
        creds: &fastclaw_core::config::CredentialsConfig,
    ) -> anyhow::Result<Arc<AgentRuntime>> {
        let primary_model_config = agents
            .first()
            .map(|a| &a.model)
            .cloned()
            .unwrap_or_default();

        let default_provider: Box<dyn fastclaw_agent::LlmProvider> = match create_provider_chain(
            &primary_model_config,
            Some(creds),
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "create_provider_chain for default agent failed; assembling FallbackProvider from configured credentials"
                );
                Self::build_credentials_fallback_chain(creds)?
            }
        };

        let self_iter = Arc::new(fastclaw_self_iter::SelfIterEngine::diagnosis_only());
        let runtime = Arc::new(
            AgentRuntime::new(Arc::from(default_provider))
                .with_self_iter_engine(self_iter),
        );

        for agent in agents {
            match create_provider_chain(&agent.model, Some(creds)) {
                Ok(p) => {
                    runtime.register_provider(&agent.agent_id, Arc::from(p));
                    tracing::info!(
                        agent_id = %agent.agent_id,
                        provider = %agent.model.provider,
                        "registered per-agent provider"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        agent_id = %agent.agent_id,
                        error = %e,
                        "failed to create agent provider, using default"
                    );
                }
            }
        }

        Ok(runtime)
    }

    /// Register channel plugins and return the registry plus the inbound message pipe.
    async fn build_channels(
        config: &FastClawConfig,
        tool_registry: &ToolRegistry,
    ) -> anyhow::Result<(
        ChannelRegistry,
        tokio::sync::mpsc::UnboundedSender<fastclaw_core::channel::InboundMessage>,
        tokio::sync::mpsc::UnboundedReceiver<fastclaw_core::channel::InboundMessage>,
    )> {
        let mut channel_registry = ChannelRegistry::new();
        let (inbound_tx, inbound_rx) =
            tokio::sync::mpsc::unbounded_channel::<fastclaw_core::channel::InboundMessage>();

        let feishu_config = config.channels.get("feishu").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_feishu::FeishuPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(feishu_plugin) = feishu_config.map(fastclaw_feishu::FeishuPlugin::new) {
            let feishu_plugin = Arc::new(feishu_plugin);
            for t in feishu_plugin.tools() {
                tool_registry.register(t);
            }
            let mode = feishu_plugin.connection_mode().to_string();
            if let Err(e) = feishu_plugin.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Feishu channel plugin");
            }
            channel_registry.register(feishu_plugin);
            tracing::info!(mode, "Feishu channel plugin registered with tools");
        } else {
            tracing::debug!("feishu channel not configured, plugin not loaded");
        }

        let telegram_config = config.channels.get("telegram").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_telegram::TelegramPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(tg) = telegram_config.map(fastclaw_telegram::TelegramPlugin::new) {
            let tg = Arc::new(tg);
            if let Err(e) = tg.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Telegram channel plugin");
            }
            channel_registry.register(tg);
            tracing::info!("Telegram channel plugin registered");
        }

        let discord_config = config.channels.get("discord").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_discord::DiscordPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(dc) = discord_config.map(fastclaw_discord::DiscordPlugin::new) {
            let dc = Arc::new(dc);
            if let Err(e) = dc.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Discord channel plugin");
            }
            channel_registry.register(dc);
            tracing::info!("Discord channel plugin registered");
        }

        let slack_config = config.channels.get("slack").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_slack::SlackPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(sl) = slack_config.map(fastclaw_slack::SlackPlugin::new) {
            let sl = Arc::new(sl);
            if let Err(e) = sl.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Slack channel plugin");
            }
            channel_registry.register(sl);
            tracing::info!("Slack channel plugin registered");
        }

        let whatsapp_config = config.channels.get("whatsapp").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_whatsapp::WhatsAppPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(wa) = whatsapp_config.map(fastclaw_whatsapp::WhatsAppPlugin::new) {
            let wa = Arc::new(wa);
            if let Err(e) = wa.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start WhatsApp channel plugin");
            }
            channel_registry.register(wa);
            tracing::info!("WhatsApp channel plugin registered");
        }

        let matrix_config = config.channels.get("matrix").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_matrix::MatrixPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(mx) = matrix_config.map(fastclaw_matrix::MatrixPlugin::new) {
            let mx = Arc::new(mx);
            if let Err(e) = mx.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Matrix channel plugin");
            }
            channel_registry.register(mx);
            tracing::info!("Matrix channel plugin registered");
        }

        let msteams_config = config.channels.get("msteams").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                fastclaw_msteams::TeamsPluginConfig::from_channel_config(ch)
            }
        });
        if let Some(mt) = msteams_config.map(fastclaw_msteams::TeamsPlugin::new) {
            let mt = Arc::new(mt);
            if let Err(e) = mt.start(inbound_tx.clone()).await {
                tracing::error!(error = %e, "failed to start Microsoft Teams channel plugin");
            }
            channel_registry.register(mt);
            tracing::info!("Microsoft Teams channel plugin registered");
        }

        Ok((channel_registry, inbound_tx, inbound_rx))
    }

    /// Built-ins and web/media tools (no MCP, no subagent).
    async fn build_tools_core(config: &FastClawConfig) -> anyhow::Result<ToolRegistry> {
        let creds = &config.credentials;
        let tool_registry = ToolRegistry::new();
        fastclaw_agent::builtin_tools::register_builtin_tools(&tool_registry);

        let ws_cfg = &config.web_search;
        let search_backend = match ws_cfg.backend.as_str() {
            "tavily" => {
                let key = ws_cfg
                    .api_key
                    .clone()
                    .or_else(|| creds.get_api_key("tavily").map(String::from))
                    .unwrap_or_default();
                if key.is_empty() {
                    tracing::warn!(
                        "tavily backend selected but no API key — web_search will be unavailable until configured"
                    );
                    None
                } else {
                    Some(fastclaw_agent::WebSearchBackend::Tavily { api_key: key })
                }
            }
            "searxng" => {
                let base = ws_cfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8888".to_string());
                Some(fastclaw_agent::WebSearchBackend::SearXNG { base_url: base })
            }
            "builtin" => {
                let engine_ids = ws_cfg.engines.clone().unwrap_or_else(|| {
                    fastclaw_agent::BUILTIN_ENGINE_IDS.iter().map(|s| s.to_string()).collect()
                });
                tracing::info!(engines = ?engine_ids, "using built-in meta search engine");
                Some(fastclaw_agent::WebSearchBackend::Builtin { engines: engine_ids })
            }
            _ => {
                tracing::info!("web_search backend not configured — web_search tool will prompt user to configure");
                None
            }
        };
        if let Some(backend) = search_backend {
            fastclaw_agent::builtin_tools::register_web_tools(&tool_registry, backend);
        }

        fastclaw_agent::builtin_tools::register_browser_tool(&tool_registry);
        tracing::info!("registered browser tool (headless Chrome)");

        if let Some(openai_key) = creds.get_api_key("openai") {
            let openai_base = creds
                .get_base_url("openai")
                .unwrap_or("https://api.openai.com/v1");
            fastclaw_agent::builtin_tools::register_media_tools(
                &tool_registry,
                openai_base,
                openai_key,
            );
            tracing::info!("registered image_generate and text_to_speech tools");
        }

        Ok(tool_registry)
    }

    /// MCP servers and the subagent tool (must run after [`Self::build_tools_core`]).
    ///
    /// Global MCP servers are registered first; per-agent servers with the same `id`
    /// are skipped to avoid duplicate subprocesses.
    ///
    /// Returns `(status_map, handles_map)` for populating `AppState` fields.
    async fn register_mcp_and_subagent_tools(
        agents: &[AgentConfig],
        global_mcp: &[fastclaw_core::agent_config::McpServerConfig],
        runtime: Arc<AgentRuntime>,
        tool_registry: &ToolRegistry,
    ) -> anyhow::Result<(
        std::collections::HashMap<String, fastclaw_core::types::McpServerStatus>,
        std::collections::HashMap<String, fastclaw_collab::mcp::SharedMcpClient>,
    )> {
        use fastclaw_core::types::{McpServerStatus, McpStatus};

        let mut status_map: std::collections::HashMap<String, McpServerStatus> =
            std::collections::HashMap::new();
        let mut handles_map: std::collections::HashMap<String, fastclaw_collab::mcp::SharedMcpClient> =
            std::collections::HashMap::new();
        let mut registered_ids = std::collections::HashSet::new();

        tracing::info!(
            global_count = global_mcp.len(),
            agent_count = agents.len(),
            global_ids = ?global_mcp.iter().map(|c| &c.id).collect::<Vec<_>>(),
            "register_mcp_and_subagent_tools: starting"
        );

        let all_mcp_configs: Vec<(&fastclaw_core::agent_config::McpServerConfig, &str)> = global_mcp
            .iter()
            .map(|c| (c, "global"))
            .chain(agents.iter().flat_map(|a| {
                a.mcp_servers.iter().map(move |c| (c, a.agent_id.as_str()))
            }))
            .collect();

        for (mcp_cfg, scope) in &all_mcp_configs {
            if mcp_cfg.enabled == Some(false) {
                status_map.insert(
                    mcp_cfg.id.clone(),
                    McpServerStatus {
                        id: mcp_cfg.id.clone(),
                        status: McpStatus::Disabled,
                        error: None,
                        tool_count: 0,
                        connected_at: None,
                    },
                );
                continue;
            }
            if registered_ids.contains(&mcp_cfg.id) {
                tracing::debug!(
                    mcp_id = %mcp_cfg.id,
                    scope = %scope,
                    "skipping MCP server already registered"
                );
                continue;
            }
            let args_ref: Vec<&str> = mcp_cfg.args.iter().map(|s| s.as_str()).collect();
            let prefix = format!("mcp_{}_", mcp_cfg.id);
            let tool_count_before = tool_registry.len();
            match fastclaw_collab::mcp::register_mcp_tools(
                &mcp_cfg.command,
                &args_ref,
                tool_registry,
                &prefix,
                &mcp_cfg.env,
            )
            .await
            {
                Ok(handle) => {
                    let tool_count = tool_registry.len() - tool_count_before;
                    tracing::info!(
                        mcp_id = %mcp_cfg.id,
                        scope = %scope,
                        tool_count,
                        "MCP client connected"
                    );
                    registered_ids.insert(mcp_cfg.id.clone());
                    let now = chrono::Utc::now().to_rfc3339();
                    status_map.insert(
                        mcp_cfg.id.clone(),
                        McpServerStatus {
                            id: mcp_cfg.id.clone(),
                            status: McpStatus::Connected,
                            error: None,
                            tool_count,
                            connected_at: Some(now),
                        },
                    );
                    handles_map.insert(mcp_cfg.id.clone(), handle);
                }
                Err(e) => {
                    tracing::warn!(
                        mcp_id = %mcp_cfg.id,
                        scope = %scope,
                        error = %e,
                        "failed to connect MCP server, skipping"
                    );
                    status_map.insert(
                        mcp_cfg.id.clone(),
                        McpServerStatus {
                            id: mcp_cfg.id.clone(),
                            status: McpStatus::Failed,
                            error: Some(e.to_string()),
                            tool_count: 0,
                            connected_at: None,
                        },
                    );
                }
            }
        }

        if !registered_ids.is_empty() {
            tracing::info!(
                count = registered_ids.len(),
                ids = ?registered_ids,
                "MCP servers registered"
            );
        }

        let subagent_tool = fastclaw_agent::SubAgentTool::new(
            runtime.clone(),
            Arc::new(tool_registry.clone()),
            agents.to_vec(),
        );
        tool_registry.register(Arc::new(subagent_tool));

        Ok((status_map, handles_map))
    }

    /// Per-workspace SQLite memory, optional embeddings, and memory tools on `tool_registry`.
    async fn build_memory(
        config: &FastClawConfig,
        creds: &fastclaw_core::config::CredentialsConfig,
        workspaces: &std::collections::HashMap<String, AgentWorkspace>,
        tool_registry: &ToolRegistry,
    ) -> anyhow::Result<(
        std::collections::HashMap<String, Arc<EpisodicMemory>>,
        std::collections::HashMap<String, Arc<SemanticMemory>>,
        Option<Arc<dyn EmbeddingProvider>>,
    )> {
        let mut agent_episodic_map = std::collections::HashMap::new();
        let mut agent_semantic_map = std::collections::HashMap::new();

        if !config.memory.enabled {
            tracing::info!("memory system disabled (config.memory.enabled = false)");
            return Ok((agent_episodic_map, agent_semantic_map, None));
        }

        for (agent_id, workspace) in workspaces {
            let agent_db = workspace.root.join("memory.db");
            if let Some(parent) = agent_db.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let pool = open_memory_pool_at(&agent_db).await?;
            let ep = EpisodicMemory::open(pool.clone()).await?;
            let sem = SemanticMemory::open(pool).await?;
            tracing::debug!(agent_id = %agent_id, db = %agent_db.display(), "agent memory initialized");
            agent_episodic_map.insert(agent_id.clone(), Arc::new(ep));
            agent_semantic_map.insert(agent_id.clone(), Arc::new(sem));
        }

        let embedding_provider =
            match fastclaw_memory::create_embedding_provider(&config.memory.embedding, Some(creds))
                .await
            {
                Ok(ep) => {
                    tracing::info!(
                        provider = ep.name(),
                        dims = ep.dimensions(),
                        "embedding provider initialized"
                    );
                    Some(Arc::from(ep))
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "embedding provider unavailable, memory will use keyword-only search"
                    );
                    None
                }
            };

        let multi_agent_memory = agent_episodic_map.len() > 1;
        for agent_id in agent_episodic_map.keys() {
            if let (Some(ep), Some(sem)) = (
                agent_episodic_map.get(agent_id),
                agent_semantic_map.get(agent_id),
            ) {
                let search_inner = Arc::new(fastclaw_agent::MemorySearchTool::new(
                    ep.clone(),
                    sem.clone(),
                    embedding_provider.clone(),
                ));
                let store_inner = Arc::new(fastclaw_agent::MemoryStoreTool::new(
                    ep.clone(),
                    sem.clone(),
                    embedding_provider.clone(),
                    agent_id.clone(),
                ));
                if multi_agent_memory {
                    let sfx = memory_tool_agent_suffix(agent_id);
                    let search_name = format!("memory_search__{sfx}");
                    let store_name = format!("memory_store__{sfx}");
                    let search_desc =
                        format!("{} (agent `{}`)", search_inner.description(), agent_id);
                    let store_desc =
                        format!("{} (agent `{}`)", store_inner.description(), agent_id);
                    tool_registry.register(Arc::new(RenamedTool::new(
                        search_name,
                        search_desc,
                        search_inner.clone() as Arc<dyn fastclaw_core::tool::Tool + Send + Sync>,
                    )));
                    tool_registry.register(Arc::new(RenamedTool::new(
                        store_name,
                        store_desc,
                        store_inner.clone() as Arc<dyn fastclaw_core::tool::Tool + Send + Sync>,
                    )));
                    tracing::info!(agent_id = %agent_id, "registered scoped memory_search / memory_store tools");
                } else {
                    tool_registry.register(search_inner);
                    tool_registry.register(store_inner);
                    tracing::info!(agent_id = %agent_id, "registered memory_search and memory_store tools");
                }
            }
        }

        tracing::info!("memory system enabled");
        Ok((agent_episodic_map, agent_semantic_map, embedding_provider))
    }

    /// When the primary agent model chain cannot be built, try every provider that has an API key
    /// and wrap them in a [`FallbackProvider`] (same order as sorted credential keys).
    fn build_credentials_fallback_chain(
        creds: &fastclaw_core::config::CredentialsConfig,
    ) -> anyhow::Result<Box<dyn fastclaw_agent::LlmProvider>> {
        let mut keys: Vec<String> = creds.providers.keys().cloned().collect();
        keys.sort();
        let mut chain: Vec<(String, Box<dyn fastclaw_agent::LlmProvider>)> = Vec::new();
        for key in keys {
            if creds
                .get_api_key(&key)
                .map(|k| !k.is_empty())
                .unwrap_or(false)
            {
                match create_provider_with_credentials(&key, None, None, Some(creds), None) {
                    Ok(p) => chain.push((key.clone(), p)),
                    Err(e) => {
                        tracing::warn!(provider = %key, error = %e, "skip provider in fallback chain")
                    }
                }
            }
        }
        if chain.is_empty() {
            create_provider("openai", None, None)
        } else {
            tracing::info!(
                providers = chain.len(),
                "using FallbackProvider from credentials"
            );
            Ok(Box::new(FallbackProvider::new(chain)))
        }
    }

    /// Full production wiring: chains [`StateBuilder`] phases in dependency order.
    pub async fn new(config: FastClawConfig) -> anyhow::Result<Self> {
        StateBuilder::build(config).await
    }

    fn spawn_inbound_dispatcher(
        &self,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<fastclaw_core::channel::InboundMessage>,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let channel_id = msg.channel_id.clone();
                let chat_id = msg.chat_id.clone();
                let message_id = msg.message_id.clone();
                let text = msg.text.clone();

                let registry = state.channel_registry.read().await;
                let channel = match registry.get(&channel_id) {
                    Some(ch) => ch.clone(),
                    None => {
                        tracing::warn!(channel = %channel_id, "inbound message for unknown channel");
                        continue;
                    }
                };
                drop(registry);

                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::routes::handle_channel_message(
                        state_clone,
                        channel,
                        &channel_id,
                        &chat_id,
                        &message_id,
                        &text,
                    )
                    .await
                    {
                        tracing::error!(
                            error = %e,
                            channel = %channel_id,
                            chat_id = %chat_id,
                            "inbound ws message handling failed"
                        );
                    }
                });
            }
            tracing::info!("inbound channel dispatcher stopped");
        });
    }

    /// Get the skill registry for a specific agent, falling back to base.
    pub fn skill_registry_for(&self, agent_id: &str) -> Arc<SkillRegistry> {
        let registries = self.agent_skill_registries.read().unwrap();
        registries
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| self.base_skill_registry.read().unwrap().clone())
    }

    /// Rescan skill directories from disk and rebuild all registries.
    pub fn reload_skills(&self) -> anyhow::Result<usize> {
        use fastclaw_core::skill::{load_skills_from_dirs_with_layer, SkillLayer};

        let paths_cfg = &self.config.paths;
        let skills_dir = resolve_skills_dir(paths_cfg);
        let global_skills_dir = fastclaw_core::skill::resolve_global_skills_dir();

        let project_registry =
            load_skills_from_dirs_with_layer(&[skills_dir.as_path()], SkillLayer::Project);
        let global_registry =
            load_skills_from_dirs_with_layer(&[global_skills_dir.as_path()], SkillLayer::Global);

        let mut base = SkillRegistry::new();
        base.merge_from(project_registry);
        base.merge_from(global_registry);

        let filtered_base = Arc::new(base.filtered(
            &self.config.skills.allow,
            &self.config.skills.deny,
            None,
        ));

        let resolved_agents = self.config.agents.resolved_list();
        let mut per_agent = std::collections::HashMap::new();
        let workspaces = self.workspaces.clone();
        for (agent_id, workspace) in workspaces.iter() {
            let agent_ws_skills_dir = workspace.skills_dir();
            let mut agent_reg: SkillRegistry = (*filtered_base).clone();
            if agent_ws_skills_dir.exists() {
                let ws_skills = load_skills_from_dirs_with_layer(
                    &[agent_ws_skills_dir.as_path()],
                    SkillLayer::AgentWorkspace,
                );
                agent_reg.merge_from(ws_skills);
            }
            let agent_allow = resolved_agents
                .iter()
                .find(|a| a.id == *agent_id)
                .and_then(|a| a.skills.as_deref());
            agent_reg = agent_reg.filtered(
                &self.config.skills.allow,
                &self.config.skills.deny,
                agent_allow,
            );
            per_agent.insert(agent_id.clone(), Arc::new(agent_reg));
        }

        let total = filtered_base.count();
        {
            let mut lock = self.base_skill_registry.write().unwrap();
            *lock = filtered_base;
        }
        {
            let mut lock = self.agent_skill_registries.write().unwrap();
            *lock = Arc::new(per_agent);
        }
        tracing::info!(base_skills = total, "skills hot-reloaded from disk");
        Ok(total)
    }

    /// Hot-reload agent configs from disk. Returns the number of agents loaded.
    /// Effective channel bindings: ephemeral API routes first, then config file rows.
    pub async fn merged_route_bindings(&self) -> Vec<fastclaw_core::config::BindingConfig> {
        let rt = self.runtime_route_bindings.read().await;
        fastclaw_core::routing::merge_runtime_bindings_first(&rt, &self.config.bindings)
    }

    /// Hot-reload agent configs from disk. Validates before swapping the router so a bad
    /// config never leaves the gateway in a partially-updated state.
    pub async fn reload_agents(&self) -> anyhow::Result<usize> {
        let agents = load_agents(&self.config)?;
        self.apply_validated_agent_reload(agents).await
    }

    /// Apply a candidate agent list: validate, then swap [`Self::router`] and refresh
    /// [`Self::last_good_agents`] in one logical step (router swap is a single write).
    pub async fn apply_validated_agent_reload(
        &self,
        agents: Vec<AgentConfig>,
    ) -> anyhow::Result<usize> {
        validate_agents_for_reload(&agents)?;
        self.refresh_runtime_agent_providers(&agents);
        let count = agents.len();
        let new_router = AgentRouter::new(agents.clone());
        {
            let mut router = self.router.write().await;
            *router = new_router;
        }
        *self.last_good_agents.write().await = agents;
        tracing::info!(agent_count = count, "agents hot-reloaded");
        Ok(count)
    }

    fn refresh_runtime_agent_providers(&self, agents: &[AgentConfig]) {
        self.runtime.clear_registered_providers();
        let credentials = self.current_credentials_snapshot();

        let mut registered = 0usize;
        let mut failed = 0usize;
        for agent in agents {
            match create_provider_chain(&agent.model, Some(&credentials)) {
                Ok(provider) => {
                    self.runtime
                        .register_provider(&agent.agent_id, Arc::from(provider));
                    registered += 1;
                }
                Err(e) => {
                    failed += 1;
                    tracing::warn!(
                        agent_id = %agent.agent_id,
                        error = %e,
                        "agent hot-reload: failed to refresh provider, default provider will be used"
                    );
                }
            }
        }

        tracing::info!(
            registered,
            failed,
            "agent hot-reload: refreshed runtime provider map"
        );
    }

    fn current_credentials_snapshot(&self) -> fastclaw_core::config::CredentialsConfig {
        let live = self.config_live.read().ok();
        let credentials = live
            .as_ref()
            .and_then(|cfg| cfg.get("credentials").cloned())
            .and_then(|v| serde_json::from_value::<fastclaw_core::config::CredentialsConfig>(v).ok())
            .unwrap_or_else(|| self.config.credentials.clone());

        let models_value = live
            .as_ref()
            .and_then(|cfg| cfg.get("models").cloned())
            .unwrap_or_else(|| serde_json::to_value(&self.config.models).unwrap_or_default());
        let models: std::collections::HashMap<String, fastclaw_core::config::ModelProviderConfig> =
            serde_json::from_value(models_value).unwrap_or_default();

        merge_model_base_urls_into_credentials(&credentials, &models)
    }
}

/// Merge `models.<key>.baseUrl` into `credentials.<key>.baseUrl` when the
/// credential entry is missing or has an empty `base_url`.  This ensures the
/// runtime LLM provider uses the endpoint shown in settings even when the user
/// only filled in `baseUrl` on the model form.
fn merge_model_base_urls_into_credentials(
    credentials: &fastclaw_core::config::CredentialsConfig,
    models: &std::collections::HashMap<String, fastclaw_core::config::ModelProviderConfig>,
) -> fastclaw_core::config::CredentialsConfig {
    let mut merged = credentials.clone();

    for (key, model_cfg) in models {
        let base_url = model_cfg
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(base_url) = base_url else {
            continue;
        };

        let entry = merged.providers.entry(key.clone()).or_default();
        let has_base = entry
            .base_url
            .as_deref()
            .map(str::trim)
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_base {
            entry.base_url = Some(base_url.to_string());
        }
    }

    merged
}

async fn open_memory_pool_at(db_path: &std::path::Path) -> anyhow::Result<sqlx::SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await?;
    Ok(pool)
}

async fn open_memory_pool_named(
    db_path: &std::path::Path,
    name: &str,
) -> anyhow::Result<sqlx::SqlitePool> {
    let target_db = db_path.with_file_name(name);
    if let Some(parent) = target_db.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options = SqliteConnectOptions::new()
        .filename(&target_db)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    Ok(pool)
}

fn resolve_db_path(paths_cfg: &fastclaw_core::config::PathsConfig) -> anyhow::Result<PathBuf> {
    Ok(fastclaw_core::paths::resolve_db_path_from(Some(paths_cfg)))
}

fn load_agents(config: &FastClawConfig) -> anyhow::Result<Vec<AgentConfig>> {
    let config_dir = resolve_agents_dir(&config.paths);
    if config_dir.exists() {
        let agents = agent_config::load_agent_configs(&config_dir)?;
        if agents.is_empty() {
            tracing::warn!(
                dir = %config_dir.display(),
                "agents config directory is empty, using built-in default"
            );
            Ok(vec![builtin_default_agent(config)])
        } else {
            Ok(agents)
        }
    } else {
        tracing::warn!(dir = %config_dir.display(), "agents config directory not found, using built-in default");
        Ok(vec![builtin_default_agent(config)])
    }
}

fn resolve_agents_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_agents_dir_from(Some(paths_cfg))
}

fn builtin_default_agent(config: &FastClawConfig) -> AgentConfig {
    AgentConfig {
        agent_id: "main".to_string(),
        name: Some("Main Agent".to_string()),
        description: Some("Built-in default assistant".to_string()),
        model: builtin_default_model(config),
        // Omit system_prompt so `AgentRuntime::build_messages` uses
        // `fastclaw_core::workspace::default_runtime_system_prompt_for_agent`.
        system_prompt: None,
        tools: Vec::new(),
        behavior: Default::default(),
        mcp_servers: Vec::new(),
        min_tier: None,
        max_tier: None,
        avatar: None,
        channels: std::collections::HashMap::new(),
    }
}

fn builtin_default_model(config: &FastClawConfig) -> AgentModelConfig {
    let mut model = AgentModelConfig::default();
    let default_ref = config
        .agents
        .defaults
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(model_ref) = default_ref {
        if let Some((provider, model_name)) = model_ref.split_once('/') {
            let provider = provider.trim();
            let model_name = model_name.trim();
            if !provider.is_empty() && !model_name.is_empty() {
                model.provider = provider.to_string();
                model.model = model_name.to_string();
                return model;
            }
        }
        model.model = model_ref.to_string();
        if let Some((provider_key, _)) = config.models.iter().find(|(_, cfg)| cfg.model == model.model) {
            model.provider = provider_key.clone();
        }
    }
    model
}

fn resolve_plugins_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_plugins_dir_from(Some(paths_cfg))
}

fn resolve_extensions_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_extensions_dir_from(Some(paths_cfg))
}

fn resolve_skills_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_skills_dir_from(Some(paths_cfg))
}

fn resolve_state_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_state_dir_from(Some(paths_cfg))
}

fn load_plugins_from_dir(plugin_registry: &mut PluginRegistry, dir: &std::path::Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read plugins directory");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("manifest.json").exists() || !path.join("plugin.wasm").exists() {
            continue;
        }
        match plugin_registry.load_from_dir(&path) {
            Ok(manifest) => {
                tracing::info!(plugin_id = %manifest.id, caps = manifest.capabilities.len(), "loaded WASM plugin");
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to load plugin");
            }
        }
    }
}

#[cfg(any(test, feature = "test-helpers"))]
impl AppState {
    /// Construct a minimal `AppState` backed by temp-dir SQLite databases.
    ///
    /// Skips channels, MCP, plugins, and multi-phase production wiring ([`StateBuilder::build`]).
    /// `provider` is caller-supplied so tests can inject a mock LLM.
    pub async fn for_test(
        provider: Box<dyn fastclaw_agent::LlmProvider>,
        tmp: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let mut config = FastClawConfig::default();
        config.memory.enabled = true;
        let agents = vec![builtin_default_agent(&config)];
        let last_good_agents_init = agents.clone();
        let router = AgentRouter::new(agents.clone());

        let evo_pool = {
            let target = tmp.join("evolution.db");
            let opts = SqliteConnectOptions::new()
                .filename(&target)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(opts)
                .await?
        };
        let feedback_store = FeedbackStore::open(evo_pool.clone()).await?;
        let trajectory_store = Arc::new(TrajectoryStore::open(evo_pool.clone()).await?);
        let skill_store = Arc::new(SkillStore::open(evo_pool.clone()).await?);
        let prompt_distiller = PromptDistiller::open(evo_pool).await?;

        let runtime = Arc::new(
            AgentRuntime::new(Arc::from(provider))
                .with_skill_store(skill_store.clone())
                .with_trajectory_store(trajectory_store.clone()),
        );

        let tool_registry = ToolRegistry::new();
        fastclaw_agent::builtin_tools::register_builtin_tools(&tool_registry);

        let subagent_tool = fastclaw_agent::SubAgentTool::new(
            runtime.clone(),
            Arc::new(tool_registry.clone()),
            agents,
        );
        tool_registry.register(Arc::new(subagent_tool));

        let db_path = tmp.join("sessions.db");
        let session_store = Arc::new(SessionStore::open(&db_path).await?);
        let dag_checkpoint_store: Arc<dyn CheckpointStore> =
            Arc::new(fastclaw_dag::SqliteCheckpointStore::open(session_store.pool()).await?);
        let message_bus = Arc::new(MessageBus::new(128));
        for aid in ["main"] {
            let mut rx = message_bus.register(aid).await;
            tokio::spawn(async move {
                while rx.recv().await.is_some() {}
            });
        }
        fastclaw_agent::builtin_tools::register_session_tools(
            &tool_registry,
            session_store.clone(),
            message_bus.clone(),
        );

        let mem_pool = {
            let target = tmp.join("memory.db");
            let opts = SqliteConnectOptions::new()
                .filename(&target)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(opts)
                .await?
        };
        let test_ep = EpisodicMemory::open(mem_pool.clone()).await?;
        let test_sem = SemanticMemory::open(mem_pool).await?;
        let mut test_ep_map = std::collections::HashMap::new();
        let mut test_sem_map = std::collections::HashMap::new();
        test_ep_map.insert("main".to_string(), Arc::new(test_ep));
        test_sem_map.insert("main".to_string(), Arc::new(test_sem));

        let wasm_host = fastclaw_plugin::WasmHost::new(Default::default())?;
        let plugin_registry = PluginRegistry::new(wasm_host);
        let channel_registry = ChannelRegistry::new();

        let mut context_engine =
            fastclaw_context::ContextEngine::new(fastclaw_context::DEFAULT_COMPACTION_THRESHOLD);
        context_engine.add_hook(Arc::new(fastclaw_context::CompactionHook::new(
            fastclaw_context::CompactionStrategy::default(),
        )));
        context_engine.add_hook(Arc::new(fastclaw_context::SystemReminderHook::default()));

        let cron_pool = {
            let target = tmp.join("cron.db");
            let opts = SqliteConnectOptions::new()
                .filename(&target)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(opts)
                .await?
        };
        let cron_store = CronJobStore::open(cron_pool).await?;

        let budget_tracker = BudgetTracker::new(None);

        let (ws_broadcast, _) = tokio::sync::broadcast::channel::<String>(256);

        let config_live_val = serde_json::to_value(&config).unwrap_or_default();
        Ok(Self {
            config: Arc::new(config),
            config_live: Arc::new(std::sync::RwLock::new(config_live_val)),
            router: Arc::new(tokio::sync::RwLock::new(router)),
            runtime,
            tool_registry: Arc::new(tool_registry),
            base_skill_registry: Arc::new(std::sync::RwLock::new(Arc::new(SkillRegistry::new()))),
            agent_skill_registries: Arc::new(std::sync::RwLock::new(Arc::new(std::collections::HashMap::new()))),
            session_store,
            dag_checkpoint_store,
            agent_episodic: Arc::new(test_ep_map),
            agent_semantic: Arc::new(test_sem_map),
            embedding_provider: None,
            message_bus,
            feedback_store: Arc::new(feedback_store),
            prompt_distiller: Arc::new(prompt_distiller),
            trajectory_store,
            skill_store,
            plugin_registry: Arc::new(tokio::sync::RwLock::new(plugin_registry)),
            channel_registry: Arc::new(tokio::sync::RwLock::new(channel_registry)),
            context_engine: Arc::new(context_engine),
            cron_store: Arc::new(cron_store),
            cron_wake: Arc::new(tokio::sync::Notify::new()),
            budget_tracker: Arc::new(budget_tracker),
            model_router: None,
            ws_broadcast,
            workspaces: Arc::new(std::collections::HashMap::new()),
            runtime_route_bindings: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            last_good_agents: Arc::new(tokio::sync::RwLock::new(last_good_agents_init)),
            stream_event_tx: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            ask_question_pending: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            mcp_status: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            mcp_handles: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        })
    }
}

/// Persist cleaned skills deny list to user config file.
/// Called during initialization to remove stale skill IDs that no longer exist on disk.
fn persist_skills_deny_cleanup(cleaned_deny: &[String]) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let cfg_path = home.join(".fastclaw/config/default.json");
    let mut cfg_value: serde_json::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        json5::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let skills_obj = cfg_value
        .as_object_mut()
        .and_then(|root| {
            root.entry("skills")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
        });

    if let Some(skills) = skills_obj {
        skills.insert(
            "deny".to_string(),
            serde_json::to_value(cleaned_deny)?,
        );
    }

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg_value)?)?;
    tracing::info!(path = %cfg_path.display(), "persisted cleaned skills.deny list");
    Ok(())
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    use fastclaw_agent::{CompletionParams, LlmProvider};
    use fastclaw_core::config::FastClawConfig;
    use fastclaw_core::types::{ChatChoice, ChatMessage, ChatRequest, ChatResponse, Role, StreamDelta};

    struct StubProvider;

    #[async_trait::async_trait]
    impl LlmProvider for StubProvider {
        async fn chat_completion(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                id: "stub".into(),
                object: "chat.completion".into(),
                created: 0,
                model: "stub".into(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage {
                        role: Role::Assistant,
                        content: Some("ok".into()),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    finish_reason: Some("stop".into()),
                }],
                usage: None,
            })
        }

        async fn chat_completion_stream(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[test]
    fn validate_rejects_empty_agent_list() {
        assert!(validate_agents_for_reload(&[]).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let a = builtin_default_agent(&FastClawConfig::default());
        let mut b = a.clone();
        b.name = Some("other".into());
        assert!(validate_agents_for_reload(&[a, b]).is_err());
    }

    #[test]
    fn load_agents_uses_builtin_default_when_agents_dir_empty() {
        let tmp = std::env::temp_dir().join(format!(
            "fcgw_load_agents_empty_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut cfg = FastClawConfig::default();
        cfg.paths.agents_dir = Some(tmp.to_string_lossy().to_string());

        let agents = load_agents(&cfg).expect("load_agents should succeed");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_id, "main");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_agents_builtin_default_inherits_agents_default_model() {
        let tmp = std::env::temp_dir().join(format!(
            "fcgw_load_agents_default_model_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut cfg = FastClawConfig::default();
        cfg.paths.agents_dir = Some(tmp.to_string_lossy().to_string());
        cfg.agents.defaults.model = Some("dashscope/qwen3.5-plus".to_string());

        let agents = load_agents(&cfg).expect("load_agents should succeed");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].model.provider, "dashscope");
        assert_eq!(agents[0].model.model, "qwen3.5-plus");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn reload_with_bad_config_keeps_previous_router() {
        let tmp = std::env::temp_dir().join(format!(
            "fcgw_reload_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let provider: Box<dyn LlmProvider> = Box::new(StubProvider);
        let state = AppState::for_test(provider, &tmp).await.unwrap();

        let req = ChatRequest {
            agent_id: Some("main".into()),
            session_id: None,
            messages: vec![],
            model: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            slash_intent: None,
            work_dir: None,
        };
        assert!(state.router.read().await.resolve(&req).is_ok());

        let err = state
            .apply_validated_agent_reload(vec![])
            .await
            .expect_err("empty reload should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("reload rejected") || msg.contains("no agents"),
            "unexpected: {msg}"
        );

        assert!(
            state.router.read().await.resolve(&req).is_ok(),
            "router must still resolve after failed reload"
        );
    }
}
