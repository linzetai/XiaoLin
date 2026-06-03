use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use xiaolin_agent::{
    create_provider, create_provider_with_credentials, process_channel::ProcessChannelPlugin,
    AgentRuntime, FallbackProvider,
};
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::bus::MessageBus;
use xiaolin_core::channel::{ChannelPlugin, ChannelRegistry};
use xiaolin_core::channel_plugin::{self, ChannelPluginConfig};
use xiaolin_core::config::XiaoLinConfig;
use xiaolin_core::routing::RuntimeRouteBinding;
use xiaolin_core::skill::SkillRegistry;
use xiaolin_core::tool::Tool;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::workspace::AgentWorkspace;
use xiaolin_core::Router as AgentRouter;
use xiaolin_cron::CronJobStore;
use xiaolin_evolution::{
    FeedbackStore, LlmExtractedPattern, LlmExtractionCallback, PromptDistiller, SkillExtractor,
    SkillParam, SkillStore, TrajectoryStore,
};
use xiaolin_memory::{EmbeddingProvider, EpisodicMemory, SemanticMemory};
use xiaolin_model_router::BudgetTracker;
use xiaolin_session::{EventLog, SessionStore};
#[cfg(any(test, feature = "test-helpers"))]
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
            anyhow::bail!("agent reload rejected: duplicate agent_id `{}`", a.agent_id);
        }
    }
    Ok(())
}

mod builder;
mod helpers;

/// Configuration and routing metadata.
#[derive(Clone)]
pub struct ConfigState {
    pub config: Arc<XiaoLinConfig>,
    pub config_live: Arc<ArcSwap<serde_json::Value>>,
    pub runtime_route_bindings: Arc<tokio::sync::RwLock<Vec<RuntimeRouteBinding>>>,
    pub last_good_agents: Arc<tokio::sync::RwLock<Vec<AgentConfig>>>,
}

/// Agent runtime, tool registry, skill registries.
#[derive(Clone)]
pub struct RuntimeState {
    pub router: SharedRouter,
    pub runtime: Arc<AgentRuntime>,
    pub tool_registry: Arc<ToolRegistry>,
    pub base_skill_registry: Arc<ArcSwap<SkillRegistry>>,
    pub agent_skill_registries: Arc<ArcSwap<std::collections::HashMap<String, Arc<SkillRegistry>>>>,
    pub workspaces: Arc<std::collections::HashMap<String, AgentWorkspace>>,
    pub prompt_guard: Arc<xiaolin_security::PromptGuard>,
    pub session_modes: xiaolin_agent::builtin_tools::SessionModeRegistry,
    pub todo_store: xiaolin_agent::builtin_tools::TodoStore,
    pub plan_file_store: xiaolin_agent::builtin_tools::PlanFileStore,
}

/// Persistent stores.
#[derive(Clone)]
pub struct StorageState {
    pub session_store: Arc<SessionStore>,
    pub event_log: Arc<EventLog>,
    pub cron_store: Arc<CronJobStore>,
    pub cron_wake: Arc<tokio::sync::Notify>,
    pub notification_store: Arc<crate::notification_store::NotificationStore>,
    pub feedback_store: Arc<FeedbackStore>,
    pub prompt_distiller: Arc<PromptDistiller>,
    pub trajectory_store: Arc<TrajectoryStore>,
    pub skill_store: Arc<SkillStore>,
    pub context_engine: Arc<xiaolin_context::ContextEngine>,
}

pub(crate) struct LlmSkillExtraction {
    pub(crate) provider: Arc<dyn xiaolin_agent::LlmProvider>,
    pub(crate) model: String,
}

#[async_trait::async_trait]
impl LlmExtractionCallback for LlmSkillExtraction {
    async fn extract_pattern(
        &self,
        trajectories_summary: &str,
    ) -> anyhow::Result<LlmExtractedPattern> {
        let prompt = format!(
            "You are a skill pattern extractor. Given a cluster of successful AI agent trajectories, \
             extract a reusable skill pattern.\n\n\
             Respond in JSON with these fields:\n\
             - name: short descriptive name\n\
             - task_pattern: when this skill applies\n\
             - strategy_template: step-by-step instructions for an AI agent to follow\n\
             - parameters: array of {{name, param_type, description}} for variable parts\n\n\
             Trajectories:\n{trajectories_summary}"
        );
        let messages = vec![xiaolin_core::types::ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(prompt)),
        ..Default::default()
        }];
        let params = xiaolin_agent::CompletionParams {
            model: &self.model,
            messages: &messages,
            temperature: 0.3,
            max_tokens: Some(500),
            tools: None,
        };
        let resp = self.provider.chat_completion(&params).await?;
        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.text_content())
            .unwrap_or_default();

        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "LlmSkillExtraction: LLM returned non-JSON, using raw text as strategy_template");
            serde_json::json!({
                "name": "unnamed",
                "task_pattern": "",
                "strategy_template": cleaned,
                "parameters": []
            })
        });
        Ok(LlmExtractedPattern {
            name: parsed["name"].as_str().unwrap_or("unnamed").to_string(),
            task_pattern: parsed["task_pattern"].as_str().unwrap_or("").to_string(),
            strategy_template: parsed["strategy_template"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            parameters: parsed["parameters"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            Some(SkillParam {
                                name: p["name"].as_str()?.to_string(),
                                param_type: p["param_type"]
                                    .as_str()
                                    .unwrap_or("string")
                                    .to_string(),
                                description: p["description"].as_str().unwrap_or("").to_string(),
                                default_value: p["default_value"].as_str().map(String::from),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// Memory subsystem.
#[derive(Clone)]
pub struct MemoryState {
    pub agent_episodic: Arc<std::collections::HashMap<String, Arc<EpisodicMemory>>>,
    pub agent_semantic: Arc<std::collections::HashMap<String, Arc<SemanticMemory>>>,
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

/// Extensions (plugins, channels, MCP, message bus).
#[derive(Clone)]
pub struct ExtensionState {
    pub channel_registry: Arc<tokio::sync::RwLock<ChannelRegistry>>,
    pub message_bus: Arc<MessageBus>,
    pub mcp_status:
        Arc<ArcSwap<std::collections::HashMap<String, xiaolin_core::types::McpServerStatus>>>,
    pub mcp_handles:
        Arc<tokio::sync::Mutex<std::collections::HashMap<String, xiaolin_mcp::SharedMcpClient>>>,
    pub channel_inbound_tx:
        tokio::sync::mpsc::UnboundedSender<xiaolin_core::channel::InboundMessage>,
    pub llm_plugin_registry: Arc<tokio::sync::RwLock<xiaolin_agent::LlmPluginRegistry>>,
    /// Per-chat semaphore to serialize message processing within the same chat_id.
    pub chat_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Semaphore>>>,
    /// Per-chat cancellation tokens for in-flight processing (used by `/stop`).
    pub chat_cancels: Arc<dashmap::DashMap<String, tokio_util::sync::CancellationToken>>,
    /// Per-chat model overrides set via `/model <name>`.
    pub chat_model_overrides: Arc<dashmap::DashMap<String, String>>,
    /// Active WeChat QR login sessions (session_key → QrLoginSession).
    pub wechat_login_sessions:
        Arc<dashmap::DashMap<String, xiaolin_wechat::auth::qr_login::QrLoginSession>>,
}

/// Observability.
#[derive(Clone)]
pub struct ObserveState {
    pub metrics_collector: Arc<xiaolin_observe::MetricsCollector>,
    pub budget_tracker: Arc<BudgetTracker>,
    pub model_router: Option<Arc<xiaolin_model_router::ModelRouter>>,
}

/// Streaming and real-time state.
///
/// After Phase A, approvals and answers resolve directly via `InteractionHandle`
/// inside the Session Actor. The DashMap bridges have been removed. Only
/// `stream_event_tx` remains for builtin-tool event delivery and
/// `tool_orchestrator` for policy-only checks (no pending DashMap).
#[derive(Clone)]
pub struct StreamState {
    /// Used by `RuntimeTurnExecutor` for `ask_question`/`confirm`
    /// builtin tool integration via task-local context key.
    pub stream_event_tx:
        Arc<DashMap<String, tokio::sync::mpsc::Sender<xiaolin_protocol::AgentEvent>>>,
    /// Policy-only orchestrator. Approvals are resolved via `InteractionHandle`
    /// when running inside a Session Actor, or via the internal DashMap
    /// when running in CLI/non-actor mode.
    pub tool_orchestrator: Arc<xiaolin_agent::ToolOrchestrator>,
    pub ws_broadcast: tokio::sync::broadcast::Sender<String>,
    pub subagent_manager: Arc<xiaolin_agent::SubAgentManager>,
    pub session_manager: Arc<xiaolin_session_actor::SessionManager>,
}

/// Services shared across session actors and the gateway.
///
/// Extracted from `AppState` to provide a clean dependency-injection boundary
/// between the session-actor layer and the transport/gateway layer. The
/// `SessionManager` receives a `TurnExecutor` backed by these services.
#[derive(Clone)]
pub struct SharedServices {
    pub runtime: Arc<AgentRuntime>,
    pub tool_registry: Arc<ToolRegistry>,
    pub session_store: Arc<SessionStore>,
    pub event_log: Arc<EventLog>,
    pub context_engine: Arc<xiaolin_context::ContextEngine>,
    pub prompt_guard: Arc<xiaolin_security::PromptGuard>,
    pub session_manager: Arc<xiaolin_session_actor::SessionManager>,
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: ConfigState,
    pub rt: RuntimeState,
    pub store: StorageState,
    pub mem: MemoryState,
    pub ext: ExtensionState,
    pub obs: ObserveState,
    pub strm: StreamState,
    pub svc: SharedServices,
}

impl AppState {
    /// Hot-reload all MCP servers: compare `config_live` with running handles,
    /// stop removed/changed servers, start new/changed ones, update status map.
    pub async fn reload_mcp_servers(&self) -> anyhow::Result<()> {
        use xiaolin_core::agent_config::McpServerConfig;
        use xiaolin_core::types::{McpServerStatus, McpStatus};

        let mut desired: Vec<McpServerConfig> = {
            let live = self.cfg.config_live.load();
            let mcp_val = live
                .get("mcpServers")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::from_value(mcp_val).unwrap_or_default()
        };

        if let Ok(cwd) = std::env::current_dir() {
            let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
            if let Some(project_mcp) =
                xiaolin_core::agent_config::load_project_mcp_config(&ws_root)
            {
                let project_configs = project_mcp.to_mcp_server_configs();
                let existing_ids: std::collections::HashSet<String> =
                    desired.iter().map(|c| c.id.clone()).collect();
                for cfg in project_configs {
                    if existing_ids.contains(&cfg.id) {
                        if cfg.enabled == Some(false) {
                            desired.retain(|c| c.id != cfg.id);
                        }
                    } else {
                        desired.push(cfg);
                    }
                }
            }
        }

        let desired_map: std::collections::HashMap<String, &McpServerConfig> =
            desired.iter().map(|c| (c.id.clone(), c)).collect();

        let mut handles = self.ext.mcp_handles.lock().await;

        let current_ids: std::collections::HashSet<String> = handles.keys().cloned().collect();
        let desired_ids: std::collections::HashSet<String> = desired_map.keys().cloned().collect();

        let to_remove: Vec<String> = current_ids.difference(&desired_ids).cloned().collect();
        for id in &to_remove {
            let prefix = format!("mcp_{}_", id);
            let removed = self.rt.tool_registry.unregister_by_prefix(&prefix);
            tracing::info!(mcp_id = %id, tools_removed = removed, "stopped MCP server (removed from config)");
            handles.remove(id);
        }

        let mut new_status: std::collections::HashMap<String, McpServerStatus> =
            std::collections::HashMap::new();

        for cfg in &desired {
            if cfg.enabled == Some(false) {
                if handles.contains_key(&cfg.id) {
                    let prefix = format!("mcp_{}_", cfg.id);
                    self.rt.tool_registry.unregister_by_prefix(&prefix);
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
                self.rt.tool_registry.unregister_by_prefix(&prefix);
                handles.remove(&cfg.id);
                tracing::info!(mcp_id = %cfg.id, "restarting MCP server");
            }

            let args_ref: Vec<&str> = cfg.args.iter().map(|s| s.as_str()).collect();
            let prefix = format!("mcp_{}_", cfg.id);
            let tool_count_before = self.rt.tool_registry.len();
            match xiaolin_mcp::register_mcp_tools(
                &cfg.command,
                &args_ref,
                &self.rt.tool_registry,
                &prefix,
                &cfg.env,
            )
            .await
            {
                Ok(handle) => {
                    let tool_count = self.rt.tool_registry.len() - tool_count_before;
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

        self.ext.mcp_status.store(Arc::new(new_status));

        Ok(())
    }

    fn spawn_skill_evolution_tasks(&self) {
        let skill_store = self.store.skill_store.clone();
        let maintenance_secs = self.cfg.config.evolution.skill_maintenance_interval_secs;
        if maintenance_secs > 0 {
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(maintenance_secs.max(1)));
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

        let skill_store_ex = self.store.skill_store.clone();
        let trajectory_store_ex = self.store.trajectory_store.clone();
        let llm_for_extraction = Arc::new(LlmSkillExtraction {
            provider: self.rt.runtime.default_provider_arc(),
            model: self
                .cfg
                .config
                .agents
                .list
                .first()
                .and_then(|a| a.model.clone())
                .unwrap_or_else(|| "gpt-4o-mini".to_string()),
        });
        let extraction_secs = self.cfg.config.evolution.skill_extraction_interval_secs;
        if extraction_secs > 0 {
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(extraction_secs.max(1)));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match trajectory_store_ex.get_recent_successful_global(200).await {
                        Ok(trajs) if !trajs.is_empty() => {
                            let extractor = SkillExtractor::default();
                            let extracted = match extractor
                                .extract_skills_with_llm(&trajs, llm_for_extraction.as_ref())
                                .await
                            {
                                Ok(skills) => {
                                    tracing::info!(
                                        trajectories = trajs.len(),
                                        candidates = skills.len(),
                                        "skill extraction pass (LLM-enhanced)"
                                    );
                                    skills
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "LLM skill extraction failed, falling back to rule-based");
                                    extractor.extract_skills(&trajs)
                                }
                            };
                            for ext in extracted {
                                let needle = format!("{} {}", ext.name, ext.task_pattern);
                                let similar = match skill_store_ex.find_similar(&needle, 18).await {
                                    Ok(s) => s,
                                    Err(e) => {
                                        tracing::warn!(error = %e, "find_similar during extraction failed");
                                        continue;
                                    }
                                };
                                let duplicate =
                                    similar.iter().any(|s| s.task_pattern == ext.task_pattern);
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
                        Err(e) => {
                            tracing::warn!(error = %e, "load trajectories for extraction failed")
                        }
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
    /// When a plugin registry is provided, providers with a `plugin:` prefix
    /// are resolved through it.
    fn build_runtime(
        agents: &[AgentConfig],
        creds: &xiaolin_core::config::CredentialsConfig,
        plugin_registry: Option<&xiaolin_agent::LlmPluginRegistry>,
    ) -> anyhow::Result<Arc<AgentRuntime>> {
        let primary_model_config = agents
            .first()
            .map(|a| &a.model)
            .cloned()
            .unwrap_or_default();

        let default_provider: Box<dyn xiaolin_agent::LlmProvider> =
            match xiaolin_agent::create_provider_chain_with_plugins(
                &primary_model_config,
                Some(creds),
                plugin_registry,
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

        let runtime = Arc::new({
            let rt = AgentRuntime::new(Arc::from(default_provider));
            #[cfg(feature = "self-iter")]
            let rt = rt.with_self_iter_engine(Arc::new(
                xiaolin_self_iter::SelfIterEngine::diagnosis_only(),
            ));
            rt
        });

        for agent in agents {
            match xiaolin_agent::create_provider_chain_with_plugins(
                &agent.model,
                Some(creds),
                plugin_registry,
            ) {
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
        config: &XiaoLinConfig,
        tool_registry: &ToolRegistry,
    ) -> anyhow::Result<(
        ChannelRegistry,
        tokio::sync::mpsc::UnboundedSender<xiaolin_core::channel::InboundMessage>,
        tokio::sync::mpsc::UnboundedReceiver<xiaolin_core::channel::InboundMessage>,
    )> {
        let mut channel_registry = ChannelRegistry::new();
        let (inbound_tx, inbound_rx) =
            tokio::sync::mpsc::unbounded_channel::<xiaolin_core::channel::InboundMessage>();

        // Build Feishu and WeChat channel plugins in parallel.
        let feishu_config = config.channels.get("feishu").and_then(|ch| {
            if ch.enabled == Some(false) {
                None
            } else {
                xiaolin_feishu::FeishuPluginConfig::from_channel_config(ch)
            }
        });
        let wechat_config = config
            .channels
            .get("wechat")
            .and_then(xiaolin_wechat::WechatChannelConfig::from_channel_config);

        let feishu_tx = inbound_tx.clone();
        let wechat_tx = inbound_tx.clone();

        let (feishu_result, wechat_result) = tokio::join!(
            async {
                if let Some(plugin) = feishu_config.map(xiaolin_feishu::FeishuPlugin::new) {
                    let plugin = Arc::new(plugin);
                    let mode = plugin.connection_mode().to_string();
                    if let Err(e) = plugin.start(feishu_tx).await {
                        tracing::error!(error = %e, "failed to start Feishu channel plugin");
                    }
                    Some((plugin, mode))
                } else {
                    tracing::debug!("feishu channel not configured, plugin not loaded");
                    None
                }
            },
            async {
                if let Some(wechat_cfg) = wechat_config {
                    let plugin = Arc::new(xiaolin_wechat::WechatPlugin::new(wechat_cfg));
                    let mode = plugin.connection_mode().to_string();
                    if let Err(e) = plugin.start(wechat_tx).await {
                        tracing::error!(error = %e, "failed to start WeChat channel plugin");
                    }
                    Some((plugin, mode))
                } else {
                    tracing::debug!("wechat channel not configured, plugin not loaded");
                    None
                }
            },
        );

        if let Some((feishu_plugin, mode)) = feishu_result {
            let llm_tools = feishu_plugin.llm_tools();
            let tool_count = llm_tools.len();
            for tool in llm_tools {
                tool_registry.register_channel_scoped(tool);
            }
            channel_registry.register(feishu_plugin);
            tracing::info!(
                mode,
                tool_count,
                "Feishu channel plugin registered (tools channel-scoped, not globally visible)"
            );
        }

        if let Some((wechat_plugin, mode)) = wechat_result {
            channel_registry.register(wechat_plugin);
            tracing::info!(mode, "WeChat channel plugin registered");
        }

        // Load process-based channel plugins from config files.
        if config.channel_plugins.enabled {
            let plugins_dir =
                channel_plugin::resolve_channel_plugins_dir(&config.channel_plugins, &config.paths);
            let plugin_configs = channel_plugin::load_channel_plugins(&plugins_dir);

            for pc in plugin_configs {
                if !pc.enabled {
                    tracing::info!(plugin_id = %pc.id, "channel plugin disabled, skipping");
                    continue;
                }

                let plugin_id = pc.id.clone();
                let account_config = config
                    .channels
                    .get(&pc.id)
                    .map(|ch| serde_json::to_value(ch).unwrap_or(serde_json::Value::Null))
                    .unwrap_or(serde_json::Value::Null);

                match Self::start_process_channel(pc, account_config, &inbound_tx).await {
                    Ok(plugin) => {
                        channel_registry.register(Arc::new(plugin));
                    }
                    Err(e) => {
                        tracing::error!(
                            plugin_id = %plugin_id,
                            error = %e,
                            "failed to start process channel plugin"
                        );
                    }
                }
            }
        }

        Ok((channel_registry, inbound_tx, inbound_rx))
    }

    /// Start a process-based channel plugin.
    async fn start_process_channel(
        config: ChannelPluginConfig,
        account_config: serde_json::Value,
        inbound_tx: &tokio::sync::mpsc::UnboundedSender<xiaolin_core::channel::InboundMessage>,
    ) -> anyhow::Result<ProcessChannelPlugin> {
        let plugin_id = config.id.clone();
        let plugin = ProcessChannelPlugin::new(config);

        if let Err(e) = plugin.initialize(account_config).await {
            tracing::error!(plugin_id = %plugin_id, error = %e, "failed to initialize process channel plugin");
            anyhow::bail!(
                "failed to initialize process channel plugin '{}': {e}",
                plugin_id
            );
        }

        if let Err(e) = plugin.start(inbound_tx.clone()).await {
            tracing::error!(plugin_id = %plugin_id, error = %e, "failed to start process channel plugin");
            anyhow::bail!(
                "failed to start process channel plugin '{}': {e}",
                plugin_id
            );
        }

        tracing::info!(
            plugin_id = %plugin_id,
            mode = plugin.connection_mode(),
            "process channel plugin registered"
        );

        Ok(plugin)
    }

    /// Built-ins and web/media tools (no MCP, no subagent).
    /// Returns the `ToolRegistry` together with the shared `TodoStore` so
    /// stop-hooks can inspect incomplete todos at runtime.
    async fn build_tools_core(
        config: &XiaoLinConfig,
    ) -> anyhow::Result<(ToolRegistry, xiaolin_agent::builtin_tools::TodoStore)> {
        let creds = &config.credentials;
        let tool_registry = ToolRegistry::new();
        xiaolin_agent::builtin_tools::register_builtin_tools(&tool_registry);
        let todo_store = xiaolin_agent::builtin_tools::TodoStore::new();
        xiaolin_agent::builtin_tools::register_todo_tools(&tool_registry, todo_store.clone());

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
                    Some(xiaolin_agent::WebSearchBackend::Tavily { api_key: key })
                }
            }
            "searxng" => {
                let base = ws_cfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8888".to_string());
                Some(xiaolin_agent::WebSearchBackend::SearXNG { base_url: base })
            }
            "builtin" => {
                let engine_ids = ws_cfg.engines.clone().unwrap_or_else(|| {
                    xiaolin_agent::BUILTIN_ENGINE_IDS
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                });
                tracing::info!(engines = ?engine_ids, "using built-in meta search engine");
                Some(xiaolin_agent::WebSearchBackend::Builtin {
                    engines: engine_ids,
                })
            }
            _ => {
                tracing::info!("web_search backend not configured — web_search tool will prompt user to configure");
                None
            }
        };
        if let Some(backend) = search_backend {
            xiaolin_agent::builtin_tools::register_web_tools(&tool_registry, backend);
        }

        xiaolin_agent::builtin_tools::register_browser_tool(&tool_registry);
        tracing::info!("registered browser tool (headless Chrome)");

        if let Some(openai_key) = creds.get_api_key("openai") {
            let openai_base = creds
                .get_base_url("openai")
                .unwrap_or("https://api.openai.com/v1");
            xiaolin_agent::builtin_tools::register_media_tools(
                &tool_registry,
                openai_base,
                openai_key,
            );
            tracing::info!("registered image_generate and text_to_speech tools");
        }

        Ok((tool_registry, todo_store))
    }

    /// MCP servers and the subagent tool (must run after [`Self::build_tools_core`]).
    ///
    /// Global MCP servers are registered first; per-agent servers with the same `id`
    /// are skipped to avoid duplicate subprocesses.
    ///
    /// Returns `(status_map, handles_map)` for populating `AppState` fields.
    async fn register_mcp_and_subagent_tools(
        agents: &[AgentConfig],
        global_mcp: &[xiaolin_core::agent_config::McpServerConfig],
        _runtime: Arc<AgentRuntime>,
        tool_registry: &ToolRegistry,
    ) -> anyhow::Result<(
        std::collections::HashMap<String, xiaolin_core::types::McpServerStatus>,
        std::collections::HashMap<String, xiaolin_mcp::SharedMcpClient>,
    )> {
        use xiaolin_core::types::{McpServerStatus, McpStatus};

        let mut status_map: std::collections::HashMap<String, McpServerStatus> =
            std::collections::HashMap::new();
        let mut handles_map: std::collections::HashMap<String, xiaolin_mcp::SharedMcpClient> =
            std::collections::HashMap::new();
        let mut registered_ids = std::collections::HashSet::new();

        tracing::info!(
            global_count = global_mcp.len(),
            agent_count = agents.len(),
            global_ids = ?global_mcp.iter().map(|c| &c.id).collect::<Vec<_>>(),
            "register_mcp_and_subagent_tools: starting"
        );

        let all_mcp_configs: Vec<(&xiaolin_core::agent_config::McpServerConfig, &str)> =
            global_mcp
                .iter()
                .map(|c| (c, "global"))
                .chain(
                    agents
                        .iter()
                        .flat_map(|a| a.mcp_servers.iter().map(move |c| (c, a.agent_id.as_str()))),
                )
                .collect();

        // Partition configs: disabled/duplicate → immediate status; enabled → parallel connect.
        let mut to_connect = Vec::new();
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
            registered_ids.insert(mcp_cfg.id.clone());
            to_connect.push((*mcp_cfg, *scope));
        }

        let futs: Vec<_> = to_connect
            .iter()
            .map(|(mcp_cfg, scope)| {
                let id = mcp_cfg.id.clone();
                let command = mcp_cfg.command.clone();
                let args: Vec<String> = mcp_cfg.args.clone();
                let env = mcp_cfg.env.clone();
                let prefix = format!("mcp_{}_", id);
                let scope = scope.to_string();
                let registry = tool_registry.clone();
                async move {
                    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    let result = xiaolin_mcp::register_mcp_tools(
                        &command, &args_ref, &registry, &prefix, &env,
                    )
                    .await;
                    (id, scope, result)
                }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        for (id, scope, result) in results {
            match result {
                Ok(handle) => {
                    tracing::info!(
                        mcp_id = %id,
                        scope = %scope,
                        "MCP client connected"
                    );
                    let now = chrono::Utc::now().to_rfc3339();
                    let tool_count = handle.tools().len();
                    status_map.insert(
                        id.clone(),
                        McpServerStatus {
                            id: id.clone(),
                            status: McpStatus::Connected,
                            error: None,
                            tool_count,
                            connected_at: Some(now),
                        },
                    );
                    handles_map.insert(id, handle);
                }
                Err(e) => {
                    tracing::warn!(
                        mcp_id = %id,
                        scope = %scope,
                        error = %e,
                        "failed to connect MCP server, skipping"
                    );
                    status_map.insert(
                        id.clone(),
                        McpServerStatus {
                            id,
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

        Ok((status_map, handles_map))
    }

    /// Per-workspace SQLite memory, pre-loaded embeddings, and memory tools on `tool_registry`.
    async fn build_memory_with_provider(
        config: &XiaoLinConfig,
        workspaces: &std::collections::HashMap<String, AgentWorkspace>,
        tool_registry: &ToolRegistry,
        preloaded_embedding: Option<Arc<dyn EmbeddingProvider>>,
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
            let pool = helpers::open_memory_pool_at(&agent_db).await?;
            let ep = EpisodicMemory::open(pool.clone()).await?;
            let sem = SemanticMemory::open(pool).await?;
            tracing::debug!(agent_id = %agent_id, db = %agent_db.display(), "agent memory initialized");
            agent_episodic_map.insert(agent_id.clone(), Arc::new(ep));
            agent_semantic_map.insert(agent_id.clone(), Arc::new(sem));
        }

        let embedding_provider = preloaded_embedding;

        let multi_agent_memory = agent_episodic_map.len() > 1;
        for agent_id in agent_episodic_map.keys() {
            if let (Some(ep), Some(sem)) = (
                agent_episodic_map.get(agent_id),
                agent_semantic_map.get(agent_id),
            ) {
                let unified_memory = Arc::new(xiaolin_agent::UnifiedMemoryTool::new(
                    ep.clone(),
                    sem.clone(),
                    embedding_provider.clone(),
                    agent_id.clone(),
                ));
                if multi_agent_memory {
                    let sfx = memory_tool_agent_suffix(agent_id);
                    let mem_name = format!("memory__{sfx}");
                    let mem_desc =
                        format!("{} (agent `{}`)", unified_memory.description(), agent_id);
                    tool_registry.register(Arc::new(RenamedTool::new(
                        mem_name,
                        mem_desc,
                        unified_memory as Arc<dyn xiaolin_core::tool::Tool + Send + Sync>,
                    )));
                    tracing::info!(agent_id = %agent_id, "registered scoped unified memory tool");
                } else {
                    tool_registry.register(unified_memory);
                    tracing::info!(agent_id = %agent_id, "registered unified memory tool");
                }
            }
        }

        tracing::info!("memory system enabled");
        Ok((agent_episodic_map, agent_semantic_map, embedding_provider))
    }

    /// When the primary agent model chain cannot be built, try every provider that has an API key
    /// and wrap them in a [`FallbackProvider`] (same order as sorted credential keys).
    fn build_credentials_fallback_chain(
        creds: &xiaolin_core::config::CredentialsConfig,
    ) -> anyhow::Result<Box<dyn xiaolin_agent::LlmProvider>> {
        let mut keys: Vec<String> = creds.providers.keys().cloned().collect();
        keys.sort();
        let mut chain: Vec<(String, Box<dyn xiaolin_agent::LlmProvider>)> = Vec::new();
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
    pub async fn new(config: XiaoLinConfig) -> anyhow::Result<Self> {
        builder::StateBuilder::build(config).await
    }

    fn spawn_inbound_dispatcher(
        &self,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<xiaolin_core::channel::InboundMessage>,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                // Handle card action callbacks (ask_question answers)
                if msg.msg_type == "card_action" {
                    let request_id = msg
                        .extra
                        .get("request_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&msg.message_id)
                        .to_string();
                    let answer = msg
                        .extra
                        .get("option_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&msg.text)
                        .to_string();

                    let card_session_id = msg
                        .extra
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let mut resolved = false;
                    if let Some(sid) = &card_session_id {
                        if let Some(handle) = state
                            .svc
                            .session_manager
                            .get(&xiaolin_protocol::SessionId::new(sid))
                            .await
                        {
                            resolved = handle
                                .submit(xiaolin_session_actor::SessionOp::ResolveAnswer {
                                    interaction_id: request_id.clone(),
                                    answer: answer.clone(),
                                })
                                .await
                                .is_ok();
                        }
                    }
                    if resolved {
                        tracing::info!(
                            request_id = %request_id,
                            answer = %answer,
                            "resolved ask_question from card callback"
                        );
                    } else {
                        tracing::debug!(
                            request_id = %request_id,
                            "card action callback for unknown request_id (may have timed out)"
                        );
                    }
                    continue;
                }

                let channel_id = msg.channel_id.clone();
                let chat_id = msg.chat_id.clone();
                let message_id = msg.message_id.clone();
                let text = msg.text.clone();
                let account_id = msg.account_id.clone();
                let chat_type = msg.chat_type.clone();
                let attachments = msg.attachments.clone();

                let registry = state.ext.channel_registry.read().await;
                let channel = match registry.get(&channel_id) {
                    Some(ch) => ch.clone(),
                    None => {
                        tracing::warn!(channel = %channel_id, "inbound message for unknown channel");
                        continue;
                    }
                };
                drop(registry);

                let trimmed = text.trim();
                let cancels_inflight = matches!(trimmed, "/stop" | "/new" | "/new session" | "/reset");
                let is_slash = trimmed.starts_with('/') && trimmed.len() > 1;

                if cancels_inflight {
                    if let Some((_, token)) = state.ext.chat_cancels.remove(&chat_id) {
                        token.cancel();
                        tracing::info!(chat_id = %chat_id, cmd = trimmed, "cancelled in-flight processing");
                    }
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::routes::handle_channel_message(
                            state_clone, channel, &channel_id, &chat_id,
                            &message_id, &text, account_id.as_deref(), &chat_type,
                            vec![],
                        ).await {
                            tracing::debug!(error = %e, "command reply failed");
                        }
                    });
                    continue;
                }

                if is_slash {
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::routes::handle_channel_message(
                            state_clone, channel, &channel_id, &chat_id,
                            &message_id, &text, account_id.as_deref(), &chat_type,
                            vec![],
                        ).await {
                            tracing::error!(
                                error = %e, channel = %channel_id, chat_id = %chat_id,
                                "slash command handling failed"
                            );
                        }
                    });
                    continue;
                }

                let sem = state
                    .ext
                    .chat_locks
                    .entry(chat_id.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
                    .clone();

                let cancel = tokio_util::sync::CancellationToken::new();
                state.ext.chat_cancels.insert(chat_id.clone(), cancel.clone());

                let state_clone = state.clone();
                let chat_id_for_cleanup = chat_id.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    let result = tokio::select! {
                        r = crate::routes::handle_channel_message(
                            state_clone.clone(),
                            channel,
                            &channel_id,
                            &chat_id,
                            &message_id,
                            &text,
                            account_id.as_deref(),
                            &chat_type,
                            attachments,
                        ) => r,
                        () = cancel.cancelled() => {
                            tracing::info!(chat_id = %chat_id, "message processing cancelled by /stop");
                            Ok(())
                        }
                    };
                    state_clone.ext.chat_cancels.remove(&chat_id_for_cleanup);
                    if let Err(e) = result {
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

    /// Garbage-collect stale resources tied to dead sessions.
    /// Call periodically (e.g., every 60s) to prevent unbounded memory growth.
    pub async fn gc_stale_resources(&self) {
        let gc_stats = self.strm.session_manager.gc_with_stats().await;
        let active_ids = self.strm.session_manager.active_session_id_set().await;

        // Clean chat_locks: keep entries that match active session IDs OR have
        // a corresponding active cancel token (channel dispatches use raw chat_id,
        // not session_key, so they won't appear in active_ids).
        let locks_before = self.ext.chat_locks.len();
        self.ext.chat_locks.retain(|k, sem| {
            active_ids.contains(k)
                || self.ext.chat_cancels.contains_key(k)
                || sem.available_permits() == 0 // still processing
        });
        let locks_removed = locks_before - self.ext.chat_locks.len();

        // Clean chat_model_overrides
        let overrides_before = self.ext.chat_model_overrides.len();
        self.ext.chat_model_overrides.retain(|k, _| {
            active_ids.contains(k) || self.ext.chat_cancels.contains_key(k)
        });
        let overrides_removed = overrides_before - self.ext.chat_model_overrides.len();

        // Clean stream_event_tx (remove closed senders)
        let streams_before = self.strm.stream_event_tx.len();
        self.strm.stream_event_tx.retain(|_, tx| !tx.is_closed());
        let streams_removed = streams_before - self.strm.stream_event_tx.len();

        // Clean chat_cancels: cancel tokens self-remove on task completion
        // (see inbound dispatcher), but GC any orphans.
        let cancels_before = self.ext.chat_cancels.len();
        self.ext.chat_cancels.retain(|k, _| {
            active_ids.contains(k) || self.ext.chat_locks.contains_key(k)
        });
        let cancels_removed = cancels_before - self.ext.chat_cancels.len();

        // GC SubAgentManager: remove terminal runs older than 5 minutes and
        // prune completion channels for dead sessions.
        self.strm.subagent_manager.gc(std::time::Duration::from_secs(300));

        let total_removed = locks_removed + overrides_removed + streams_removed + cancels_removed;
        if gc_stats.removed > 0 || total_removed > 0 {
            tracing::info!(
                sessions_removed = gc_stats.removed,
                sessions_alive = gc_stats.alive,
                locks_removed,
                overrides_removed,
                streams_removed,
                cancels_removed,
                "resource GC completed"
            );
        }
    }

    /// Get the skill registry for a specific agent, falling back to base.
    pub fn skill_registry_for(&self, agent_id: &str) -> Arc<SkillRegistry> {
        let registries = self.rt.agent_skill_registries.load();
        registries
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| (*self.rt.base_skill_registry.load()).clone())
    }

    /// Rescan skill directories from disk and rebuild all registries.
    pub fn reload_skills(&self) -> anyhow::Result<usize> {
        use xiaolin_core::skill::{load_skills_from_dirs_with_layer, SkillLayer};

        let paths_cfg = &self.cfg.config.paths;
        let skills_dir = helpers::resolve_skills_dir(paths_cfg);

        let workspace_root = std::env::current_dir()
            .ok()
            .map(|cwd| xiaolin_core::workspace::detect_workspace_root(&cwd));

        let legacy_project_registry =
            load_skills_from_dirs_with_layer(&[skills_dir.as_path()], SkillLayer::Project);
        let cross_tool_registry =
            xiaolin_core::skill::load_skills_cross_tool(workspace_root.as_deref());

        let mut base = SkillRegistry::new();
        base.merge_from(legacy_project_registry);
        base.merge_from(cross_tool_registry);

        let filtered_base = Arc::new(base.filtered(
            &self.cfg.config.skills.allow,
            &self.cfg.config.skills.deny,
            None,
        ));

        let resolved_agents = self.cfg.config.agents.resolved_list();
        let mut per_agent = std::collections::HashMap::new();
        let workspaces = self.rt.workspaces.clone();
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
                &self.cfg.config.skills.allow,
                &self.cfg.config.skills.deny,
                agent_allow,
            );
            per_agent.insert(agent_id.clone(), Arc::new(agent_reg));
        }

        let total = filtered_base.count();
        self.rt.base_skill_registry.store(filtered_base);
        self.rt.agent_skill_registries.store(Arc::new(per_agent));
        tracing::info!(base_skills = total, "skills hot-reloaded from disk");
        Ok(total)
    }

    /// Hot-reload agent configs from disk. Returns the number of agents loaded.
    /// Effective channel bindings: ephemeral API routes first, then config file rows.
    pub async fn merged_route_bindings(&self) -> Vec<xiaolin_core::config::BindingConfig> {
        let rt = self.cfg.runtime_route_bindings.read().await;
        xiaolin_core::routing::merge_runtime_bindings_first(&rt, &self.cfg.config.bindings)
    }

    /// Hot-reload agent configs from disk. Validates before swapping the router so a bad
    /// config never leaves the gateway in a partially-updated state.
    pub async fn reload_agents(&self) -> anyhow::Result<usize> {
        let agents = helpers::load_agents(&self.cfg.config)?;
        self.apply_validated_agent_reload(agents).await
    }

    /// Hot-reload web search tools from the live config without restarting.
    pub fn reload_web_search(&self) -> anyhow::Result<()> {
        let live_cfg = self.cfg.config_live.load();
        let parsed: xiaolin_core::config::XiaoLinConfig =
            serde_json::from_value((**live_cfg).clone())?;
        let ws_cfg = &parsed.web_search;
        let creds = &parsed.credentials;
        let search_backend = match ws_cfg.backend.as_str() {
            "tavily" => {
                let key = ws_cfg
                    .api_key
                    .clone()
                    .or_else(|| creds.get_api_key("tavily").map(String::from))
                    .unwrap_or_default();
                if key.is_empty() {
                    None
                } else {
                    Some(xiaolin_agent::WebSearchBackend::Tavily { api_key: key })
                }
            }
            "searxng" => {
                let base = ws_cfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8888".to_string());
                Some(xiaolin_agent::WebSearchBackend::SearXNG { base_url: base })
            }
            "builtin" => {
                let engine_ids = ws_cfg.engines.clone().unwrap_or_else(|| {
                    xiaolin_agent::BUILTIN_ENGINE_IDS
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                });
                Some(xiaolin_agent::WebSearchBackend::Builtin {
                    engines: engine_ids,
                })
            }
            _ => None,
        };
        if let Some(backend) = search_backend {
            xiaolin_agent::builtin_tools::register_web_tools(&self.rt.tool_registry, backend);
            tracing::info!(
                backend = ws_cfg.backend.as_str(),
                "web_search tools hot-reloaded"
            );
        } else {
            tracing::info!("web_search backend unconfigured after reload");
        }
        Ok(())
    }

    /// Hot-reload a single channel from the live config. Starts the plugin if config is
    /// valid. If the channel is already running, stops the old instance first.
    pub async fn reload_channel(&self, channel_id: &str) -> anyhow::Result<()> {
        let parsed: XiaoLinConfig = {
            let live = self.cfg.config_live.load();
            serde_json::from_value((**live).clone())?
        };
        let ch = parsed
            .channels
            .get(channel_id)
            .ok_or_else(|| anyhow::anyhow!("channel '{channel_id}' not in config"))?;

        tracing::info!(
            channel_id,
            app_id = ?ch.app_id,
            app_secret_len = ch.app_secret.as_ref().map(|s| s.len()).unwrap_or(0),
            enabled = ?ch.enabled,
            domain = ?ch.domain,
            connection_mode = ?ch.connection_mode,
            reply_mode = ?ch.reply_mode,
            "reload_channel: config snapshot"
        );

        if ch.enabled == Some(false) {
            return Err(anyhow::anyhow!("channel '{channel_id}' is disabled"));
        }

        // Stop and remove existing channel if running
        {
            let mut reg = self.ext.channel_registry.write().await;
            if let Some(old_plugin) = reg.get(channel_id) {
                tracing::info!(channel = channel_id, "stopping existing channel for reload");
                if let Err(e) = old_plugin.stop().await {
                    tracing::warn!(channel = channel_id, error = %e, "failed to stop old channel");
                }
                reg.unregister(channel_id);
            }
        }

        let tx = self.ext.channel_inbound_tx.clone();
        let started = match channel_id {
            "feishu" => {
                if let Some(cfg) = xiaolin_feishu::FeishuPluginConfig::from_channel_config(ch) {
                    let plugin = Arc::new(xiaolin_feishu::FeishuPlugin::new(cfg));
                    plugin.start(tx).await?;
                    self.ext.channel_registry.write().await.register(plugin);
                    tracing::info!("feishu channel hot-reloaded");
                    true
                } else {
                    tracing::warn!("feishu: config missing required fields (appId/appSecret)");
                    false
                }
            }
            "wechat" => {
                let wechat_cfg = xiaolin_wechat::WechatChannelConfig::from_channel_config(ch)
                    .unwrap_or_default();
                let plugin = Arc::new(xiaolin_wechat::WechatPlugin::new(wechat_cfg));
                plugin.start(tx).await?;
                self.ext.channel_registry.write().await.register(plugin);
                tracing::info!("wechat channel hot-reloaded");
                true
            }
            other => return Err(anyhow::anyhow!("unknown channel type: {other}")),
        };
        if !started {
            return Err(anyhow::anyhow!(
                "channel '{channel_id}' config is incomplete — check appId/appSecret"
            ));
        }
        Ok(())
    }

    /// Apply a candidate agent list: validate, then swap [`Self::router`] and refresh
    /// [`Self::last_good_agents`] in one logical step (router swap is a single write).
    pub async fn apply_validated_agent_reload(
        &self,
        mut agents: Vec<AgentConfig>,
    ) -> anyhow::Result<usize> {
        validate_agents_for_reload(&agents)?;
        // Resolve plugin-declared context windows before storing configs.
        {
            let plugin_guard = self.ext.llm_plugin_registry.try_read();
            let plugin_ref = plugin_guard.as_ref().map(|g| &**g).ok();
            xiaolin_agent::patch_agent_context_windows(&mut agents, plugin_ref);
        }
        self.refresh_runtime_agent_providers(&agents);
        let count = agents.len();
        let new_router = AgentRouter::new(agents.clone());
        {
            let mut router = self.rt.router.write().await;
            *router = new_router;
        }
        *self.cfg.last_good_agents.write().await = agents;
        tracing::info!(agent_count = count, "agents hot-reloaded");
        Ok(count)
    }

    pub(crate) fn refresh_runtime_agent_providers(&self, agents: &[AgentConfig]) {
        self.rt.runtime.clear_registered_providers();
        let credentials = self.current_credentials_snapshot();

        // Try to read the plugin registry for plugin-aware provider creation.
        // If the lock is contended we fall back to no-plugin mode.
        let plugin_guard = self.ext.llm_plugin_registry.try_read();
        let plugin_ref = plugin_guard.as_ref().map(|g| &**g).ok();

        let mut registered = 0usize;
        let mut failed = 0usize;
        let mut default_refreshed = false;
        for agent in agents {
            match xiaolin_agent::create_provider_chain_with_plugins(
                &agent.model,
                Some(&credentials),
                plugin_ref,
            ) {
                Ok(provider) => {
                    let provider: Arc<dyn xiaolin_agent::LlmProvider> = Arc::from(provider);
                    if !default_refreshed {
                        self.rt.runtime.set_default_provider(provider.clone());
                        default_refreshed = true;
                    }
                    self.rt.runtime.register_provider(&agent.agent_id, provider);
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
            default_refreshed,
            "agent hot-reload: refreshed runtime provider map"
        );
    }

    pub(crate) fn current_credentials_snapshot(&self) -> xiaolin_core::config::CredentialsConfig {
        let live = self.cfg.config_live.load();
        let credentials = live
            .get("credentials")
            .cloned()
            .and_then(|v| {
                serde_json::from_value::<xiaolin_core::config::CredentialsConfig>(v).ok()
            })
            .unwrap_or_else(|| self.cfg.config.credentials.clone());

        let models_value = live
            .get("models")
            .cloned()
            .unwrap_or_else(|| serde_json::to_value(&self.cfg.config.models).unwrap_or_default());
        let models: std::collections::HashMap<String, xiaolin_core::config::ModelProviderConfig> =
            serde_json::from_value(models_value).unwrap_or_default();

        helpers::merge_model_base_urls_into_credentials(&credentials, &models)
    }
}

#[cfg(any(test, feature = "test-helpers"))]
impl AppState {
    /// Construct a minimal `AppState` backed by temp-dir SQLite databases.
    ///
    /// Skips channels, MCP, plugins, and multi-phase production wiring ([`StateBuilder::build`]).
    /// `provider` is caller-supplied so tests can inject a mock LLM.
    pub async fn for_test(
        provider: Box<dyn xiaolin_agent::LlmProvider>,
        tmp: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let mut config = XiaoLinConfig::default();
        config.memory.enabled = true;
        let agents = vec![helpers::builtin_default_agent(&config)];
        let last_good_agents_init = agents.clone();
        let router = AgentRouter::new(agents.clone());

        let (feedback_store, trajectory_store, skill_store, prompt_distiller) = {
            let target = tmp.join("evolution.db");
            let opts = SqliteConnectOptions::new()
                .filename(&target)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            let evo_pool = SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(opts)
                .await?;
            let fs = FeedbackStore::open(evo_pool.clone()).await?;
            let ts = Arc::new(TrajectoryStore::open(evo_pool.clone()).await?);
            let ss = Arc::new(SkillStore::open(evo_pool.clone()).await?);
            let pd = PromptDistiller::open(evo_pool).await?;
            (fs, ts, ss, pd)
        };

        let runtime = Arc::new({
            let rt = AgentRuntime::new(Arc::from(provider));

            rt.with_skill_store(skill_store.clone())
                .with_trajectory_store(trajectory_store.clone())
        });

        let tool_registry = ToolRegistry::new();
        xiaolin_agent::builtin_tools::register_builtin_tools(&tool_registry);
        let todo_store = xiaolin_agent::builtin_tools::TodoStore::new();
        xiaolin_agent::builtin_tools::register_todo_tools(&tool_registry, todo_store.clone());

        let default_agent_cfg = agents.first().expect("need at least one agent").clone();
        let subagent_manager = Arc::new(xiaolin_agent::SubAgentManager::new(
            runtime.clone(),
            agents,
            xiaolin_core::agent_config::SubAgentPolicy::default(),
            Arc::new(xiaolin_agent::SpawnController::new(
                xiaolin_agent::SpawnConfig::from_policy_fallback(
                    xiaolin_core::agent_config::SubAgentPolicy::default().max_parallel,
                ),
            )),
        ));
        let subagent_tool = xiaolin_agent::SubAgentTool::new(
            subagent_manager.clone(),
            Arc::new(tool_registry.clone()),
            xiaolin_core::agent_config::SubAgentPolicy::default(),
        );
        tool_registry.register(Arc::new(subagent_tool));
        tool_registry.register(Arc::new(xiaolin_agent::SubAgentGetTool::new(
            subagent_manager.clone(),
        )));
        tool_registry.register(Arc::new(xiaolin_agent::SubAgentListTool::new(
            subagent_manager.clone(),
        )));
        tool_registry.register(Arc::new(xiaolin_agent::WaitAgentTool::new(
            subagent_manager.clone(),
        )));
        tool_registry.register(Arc::new(xiaolin_agent::ListAgentsTool::new(
            subagent_manager.clone(),
        )));
        tool_registry.register(Arc::new(xiaolin_agent::GetAgentInfoTool::new(
            subagent_manager.clone(),
        )));

        let db_path = tmp.join("sessions.db");
        let session_store = Arc::new(SessionStore::open(&db_path).await?);
        let event_log = Arc::new(EventLog::new(session_store.pool()));
        event_log.ensure_table().await?;
        let message_bus = Arc::new(MessageBus::new(128));
        for aid in ["main"] {
            let mut rx = message_bus.register(aid).await;
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
        }
        xiaolin_agent::builtin_tools::register_session_tools(
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

        let channel_registry = ChannelRegistry::new();

        let mut context_engine =
            xiaolin_context::ContextEngine::new(xiaolin_context::DEFAULT_COMPACTION_THRESHOLD);
        context_engine.add_hook(Arc::new(xiaolin_context::CompactionHook::new(
            xiaolin_context::CompactionStrategy::default(),
        )));
        context_engine.add_hook(Arc::new(xiaolin_context::SystemReminderHook::default()));

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
        let cron_store = CronJobStore::open(cron_pool.clone()).await?;
        let notification_store =
            crate::notification_store::NotificationStore::open(cron_pool).await?;

        let budget_tracker = BudgetTracker::new(None);

        let (ws_broadcast, _) = tokio::sync::broadcast::channel::<String>(256);

        let tool_orchestrator =
            Arc::new(xiaolin_agent::ToolOrchestrator::new());

        let config_live_val = serde_json::to_value(&config).unwrap_or_default();
        let channel_inbound_tx = {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            tx
        };

        let tool_registry = {
            let reg = Arc::new(tool_registry);
            xiaolin_agent::builtin_tools::register_tool_search(&reg);
            reg
        };
        let prompt_guard = Arc::new(xiaolin_security::PromptGuard::new());
        let context_engine = Arc::new(context_engine);

        let executor: Arc<dyn xiaolin_session_actor::TurnExecutor> =
            Arc::new(xiaolin_agent::RuntimeTurnExecutor {
                runtime: runtime.clone(),
                config: default_agent_cfg,
                tool_registry: tool_registry.clone(),
                llm_override: None,
                session_store: Some(session_store.clone()),
                mode_registry: None,
                todo_store: None,
                plan_file_store: None,
                stream_event_tx: None,
                subagent_manager: None,
                tool_orchestrator: None,
            });
        let session_manager = Arc::new(xiaolin_session_actor::SessionManager::new(executor));

        Ok(Self {
            cfg: ConfigState {
                config: Arc::new(config),
                config_live: Arc::new(ArcSwap::new(Arc::new(config_live_val))),
                runtime_route_bindings: Arc::new(tokio::sync::RwLock::new(Vec::new())),
                last_good_agents: Arc::new(tokio::sync::RwLock::new(last_good_agents_init)),
            },
            rt: RuntimeState {
                router: Arc::new(tokio::sync::RwLock::new(router)),
                runtime: runtime.clone(),
                tool_registry: tool_registry.clone(),
                base_skill_registry: Arc::new(ArcSwap::new(Arc::new(SkillRegistry::new()))),
                agent_skill_registries: Arc::new(ArcSwap::new(Arc::new(
                    std::collections::HashMap::new(),
                ))),
                workspaces: Arc::new(std::collections::HashMap::new()),
                prompt_guard: prompt_guard.clone(),
                session_modes: xiaolin_agent::builtin_tools::SessionModeRegistry::new(),
                todo_store,
                plan_file_store: xiaolin_agent::builtin_tools::PlanFileStore::default(),
            },
            store: StorageState {
                session_store: session_store.clone(),
                event_log: event_log.clone(),
                cron_store: Arc::new(cron_store),
                cron_wake: Arc::new(tokio::sync::Notify::new()),
                notification_store: Arc::new(notification_store),
                feedback_store: Arc::new(feedback_store),
                prompt_distiller: Arc::new(prompt_distiller),
                trajectory_store,
                skill_store,
                context_engine: context_engine.clone(),
            },
            mem: MemoryState {
                agent_episodic: Arc::new(test_ep_map),
                agent_semantic: Arc::new(test_sem_map),
                embedding_provider: None,
            },
            ext: ExtensionState {
                channel_registry: Arc::new(tokio::sync::RwLock::new(channel_registry)),
                message_bus,
                mcp_status: Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::new()))),
                mcp_handles: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                channel_inbound_tx,
                llm_plugin_registry: Arc::new(tokio::sync::RwLock::new(
                    xiaolin_agent::LlmPluginRegistry::new(),
                )),
                chat_locks: Arc::new(dashmap::DashMap::new()),
                chat_cancels: Arc::new(dashmap::DashMap::new()),
                chat_model_overrides: Arc::new(dashmap::DashMap::new()),
                wechat_login_sessions: Arc::new(dashmap::DashMap::new()),
            },
            obs: ObserveState {
                metrics_collector: Arc::new(xiaolin_observe::MetricsCollector::new()),
                budget_tracker: Arc::new(budget_tracker),
                model_router: None,
            },
            strm: StreamState {
                stream_event_tx: Arc::new(DashMap::new()),
                tool_orchestrator,
                ws_broadcast,
                subagent_manager,
                session_manager: session_manager.clone(),
            },
            svc: SharedServices {
                runtime,
                tool_registry,
                session_store,
                event_log,
                context_engine,
                prompt_guard,
                session_manager,
            },
        })
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    use xiaolin_agent::{CompletionParams, LlmProvider};
    use xiaolin_core::config::XiaoLinConfig;
    use xiaolin_core::types::{
        ChatChoice, ChatMessage, ChatRequest, ChatResponse, Role, StreamDelta,
    };

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
                    ..Default::default()
                    },
                    finish_reason: Some("stop".into()),
                }],
                usage: None,
            })
        }

        async fn chat_completion_stream(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[test]
    fn validate_rejects_empty_agent_list() {
        assert!(validate_agents_for_reload(&[]).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let a = helpers::builtin_default_agent(&XiaoLinConfig::default());
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

        let mut cfg = XiaoLinConfig::default();
        cfg.paths.agents_dir = Some(tmp.to_string_lossy().to_string());

        let agents = helpers::load_agents(&cfg).expect("load_agents should succeed");
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

        let mut cfg = XiaoLinConfig::default();
        cfg.paths.agents_dir = Some(tmp.to_string_lossy().to_string());
        cfg.agents.defaults.model = Some("dashscope/qwen3.5-plus".to_string());

        let agents = helpers::load_agents(&cfg).expect("load_agents should succeed");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].model.provider, "dashscope");
        assert_eq!(agents[0].model.model, "qwen3.5-plus");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn reload_with_bad_config_keeps_previous_router() {
        let tmp =
            std::env::temp_dir().join(format!("fcgw_reload_{}", uuid::Uuid::new_v4().simple()));
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
        assert!(state.rt.router.read().await.resolve(&req).is_ok());

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
            state.rt.router.read().await.resolve(&req).is_ok(),
            "router must still resolve after failed reload"
        );
    }
}
