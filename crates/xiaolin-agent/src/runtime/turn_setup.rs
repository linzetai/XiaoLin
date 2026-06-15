use std::sync::Arc;

use xiaolin_core::tool::{ToolDefinition, ToolProfile};
use xiaolin_core::types::Role;
use xiaolin_protocol::{ExecutionMode, TurnId};

use super::agent_context::AgentContext;
use super::approval_cache::ApprovalCache;
use super::cache_break_detection::CacheBreakDetector;
use super::context_assembly;
use super::dispatcher::ToolDispatcher;
use super::file_persistence::SessionFileTracker;
use super::observer::RuntimeObserver;
use super::orchestrator;
use super::permissions::DenialTracker;
use super::query_deps::ProductionDeps;
use super::query_state::QueryLoopState;
use super::runtimes;
use super::runtime_services;
use super::session_memory;
use super::task_decomposer;
use super::token_budget;
use super::tool_executor::filter_tool_definitions;
use super::turn_state::{TurnMutableState, TurnServices};
use super::undo_engine;
use super::validation_pipeline;
use super::{
    create_tool_result_storage, build_skip_tool_names, inject_system_block,
    AgentRuntime,
};
use crate::llm::LlmProvider;

/// Performs the one-time setup for an agent turn.
///
/// Returns the mutable state and immutable service dependencies needed
/// for the iterative agent loop.
pub(crate) async fn setup_turn(
    runtime: Arc<AgentRuntime>,
    ctx: &AgentContext,
) -> anyhow::Result<(TurnMutableState, TurnServices)> {
    let config = &ctx.config;
    let request = &ctx.request;
    let tool_registry = &ctx.tool_registry;
    let llm_override = &ctx.llm_override;
    let mode_state = &ctx.mode_state;
    let session_store = &ctx.session_store;
    let todo_store = &ctx.todo_store;
    let goal_store = &ctx.goal_store;
    let cancel_token = ctx.cancel_token.clone();

    let turn_id = TurnId::generate();
    let stream_start = std::time::Instant::now();

    // --- Message building ---
    let t0 = std::time::Instant::now();
    let mut messages = runtime.build_messages(ctx);
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: build_messages (stream)"
    );

    // --- Skill injection ---
    let t0 = std::time::Instant::now();
    let mut injected_skill_ids: Vec<String> = Vec::new();
    if let Err(e) = runtime
        .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
        .await
    {
        tracing::warn!(error = %e, "skill injection skipped (stream)");
    }
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "perf: inject_relevant_skills (stream)"
    );

    // --- Context Assembly: inject project hints ---
    if let Some(ref wd) = request.work_dir {
        let hints = context_assembly::detect_project_hints(std::path::Path::new(wd));
        if !hints.is_empty() {
            let hints_block = format!(
                "\n─── Project Context ───\n{}\n───────────────────────\n",
                hints
                    .iter()
                    .map(|h| format!("• {}", h))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            inject_system_block(&mut messages, &hints_block);
            tracing::info!(
                hint_count = hints.len(),
                "context_assembly: project hints injected"
            );
        }
    }

    // --- Extract last user message for downstream injections ---
    let last_user_msg = request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.text_content())
        .unwrap_or_default();

    // --- Task Decomposer ---
    if last_user_msg.len() >= 200 {
        let decomp_provider = runtime.provider();
        let decomp_config = task_decomposer::TaskDecomposerConfig {
            model: config.model.model.clone(),
            ..Default::default()
        };
        if let Some(decomp) =
            task_decomposer::decompose_task(&decomp_provider, &last_user_msg, &decomp_config)
                .await
        {
            if let Some(block) = task_decomposer::format_decomposition_for_prompt(&decomp) {
                inject_system_block(&mut messages, &block);
                tracing::info!(
                    task_type = decomp.task_type.as_str(),
                    steps = decomp.steps.len(),
                    "task_decomposer: plan injected"
                );
            }
        }
    }

    // --- Tool definitions ---
    let t0 = std::time::Instant::now();
    let mode_profile = mode_state
        .as_ref()
        .map(|ms| match ms.current_mode() {
            ExecutionMode::Plan => ToolProfile::plan_mode(),
            _ => ToolProfile::default(),
        })
        .unwrap_or_default();
    let extra_tool_defs: Vec<ToolDefinition> = request
        .tools
        .as_deref()
        .unwrap_or(&[])
        .to_vec();
    let mut all_tool_defs = tool_registry.definitions_with_profile(&mode_profile);
    all_tool_defs.extend(extra_tool_defs.iter().cloned());
    let tool_defs = filter_tool_definitions(&all_tool_defs, config);
    let tool_defs_json_chars: usize = tool_defs
        .iter()
        .map(|td| serde_json::to_string(td).map(|s| s.len()).unwrap_or(0))
        .sum();
    let tool_defs_est_tokens = tool_defs_json_chars / 4;
    tracing::info!(
        elapsed_ms = t0.elapsed().as_millis() as u64,
        count = tool_defs.len(),
        json_chars = tool_defs_json_chars,
        est_tokens = tool_defs_est_tokens,
        "perf: tool_definitions (stream)"
    );

    // --- Model parameters ---
    let temperature = request.temperature.unwrap_or(config.model.temperature);
    let model = request
        .model
        .as_deref()
        .unwrap_or(&config.model.model)
        .to_string();
    let max_tokens = request.max_tokens.or(config.model.max_tokens).or_else(|| {
        let inferred = xiaolin_context::infer_output_limit_from_model(&model);
        if inferred > 0 { Some(inferred) } else { None }
    });

    // --- QueryLoopState ---
    let max_iterations = config.behavior.max_tool_calls_per_turn;
    let mut state = QueryLoopState::new(max_iterations);
    let tool_storage = create_tool_result_storage(request.session_id.as_deref());
    let skip_tool_names = build_skip_tool_names(tool_registry);

    // Set session_id on goal store
    if let (Some(gs), Some(sid)) = (goal_store, request.session_id.as_deref()) {
        gs.set_session_id(sid.to_string()).await;
    }

    // Load session memory
    if let (Some(store), Some(sid)) = (session_store, request.session_id.as_deref()) {
        if let Some(mem) = session_memory::load_session_memory(store.as_ref(), sid).await {
            state.session_memory = Some(mem);
        }
    }

    // --- Context window ---
    let context_window = config.model.context_window.unwrap_or(
        xiaolin_context::infer_context_window_from_model(&config.model.model),
    );

    // --- ProductionDeps ---
    let provider_for_deps: Arc<dyn LlmProvider> = match &llm_override {
        Some(p) => p.clone(),
        None => runtime.resolve_provider(&config.agent_id)?,
    };
    let pipeline_config = xiaolin_context::PipelineConfig {
        snip_max_tokens: context_window as usize,
        reactive_target_tokens: (context_window as f64 * 0.8) as usize,
        ..Default::default()
    };
    let auto_compact_enabled = pipeline_config.enable_auto_compact;
    let compact_pipeline = xiaolin_context::ContextPipeline::new(pipeline_config);
    let deps = ProductionDeps::new(provider_for_deps, compact_pipeline);

    // --- Content replacement state ---
    let replacement_state = AgentRuntime::load_or_create_replacement_state(
        session_store,
        request.session_id.as_deref(),
        &request.messages,
    )
    .await;

    // --- RuntimeServices ---
    let abort_token = tokio_util::sync::CancellationToken::new();
    let workspace_dir = request.work_dir.as_ref().map(std::path::Path::new);
    let budget_limit = config.behavior.budget_limit_usd;
    let services = runtime_services::RuntimeServices::from_config_with_store(
        workspace_dir,
        budget_limit,
        abort_token,
        ctx.cost_store.clone(),
        request.session_id.as_ref().map(|s| s.to_string()),
    );

    // --- Validation, Undo, Approval, Dispatcher ---
    let validation_pipeline = validation_pipeline::ValidationPipeline::default();
    let undo_engine = undo_engine::UndoEngine::new(undo_engine::UndoEngineConfig::default());
    let approval_cache = ApprovalCache::new();
    let denial_tracker = DenialTracker::new();

    let runtime_registry = ctx
        .runtime_registry
        .as_ref()
        .map(Arc::clone)
        .unwrap_or_else(|| Arc::new(runtimes::register_default_runtimes()));
    let orch = ctx
        .orchestrator
        .as_ref()
        .map(Arc::clone)
        .unwrap_or_else(|| Arc::new(orchestrator::ToolOrchestrator::new()));
    let dispatcher = ToolDispatcher::new(
        Arc::clone(tool_registry),
        Arc::clone(&runtime_registry),
        orch,
    );

    // --- Observer, Cache, FileTracker ---
    let runtime_observer = RuntimeObserver::new(
        request.session_id.as_deref().unwrap_or("anonymous"),
        &config.agent_id,
        None,
    );
    let cache_detector = CacheBreakDetector::new();
    let file_tracker = SessionFileTracker::new();

    // --- Magic Docs injection ---
    {
        let keywords: Vec<&str> = last_user_msg
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .take(10)
            .collect();
        if !keywords.is_empty() {
            let docs_content = services.query_magic_docs(&keywords, 2000);
            if !docs_content.is_empty() {
                let docs_block = format!(
                    "\n─── Relevant Documentation ───\n{}\n──────────────────────────────\n",
                    docs_content
                );
                inject_system_block(&mut messages, &docs_block);
                tracing::info!(
                    chars = docs_content.len(),
                    "magic_docs: documentation injected"
                );
            }
        }
    }

    // --- Goal store initial state ---
    let last_seen_goal_id: Option<String> = if let Some(ref gs) = goal_store {
        gs.get_current().await.map(|g| g.id)
    } else {
        None
    };

    // --- Assemble outputs ---
    let budget_tracker = token_budget::resolve_turn_budget(
        &last_user_msg,
        request.session_id.as_deref(),
    )
    .map(token_budget::BudgetTracker::new);

    if let Some(ref bt) = budget_tracker {
        let budget_block = format!(
            "\n─── Token Budget ───\n\
             The user set a token budget of {} tokens as a safety ceiling. \
             Complete your task naturally — stop when done, do not pad output. \
             If you approach the budget limit, the system will ask you to wrap up.\n\
             ────────────────────\n",
            bt.budget.target_tokens
        );
        inject_system_block(&mut messages, &budget_block);
        tracing::info!(
            target_tokens = bt.budget.target_tokens,
            "token_budget: budget injected into system prompt"
        );
    }

    let registry_version_at_setup = tool_registry.version();
    let mutable_state = TurnMutableState {
        messages,
        max_tokens,
        query_loop: state,
        replacement_state,
        undo_engine,
        approval_cache,
        denial_tracker,
        cache_detector,
        file_tracker,
        last_seen_goal_id,
        had_tool_calls_this_round: false,
        had_progress_this_round: false,
        injected_skill_ids,
        trajectory_steps: Vec::new(),
        budget_tracker,
        token_budget_reached: false,
        tool_defs,
        tool_defs_est_tokens,
        registry_version_at_setup,
        extra_tool_defs,
    };

    let turn_services = TurnServices {
        runtime,
        turn_id,
        stream_start,
        model,
        temperature,
        context_window,
        auto_compact_enabled,
        config: Arc::new(config.clone()),
        session_id: request.session_id.clone().map(Into::into),
        work_dir: request.work_dir.clone(),
        last_user_msg: last_user_msg.to_string(),
        mode_state: mode_state.clone(),
        runtime_registry,
        tool_registry: tool_registry.clone(),
        session_store: session_store.clone(),
        todo_store: todo_store.clone(),
        goal_store: goal_store.clone(),
        plan_file_path: crate::builtin_tools::plan_mode::current_plan_context()
            .map(|pc| pc.store.plan_path(&pc.session_id))
            .or_else(|| ctx.plan_file_path.clone()),
        step_tx: ctx.step_tx.clone().expect("AgentContext.step_tx must be set for streaming execution"),
        event_tx: ctx.event_tx.clone().expect("AgentContext.event_tx must be set for streaming execution"),
        approval_strategy: ctx.approval_strategy.clone(),
        interaction_handle: ctx.interaction_handle.clone(),
        behavior_overrides: ctx.behavior_overrides.clone(),
        deps,
        services,
        dispatcher,
        tool_storage,
        skip_tool_names,
        validation_pipeline,
        runtime_observer,
        message_queue: ctx.message_queue.clone(),
        cancel_token,
    };

    Ok((mutable_state, turn_services))
}
