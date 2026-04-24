use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use fastclaw_agent::AgentRuntime;
use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::bus::MessageBus;
use fastclaw_core::channel::ChannelRegistry;
use fastclaw_core::config::FastClawConfig;
use fastclaw_core::skill::SkillRegistry;
use fastclaw_core::tool::{Tool, ToolRegistry};
use fastclaw_core::workspace::AgentWorkspace;
use fastclaw_core::Router as AgentRouter;
use fastclaw_cron::CronJobStore;
use fastclaw_dag::CheckpointStore;
use fastclaw_evolution::{
    FeedbackStore, PromptDistiller, SkillStore, TrajectoryStore,
};
use fastclaw_memory::{DreamingPipeline, EmbeddingProvider, EpisodicMemory, SemanticMemory};
use fastclaw_model_router::BudgetTracker;
use fastclaw_plugin::PluginRegistry;
use fastclaw_session::SessionStore;

use crate::memory_scope::memory_tool_agent_suffix;
use crate::scoped_tool::RenamedTool;

use super::helpers;
use super::AppState;

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
    channel_inbound_tx: tokio::sync::mpsc::UnboundedSender<fastclaw_core::channel::InboundMessage>,
    inbound_rx: tokio::sync::mpsc::UnboundedReceiver<fastclaw_core::channel::InboundMessage>,
    base_skill_registry: Arc<SkillRegistry>,
    stream_event_tx: Arc<
        DashMap<String, tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>,
    >,
    ask_question_pending: Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>,
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
    notification_store: crate::notification_store::NotificationStore,
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
        let agents = helpers::load_agents(config)?;
        let agent_count = agents.len();
        let db_path = helpers::resolve_db_path(&config.paths)?;
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
        let creds = helpers::merge_model_base_urls_into_credentials(&config.credentials, &config.models);
        let runtime = super::AppState::build_runtime(&p1.agents, &creds)?;
        let router = AgentRouter::new(p1.agents.clone());
        let tool_registry = super::AppState::build_tools_core(config).await?;

        let paths_cfg = &config.paths;
        let wasm_host = fastclaw_plugin::WasmHost::new(Default::default())?;
        let mut plugin_registry = PluginRegistry::new(wasm_host);
        let plugins_dir = helpers::resolve_plugins_dir(paths_cfg);
        if plugins_dir.exists() {
            helpers::load_plugins_from_dir(&mut plugin_registry, &plugins_dir);
        }

        let extensions_dir = helpers::resolve_extensions_dir(paths_cfg);
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

        let skills_dir = helpers::resolve_skills_dir(paths_cfg);
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
                    if p.is_relative() {
                        state_dir.join(p)
                    } else {
                        p
                    }
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
        let (mcp_status_init, mcp_handles_init) = super::AppState::register_mcp_and_subagent_tools(
            &p3.phase1.agents,
            &config.mcp_servers,
            p3.runtime.clone(),
            &p3.tool_registry,
        )
        .await?;

        let (channel_registry, channel_inbound_tx, inbound_rx) =
            super::AppState::build_channels(config, &p3.tool_registry).await?;

        let base_skill_registry = Arc::new(p3.base_skill_registry.filtered(
            &config.skills.allow,
            &config.skills.deny,
            None,
        ));
        fastclaw_core::workspace::set_skill_prompt_mode(config.skills.prompt_mode.clone());
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

        let multi_agent_identity = p3.workspaces.len() > 1;
        for (agent_id, ws) in &p3.workspaces {
            let ws_arc = Arc::new(ws.clone());
            if multi_agent_identity {
                let sfx = memory_tool_agent_suffix(agent_id);
                let get_inner = Arc::new(fastclaw_agent::builtin_tools::GetIdentityTool::new(
                    ws_arc.clone(),
                ));
                let set_inner =
                    Arc::new(fastclaw_agent::builtin_tools::SetIdentityTool::new(ws_arc));
                let get_name = format!("get_identity__{sfx}");
                let set_name = format!("set_identity__{sfx}");
                let get_desc = format!("{} (agent `{}`)", get_inner.description(), agent_id);
                let set_desc = format!("{} (agent `{}`)", set_inner.description(), agent_id);
                p3.tool_registry.register(Arc::new(RenamedTool::new(
                    get_name,
                    get_desc,
                    get_inner as Arc<dyn fastclaw_core::tool::Tool + Send + Sync>,
                )));
                p3.tool_registry.register(Arc::new(RenamedTool::new(
                    set_name,
                    set_desc,
                    set_inner as Arc<dyn fastclaw_core::tool::Tool + Send + Sync>,
                )));
                tracing::info!(agent_id = %agent_id, "registered scoped get_identity / set_identity tools");
            } else {
                fastclaw_agent::builtin_tools::register_identity_tools(&p3.tool_registry, ws_arc);
                tracing::info!(agent_id = %agent_id, "registered get_identity / set_identity tools");
            }
        }

        let stream_event_tx = Arc::new(DashMap::new());
        let ask_question_pending = Arc::new(DashMap::new());
        p3.tool_registry.register(Arc::new(
            fastclaw_agent::builtin_tools::AskQuestionTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            ),
        ));
        p3.tool_registry
            .register(Arc::new(fastclaw_agent::builtin_tools::ConfirmTool::new(
                stream_event_tx.clone(),
                ask_question_pending.clone(),
            )));
        tracing::info!("registered ask_question + confirm tools");

        Ok(BuildPhase4 {
            phase3: p3,
            channel_registry,
            channel_inbound_tx,
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
        let (agent_episodic_map, agent_semantic_map, embedding_provider) = super::AppState::build_memory(
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
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
        }
        fastclaw_agent::builtin_tools::register_session_tools(
            &p4.phase3.tool_registry,
            p4.phase3.phase1.session_store.clone(),
            message_bus.clone(),
        );

        let evo_pool = helpers::open_memory_pool_named(&p4.phase3.phase1.db_path, "evolution.db").await?;
        let feedback_store = FeedbackStore::open(evo_pool.clone()).await?;
        let trajectory_store = TrajectoryStore::open(evo_pool.clone()).await?;
        let skill_store = SkillStore::open(evo_pool.clone()).await?;
        let prompt_distiller = PromptDistiller::open(evo_pool).await?;

        let mut context_engine =
            fastclaw_context::ContextEngine::new(fastclaw_context::DEFAULT_COMPACTION_THRESHOLD);
        context_engine.add_hook(Arc::new(fastclaw_context::CompactionHook::new(
            fastclaw_context::CompactionStrategy::default(),
        )));
        context_engine.add_hook(Arc::new(fastclaw_context::ContentFilterHook::default()));
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

            context_engine.add_hook(Arc::new(fastclaw_context::MemoryKeywordInterceptor::new(
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
                .unwrap_or_else(|| "gpt-4o-mini".to_string());

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
                    fastclaw_memory::ImportanceScorer::from(config.memory.importance.clone()),
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

        let cron_pool = helpers::open_memory_pool_named(&p2.phase4.phase3.phase1.db_path, "cron.db").await?;
        let cron_store = CronJobStore::open(cron_pool.clone()).await?;
        let notification_store =
            crate::notification_store::NotificationStore::open(cron_pool).await?;

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
            notification_store,
            budget_tracker,
            model_router,
            ws_broadcast,
        })
    }

    /// Run all phases in dependency order and produce a ready [`AppState`].
    pub(crate) async fn build(config: FastClawConfig) -> anyhow::Result<AppState> {
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

        let prompt_injection_enabled = config.security.prompt_injection_detection;
        let config_live_val = serde_json::to_value(&config).unwrap_or_default();
        let runtime_for_subagent = p5.phase2.phase4.phase3.runtime.clone();
        let state = AppState {
            cfg: super::ConfigState {
                config: Arc::new(config),
                config_live: Arc::new(ArcSwap::new(Arc::new(config_live_val))),
                runtime_route_bindings: Arc::new(tokio::sync::RwLock::new(Vec::new())),
                last_good_agents: Arc::new(tokio::sync::RwLock::new(initial_agents.clone())),
            },
            rt: super::RuntimeState {
                router: Arc::new(tokio::sync::RwLock::new(p5.phase2.phase4.phase3.router)),
                runtime: p5.phase2.phase4.phase3.runtime,
                tool_registry: Arc::new(p5.phase2.phase4.phase3.tool_registry),
                base_skill_registry: Arc::new(ArcSwap::new(
                    p5.phase2.phase4.base_skill_registry,
                )),
                agent_skill_registries: Arc::new(ArcSwap::new(Arc::new(
                    p5.phase2.phase4.phase3.agent_skill_registries,
                ))),
                workspaces: Arc::new(p5.phase2.phase4.phase3.workspaces),
                prompt_guard: {
                    let mut pg = fastclaw_security::PromptGuard::new();
                    pg.set_enabled(prompt_injection_enabled);
                    Arc::new(pg)
                },
            },
            store: super::StorageState {
                session_store: p5.phase2.phase4.phase3.phase1.session_store,
                dag_checkpoint_store: p5.dag_checkpoint_store,
                cron_store: Arc::new(p5.cron_store),
                cron_wake: Arc::new(tokio::sync::Notify::new()),
                notification_store: Arc::new(p5.notification_store),
                feedback_store: Arc::new(p5.phase2.feedback_store),
                prompt_distiller: Arc::new(p5.phase2.prompt_distiller),
                trajectory_store,
                skill_store,
                context_engine: Arc::new(p5.phase2.context_engine),
            },
            mem: super::MemoryState {
                agent_episodic: Arc::new(p5.phase2.agent_episodic_map),
                agent_semantic: Arc::new(p5.phase2.agent_semantic_map),
                embedding_provider: p5.phase2.embedding_provider,
            },
            ext: super::ExtensionState {
                plugin_registry: Arc::new(tokio::sync::RwLock::new(
                    p5.phase2.phase4.phase3.plugin_registry,
                )),
                channel_registry: Arc::new(tokio::sync::RwLock::new(
                    p5.phase2.phase4.channel_registry,
                )),
                message_bus: p5.phase2.message_bus,
                mcp_status: Arc::new(ArcSwap::new(Arc::new(
                    p5.phase2.phase4.mcp_status_init,
                ))),
                mcp_handles: Arc::new(tokio::sync::Mutex::new(
                    p5.phase2.phase4.mcp_handles_init,
                )),
                channel_inbound_tx: p5.phase2.phase4.channel_inbound_tx,
            },
            obs: super::ObserveState {
                metrics_collector: Arc::new(fastclaw_observe::MetricsCollector::new()),
                budget_tracker: Arc::new(p5.budget_tracker),
                model_router: p5.model_router,
            },
            strm: super::StreamState {
                stream_event_tx: p5.phase2.phase4.stream_event_tx,
                ask_question_pending: p5.phase2.phase4.ask_question_pending,
                ws_broadcast: p5.ws_broadcast,
                subagent_manager: Arc::new(fastclaw_agent::SubAgentManager::new(
                    runtime_for_subagent,
                    initial_agents.clone(),
                    fastclaw_core::agent_config::SubAgentPolicy::default(),
                )),
            },
        };

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
        tracing::info!("registered channel management tools");

        state
            .rt
            .tool_registry
            .register(Arc::new(fastclaw_agent::SubAgentTool::new(
                state.strm.subagent_manager.clone(),
                state.rt.tool_registry.clone(),
                fastclaw_core::agent_config::SubAgentPolicy::default(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(fastclaw_agent::ListAgentsTool::new(
                state.strm.subagent_manager.clone(),
            )));
        state
            .rt
            .tool_registry
            .register(Arc::new(fastclaw_agent::GetAgentInfoTool::new(
                state.strm.subagent_manager.clone(),
            )));
        tracing::info!("registered sub-agent tools (spawn_subagent, list_agents, get_agent_info)");

        state.spawn_skill_evolution_tasks();

        state.spawn_inbound_dispatcher(inbound_rx);

        let dream_secs = state.cfg.config.memory.dreaming_interval_secs;
        if state.cfg.config.memory.enabled && dream_secs > 0 && !state.mem.agent_episodic.is_empty()
        {
            let episodic = state.mem.agent_episodic.clone();
            let semantic = state.mem.agent_semantic.clone();
            let dream_embedder = state.mem.embedding_provider.clone();
            let dream_scorer = Some(fastclaw_memory::ImportanceScorer::from(
                state.cfg.config.memory.importance.clone(),
            ));
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

        if let Some(ref plugins_path) = state.cfg.config.paths.plugins_dir {
            let dir = PathBuf::from(plugins_path);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to create plugins directory for hot-reload watcher"
                );
            } else {
                match fastclaw_plugin::start_watching(state.ext.plugin_registry.clone(), dir.clone()) {
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

        Ok(state)
    }
}
