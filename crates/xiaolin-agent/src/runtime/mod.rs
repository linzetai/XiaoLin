use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::types::{ChatMessage, ChatRequest, ChatResponse, Role};
use xiaolin_protocol::{AgentEvent, ErrorCode, ExecutionMode, TokenUsage, TurnId, TurnSummary};

use prompt_engine::{PromptContext, PromptEngine, PromptSection};
#[cfg(not(feature = "self-iter"))]
use stream_engine::ToolCallTrace;
use xiaolin_evolution::{
    format_candidate_skills_for_prompt, format_skills_for_prompt, SkillStatus, SkillStore,
    TrajectoryStore,
};
#[cfg(feature = "self-iter")]
use xiaolin_self_iter::{SelfIterEngine, ToolCallTrace};

use crate::llm::LlmProvider;
use base64::Engine as _;

mod accumulator;
pub mod agent_context;
pub mod agent_step;
pub mod api_errors;
pub mod approval_cache;
pub mod cache_break_detection;
#[allow(dead_code)] // TODO(integrate): assemble related files at query start
pub mod context_assembly;
pub(crate) mod context_budget;
pub(crate) mod context_compressor;
pub(crate) mod context_projection;
pub mod cost_tracker;
pub(crate) mod end_turn;
pub mod file_persistence;
pub(crate) mod iteration_check;
pub(crate) mod llm_call;
pub(crate) mod plan_arg_interceptor;
pub(crate) mod post_tool;
pub mod runtimes;
mod stream_loop;
pub(crate) mod tool_round;
pub(crate) mod turn_loop;
pub(crate) mod turn_setup;
pub(crate) mod turn_state;
pub use xiaolin_tools_fs::file_state_cache;
pub mod dispatcher;
pub(crate) mod goal_prompts;
pub mod hook_config;
pub mod hook_events;
pub mod hook_executor;
#[allow(dead_code)]
pub mod lsp_actions;
pub mod magic_docs;
#[allow(dead_code)]
pub mod memory_selection;
pub mod message_injection;
pub mod mode_attachments;
pub mod model_critic;
pub(crate) mod observer;
pub mod orchestrator;
pub mod permissions;
pub mod post_compact_restore;
mod prompt_builder;
pub mod prompt_engine;
pub mod prompt_sections;
#[allow(dead_code)] // TODO(integrate): wire into AgentEvent::Suggestions
pub mod prompt_suggestion;
pub(crate) mod query_deps;
pub mod query_engine;
mod query_state;
pub mod retry;
pub(crate) mod runtime_quality;
pub(crate) mod runtime_services;
mod session_memory;
#[allow(dead_code)] // TODO(integrate): side-query tool handle for auxiliary LLM calls
pub mod side_query;
mod stop_hooks;
mod stream_engine;
pub mod streaming_tool_executor;
pub mod task_decomposer;
pub(crate) mod token_budget;
mod tool_executor;
pub mod tool_result_storage;
mod trajectory;
pub mod undo_engine;
mod unified_compact;
#[allow(dead_code)]
pub mod validation_pipeline;

pub use message_injection::{
    append_to_tier2_system, inject_user_context, merge_leading_system_into_tier2,
    push_tier2_system_prefix,
};
pub use prompt_builder::{
    build_active_runs_context, build_subagent_prompt_block, ActiveRunSummary, SubAgentPromptContext,
};

use prompt_builder::SKILL_MANAGEMENT_GUIDANCE;
use query_state::QueryLoopState;
#[allow(deprecated)]
use tool_executor::truncate_tool_result_output_with_limit;
use tool_result_storage::{
    reconstruct_state, ContentReplacementState, ToolResultEntry, ToolResultStorage,
    MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
};
use trajectory::last_user_turn_text;
use xiaolin_session::tool_output_store::ToolOutputAssetStore;

fn push_system_messages_from_prompt(messages: &mut Vec<ChatMessage>, system_text: &str) {
    use prompt_engine::{CACHE_TIER1_BOUNDARY, CACHE_TIER2_BOUNDARY, DYNAMIC_BOUNDARY};

    if let Some((tier1_raw, after_t1)) = system_text.split_once(CACHE_TIER1_BOUNDARY) {
        let tier1 = tier1_raw.trim_end();
        if let Some((tier2_raw, trailing)) = after_t1.split_once(CACHE_TIER2_BOUNDARY) {
            let tier2 = tier2_raw.trim_end();
            let trailing = trailing.trim();
            if !tier1.is_empty() {
                messages.push(ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(tier1.to_string())),
                    ..Default::default()
                });
            }
            let mut tier2_text = tier2.to_string();
            if !trailing.is_empty() {
                if !tier2_text.is_empty() {
                    tier2_text.push_str("\n\n");
                }
                tier2_text.push_str(trailing);
            }
            if !tier2_text.is_empty() {
                messages.push(ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(tier2_text)),
                    ..Default::default()
                });
            }
            return;
        }
    }

    if let Some((static_part_raw, dynamic_part_raw)) = system_text.split_once(DYNAMIC_BOUNDARY) {
        let static_part = static_part_raw.trim_end();
        let dynamic_part = dynamic_part_raw.trim_start();
        if !static_part.is_empty() {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(static_part.to_string())),
                ..Default::default()
            });
        }
        if !dynamic_part.is_empty() {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(dynamic_part.to_string())),
                ..Default::default()
            });
        }
        return;
    }

    if !system_text.trim().is_empty() {
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(system_text.to_string())),
            ..Default::default()
        });
    }
}

/// Track restoration state from tool execution.
/// Extracts file reads, skill invocations, and plan content for post-compact recovery.
fn track_restoration_state(
    restoration_state: &mut post_compact_restore::RestorationState,
    tool_name: &str,
    arguments: &str,
    output: &str,
    success: bool,
) {
    // Only track successful tool executions
    if !success {
        return;
    }

    match tool_name {
        // Track file reads
        "Read" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                if let Some(path) = args.get("file_path").and_then(|p| p.as_str()) {
                    restoration_state.add_file(std::path::PathBuf::from(path), output.to_string());
                }
            }
        }
        // Track skill invocations
        "Skill" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                if let Some(skill_name) = args.get("skill").and_then(|s| s.as_str()) {
                    restoration_state.add_skill(
                        skill_name.to_string(),
                        std::path::PathBuf::from(format!(".claude/skills/{}.md", skill_name)),
                        output.to_string(),
                    );
                }
            }
        }
        // Track plan mode - when entering plan mode, mark it
        "EnterPlanMode" => {
            restoration_state.is_plan_mode = true;
        }
        "ExitPlanMode" => {
            restoration_state.is_plan_mode = false;
            restoration_state.clear_plan();
        }
        // Track deferred tool activations via tool_search select mode
        "tool_search" => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
                if parsed.get("activated").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(tool_name) = parsed.get("tool").and_then(|v| v.as_str()) {
                        let description = parsed
                            .pointer("/schema/description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("(no description)")
                            .to_string();
                        restoration_state
                            .record_tool_activation(tool_name.to_string(), description);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Create a ToolResultStorage for the current invocation session.
///
/// - With `session_id`: uses `~/.xiaolin/sessions/<session_id>/` so tool results
///   persist across process restarts and can be recovered on session resume.
/// - Without `session_id`: uses an ephemeral temp directory that lives only as
///   long as the current process.
fn create_tool_result_storage(session_id: Option<&str>) -> ToolResultStorage {
    let session_dir = match session_id {
        Some(sid) => dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".xiaolin")
            .join("sessions")
            .join(sid),
        None => std::env::temp_dir()
            .join("xiaolin_sessions")
            .join(format!("ephemeral_{}", std::process::id())),
    };
    ToolResultStorage::new(session_dir)
}

/// Build the set of tool names whose results should skip budget enforcement.
/// These are tools with `max_result_size_chars() == usize::MAX`.
fn build_skip_tool_names(
    tool_registry: &xiaolin_core::tool::ToolRegistry,
) -> std::collections::HashSet<String> {
    tool_registry
        .tool_names()
        .into_iter()
        .filter(|name| {
            tool_registry
                .get(name)
                .map(|t| t.max_result_size_chars() == usize::MAX)
                .unwrap_or(false)
        })
        .collect()
}

fn classify_stream_error_code(message: &str) -> Option<ErrorCode> {
    Some(ErrorCode::classify(message))
}

/// Process a tool result: persist large outputs to the new ToolOutputAssetStore
/// when available, fall back to ToolResultStorage, then truncation.
///
/// When `tool_output_store` is Some, large outputs (> persistence_threshold) are
/// created as handle-backed assets before any truncation occurs. Small outputs
/// remain inline as before.
#[allow(deprecated)]
async fn process_tool_output(
    storage: &ToolResultStorage,
    tool_output_store: Option<&std::sync::Arc<ToolOutputAssetStore>>,
    session_id: Option<&str>,
    turn_id: &str,
    tool_name: &str,
    call_id: &str,
    output: String,
    max_result_size_chars: usize,
) -> String {
    use xiaolin_session::tool_output_store::CreateAssetInput;

    let threshold = tool_result_storage::get_persistence_threshold(max_result_size_chars);
    let is_large = output.len() > threshold;

    // Phase 2: route large outputs through the handle-based asset store.
    if is_large {
        if let (Some(store), Some(sid)) = (tool_output_store, session_id) {
            let storage_root = tool_result_storage::session_dir(sid);
            let input = CreateAssetInput {
                session_id: sid.to_string(),
                turn_id: turn_id.to_string(),
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: String::new(),
                success: true,
                output: output.clone(),
                storage_root,
                size_config: Default::default(),
            };
            match store.create_asset(input).await {
                Ok(handle) => {
                    // Use the projector registry to produce a typed projection
                    let msg = match store.get_asset(handle.as_str(), sid).await {
                        Ok(asset) => {
                            let projection =
                                xiaolin_session::tool_output_projector::PROJECTOR_REGISTRY
                                    .project(&asset, &output);
                            projection.format()
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, handle = handle.as_str(), "get_asset failed after create, using fallback message");
                            let (preview, has_more) = tool_result_storage::generate_preview(
                                &output,
                                tool_result_storage::PREVIEW_SIZE_BYTES,
                            );
                            tool_result_storage::build_handle_replacement_message(
                                handle.as_str(),
                                tool_name,
                                output.len(),
                                &preview,
                                has_more,
                                "large",
                            )
                        }
                    };
                    tracing::info!(
                        handle = handle.as_str(),
                        tool = tool_name,
                        bytes = output.len(),
                        "ToolOutputAssetStore: created handle-backed asset"
                    );
                    return msg;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, tool = tool_name, "ToolOutputAssetStore::create_asset failed, falling back to legacy");
                }
            }
        }
    }

    // Fallback: legacy ToolResultStorage path
    match storage.process_result(tool_name, call_id, &output, threshold) {
        Ok(Some(replacement)) => replacement,
        Ok(None) => output,
        Err(e) => {
            tracing::warn!(error = %e, tool = tool_name, "ToolResultStorage failed, falling back to truncation");
            truncate_tool_result_output_with_limit(
                &output,
                tool_name,
                "",
                Some(max_result_size_chars),
            )
        }
    }
}

/// Apply enforce_per_message_budget on messages before sending to LLM.
/// Modifies messages in-place by replacing oversized tool results with previews.
/// Returns any newly created replacement records for session persistence.
fn apply_message_budget(
    storage: &ToolResultStorage,
    messages: &mut [xiaolin_core::types::ChatMessage],
    state: &mut ContentReplacementState,
    skip_tool_names: &std::collections::HashSet<String>,
) -> Vec<tool_result_storage::ContentReplacementRecord> {
    let mut tool_entries: Vec<ToolResultEntry> = Vec::new();
    let mut entry_indices: Vec<usize> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == xiaolin_core::types::Role::Tool {
            if let Some(content) = msg.text_content() {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                let tool_name = msg.name.clone().unwrap_or_default();
                tool_entries.push(ToolResultEntry {
                    tool_use_id,
                    tool_name,
                    content: content.to_string(),
                });
                entry_indices.push(i);
            }
        }
    }

    if tool_entries.is_empty() {
        return Vec::new();
    }

    let result = storage.enforce_per_message_budget(
        tool_entries,
        state,
        skip_tool_names,
        MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
    );

    if result.replacements.is_empty() {
        return Vec::new();
    }

    for &idx in &entry_indices {
        let msg = &messages[idx];
        if let Some(tool_call_id) = &msg.tool_call_id {
            if let Some(replacement) = result.replacements.get(tool_call_id) {
                messages[idx].content = Some(serde_json::Value::String(replacement.clone()));
            }
        }
    }

    if !result.newly_replaced.is_empty() {
        tracing::info!(
            count = result.newly_replaced.len(),
            "Per-message budget: persisted tool results"
        );
    }

    result.newly_replaced
}

/// Build ChatMessage content for a tool result. When the result carries images,
/// constructs a multimodal content array so the LLM can visually interpret them.
fn tool_result_content(text: &str, result: &xiaolin_core::tool::ToolResult) -> serde_json::Value {
    if result.images.is_empty() {
        return serde_json::Value::String(text.to_string());
    }
    let mut parts = vec![serde_json::json!({"type": "text", "text": text})];
    for img in &result.images {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&img.data);
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{b64}", img.mime_type)
            }
        }));
    }
    serde_json::Value::Array(parts)
}

/// Execution result containing the final response and tool-call trace.
pub struct ExecutionResult {
    pub response: ChatResponse,
    pub tool_calls_made: u32,
    pub iterations: u32,
}

fn make_turn_summary(
    turn_id: &TurnId,
    state: &QueryLoopState,
    stream_start: std::time::Instant,
    context_window: u32,
) -> TurnSummary {
    TurnSummary {
        turn_id: turn_id.clone(),
        tool_calls_made: state.total_tool_calls,
        iterations: state.iteration,
        usage: state.build_usage().map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            cached_input_tokens: 0,
        }),
        elapsed_ms: stream_start.elapsed().as_millis() as u64,
        context_tokens: Some(state.last_estimated_tokens as u32),
        context_window: Some(context_window),
    }
}

/// Build recovery guidance from a streak of consecutive tool failures.
///
/// Returns `None` if the streak is empty. Otherwise produces actionable
/// suggestions tailored to the failing tool categories.
pub(crate) fn format_basic_recovery_guidance(failure_streak: &[ToolCallTrace]) -> Option<String> {
    if failure_streak.is_empty() {
        return None;
    }

    let mut tool_errors: Vec<String> = Vec::new();
    let mut seen_tools = std::collections::HashSet::new();
    for trace in failure_streak {
        let err = trace.error.as_deref().unwrap_or("unknown error");
        let truncated = if err.len() > 150 {
            format!("{}...", &err[..err.floor_char_boundary(150)])
        } else {
            err.to_string()
        };
        tool_errors.push(format!("- `{}`: {}", trace.tool_name, truncated));
        seen_tools.insert(trace.tool_name.as_str());
    }

    let mut guidance = format!(
        "The following tool calls have failed consecutively:\n{}\n\n",
        tool_errors.join("\n")
    );

    guidance.push_str("Before retrying, consider:\n");
    for tool in &seen_tools {
        match *tool {
            "read_file" | "list_dir" | "list_directory" =>
                guidance.push_str("- File/path errors: verify the path exists, check spelling, use `glob` or `list_dir` to discover correct paths\n"),
            "shell_exec" | "shell" | "run_command" =>
                guidance.push_str("- Command errors: check command syntax, verify required tools are installed, try simpler alternatives\n"),
            "write_file" | "edit_file" | "apply_patch" | "multi_edit" =>
                guidance.push_str("- Write errors: ensure the target directory exists, check permissions, verify the file content/diff is correct\n"),
            "grep" | "ripgrep" =>
                guidance.push_str("- Search errors: simplify the pattern, check regex syntax, try broader search scope\n"),
            _ =>
                guidance.push_str(&format!("- `{tool}` errors: review the error message carefully and try a different approach\n")),
        }
    }
    guidance.push_str("\nDo NOT repeat the same failing calls. Try an alternative approach or explain the issue to the user.");

    Some(guidance)
}

/// Inject tool recovery guidance into the last user message as system context.
pub(crate) fn inject_tool_recovery_guidance(messages: &mut Vec<ChatMessage>, guidance: &str) {
    let block = format!(
        "---\n[Tool execution recovery — review before your next tool_calls]\n{guidance}\n---"
    );
    inject_user_context(messages, &block);
}

/// When adding tools with file path parameters, keep this list in sync.
const PATH_PARAM_KEYS: &[&str] = &["path", "file_path", "file", "target_path", "filename"];

/// Extract file path from tool arguments JSON.
fn extract_file_path_from_args(arguments: &str) -> Option<std::path::PathBuf> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    PATH_PARAM_KEYS
        .iter()
        .find_map(|key| v.get(*key).and_then(|p| p.as_str()))
        .map(std::path::PathBuf::from)
}

/// Extract all file paths affected by a file tool call.
pub(crate) fn extract_file_paths_from_args(
    tool_name: &str,
    arguments: &str,
) -> Vec<std::path::PathBuf> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };

    match tool_name {
        "multi_edit" => v
            .get("edits")
            .and_then(|e| e.as_array())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        PATH_PARAM_KEYS
                            .iter()
                            .find_map(|key| entry.get(*key).and_then(|p| p.as_str()))
                            .map(std::path::PathBuf::from)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => extract_file_path_from_args(arguments).into_iter().collect(),
    }
}

/// Manages the execution of a single agent invocation, including
/// the tool-calling loop: LLM → tool_calls → execute → inject result → repeat.
/// Internal key for the default/fallback provider inside `agent_providers`.
const DEFAULT_PROVIDER_KEY: &str = "";

pub struct AgentRuntime {
    agent_providers: ArcSwap<HashMap<String, Arc<dyn LlmProvider>>>,
    prompt_engine: PromptEngine,
    #[cfg(feature = "self-iter")]
    self_iter_engine: Option<Arc<SelfIterEngine>>,
    #[cfg(feature = "self-iter")]
    self_iter_max_recovery_attempts: u32,
    skill_store: ArcSwap<Option<Arc<SkillStore>>>,
    skill_usage_store: ArcSwap<Option<Arc<xiaolin_core::skill_usage::SkillUsageStore>>>,
    /// Live skills deny list (synced from gateway `config_live.skills.deny`).
    skills_deny: ArcSwap<Vec<String>>,
    /// Live skills allow list (synced from gateway `config_live.skills.allow`).
    skills_allow: ArcSwap<Vec<String>>,
    /// Live skills context budget percent (synced from gateway `config_live.skills.contextBudgetPercent`).
    skills_context_budget_percent: ArcSwap<u8>,
    trajectory_store: ArcSwap<Option<Arc<TrajectoryStore>>>,
    cached_runtime_registry: Arc<runtimes::RuntimeRegistry>,
    /// Last-seen `ToolRegistry::mcp_instructions_version()`. When the live
    /// registry version differs, the memoized `mcp_instructions` prompt section
    /// is invalidated (event-driven, see §5.2). `u64::MAX` forces a sync on the
    /// first build.
    last_mcp_instructions_version: std::sync::atomic::AtomicU64,
    self_handle: std::sync::OnceLock<std::sync::Weak<Self>>,
}

impl AgentRuntime {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        let mut initial = HashMap::new();
        initial.insert(DEFAULT_PROVIDER_KEY.to_string(), provider);

        // Kick off background symbol indexing if we're in a workspace.
        if let Ok(root) = std::env::current_dir() {
            let index = crate::symbol_index::SymbolIndex::global().clone();
            crate::symbol_index::start_background_scan(root.clone(), index.clone());
            crate::symbol_index::start_watcher(root, index);
        }

        Self {
            agent_providers: ArcSwap::new(Arc::new(initial)),
            prompt_engine: Self::default_prompt_engine(),
            #[cfg(feature = "self-iter")]
            self_iter_engine: None,
            #[cfg(feature = "self-iter")]
            self_iter_max_recovery_attempts: 3,
            skill_store: ArcSwap::new(Arc::new(None)),
            skill_usage_store: ArcSwap::new(Arc::new(None)),
            skills_deny: ArcSwap::new(Arc::new(Vec::new())),
            skills_allow: ArcSwap::new(Arc::new(Vec::new())),
            skills_context_budget_percent: ArcSwap::new(Arc::new(
                xiaolin_core::config::SkillsConfig::default().context_budget_percent,
            )),
            trajectory_store: ArcSwap::new(Arc::new(None)),
            cached_runtime_registry: Arc::new(runtimes::register_default_runtimes()),
            last_mcp_instructions_version: std::sync::atomic::AtomicU64::new(u64::MAX),
            self_handle: std::sync::OnceLock::new(),
        }
    }

    /// Store a weak self-reference so `&self` methods can obtain `Arc<Self>`.
    ///
    /// Must be called once immediately after `Arc::new(AgentRuntime::new(...))`.
    pub fn init_self_arc(self: &Arc<Self>) {
        let _ = self.self_handle.set(Arc::downgrade(self));
    }

    /// Reconstruct `Arc<Self>` from the stored weak reference.
    ///
    /// Panics if `init_self_arc` was never called or the Arc was already dropped.
    fn arc_self(&self) -> Arc<Self> {
        self.self_handle
            .get()
            .expect("AgentRuntime::init_self_arc was not called")
            .upgrade()
            .expect("AgentRuntime Arc already dropped")
    }

    fn default_prompt_engine() -> PromptEngine {
        use prompt_sections::dynamic::{
            environment_section, frc_section, language_section, mcp_instructions_section,
            memory_section, session_guidance_section, token_budget_section,
        };
        use prompt_sections::{
            actions_section, doing_tasks_section, intro_section, output_efficiency_section,
            system_section, tone_and_style_section, using_tools_section,
        };

        // Tier-1: pure templates (cross-session stable)
        let static_sections: Vec<PromptSection> = vec![
            intro_section(),
            doing_tasks_section(),
            actions_section(),
            tone_and_style_section(),
            output_efficiency_section(),
            frc_section(),
        ];

        // Tier-2: session-stable (invalidated on explicit events)
        let dynamic_sections: Vec<PromptSection> = vec![
            system_section(),
            using_tools_section(),
            session_guidance_section(),
            environment_section(),
            memory_section(),
            language_section(),
            mcp_instructions_section(),
            token_budget_section(),
        ];

        PromptEngine::new(static_sections, dynamic_sections)
    }

    pub fn with_skill_store(self, store: Arc<SkillStore>) -> Self {
        self.skill_store.store(Arc::new(Some(store)));
        self
    }

    pub fn with_trajectory_store(self, store: Arc<TrajectoryStore>) -> Self {
        self.trajectory_store.store(Arc::new(Some(store)));
        self
    }

    pub fn attach_evolution_stores(
        &self,
        skill: Arc<SkillStore>,
        trajectory: Arc<TrajectoryStore>,
    ) {
        self.skill_store.store(Arc::new(Some(skill)));
        self.trajectory_store.store(Arc::new(Some(trajectory)));
    }

    pub fn attach_skill_usage_store(&self, store: Arc<xiaolin_core::skill_usage::SkillUsageStore>) {
        self.skill_usage_store.store(Arc::new(Some(store)));
    }

    /// Update the live skills deny list used by evolution skill injection.
    pub fn set_skills_deny(&self, deny: Vec<String>) {
        self.skills_deny.store(Arc::new(deny));
    }

    /// Update the live skills allow list used by evolution skill injection.
    pub fn set_skills_allow(&self, allow: Vec<String>) {
        self.skills_allow.store(Arc::new(allow));
    }

    /// Update the live skills context budget percent used by evolution skill injection.
    pub fn set_skills_context_budget_percent(&self, percent: u8) {
        self.skills_context_budget_percent.store(Arc::new(percent));
    }

    #[cfg(feature = "self-iter")]
    pub fn with_self_iter_engine(mut self, engine: Arc<SelfIterEngine>) -> Self {
        self.self_iter_engine = Some(engine);
        self
    }

    #[cfg(feature = "self-iter")]
    pub fn with_self_iter_max_recovery_attempts(mut self, n: u32) -> Self {
        self.self_iter_max_recovery_attempts = n.max(1);
        self
    }

    pub fn provider(&self) -> Arc<dyn LlmProvider> {
        self.default_provider_arc()
    }

    /// Get a shared reference to the default LLM provider.
    pub fn default_provider_arc(&self) -> Arc<dyn LlmProvider> {
        let guard = self.agent_providers.load();
        guard
            .get(DEFAULT_PROVIDER_KEY)
            .cloned()
            .expect("default provider must exist")
    }

    /// Atomically replace the default LLM provider used as fallback when no
    /// per-agent provider is registered.
    pub fn set_default_provider(&self, provider: Arc<dyn LlmProvider>) {
        let mut m = self.agent_providers.load().as_ref().clone();
        m.insert(DEFAULT_PROVIDER_KEY.to_string(), provider);
        self.agent_providers.store(Arc::new(m));
        tracing::info!("default LLM provider hot-swapped");
    }

    pub fn register_provider(&self, agent_id: &str, provider: Arc<dyn LlmProvider>) {
        let mut m = self.agent_providers.load().as_ref().clone();
        m.insert(agent_id.to_string(), provider);
        self.agent_providers.store(Arc::new(m));
    }

    /// Drop all per-agent provider overrides, keeping only the default provider.
    pub fn clear_registered_providers(&self) {
        let guard = self.agent_providers.load();
        let default = guard.get(DEFAULT_PROVIDER_KEY).cloned();
        let mut fresh = HashMap::new();
        if let Some(p) = default {
            fresh.insert(DEFAULT_PROVIDER_KEY.to_string(), p);
        }
        self.agent_providers.store(Arc::new(fresh));
    }

    fn resolve_provider(&self, agent_id: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let guard = self.agent_providers.load();
        guard
            .get(agent_id)
            .or_else(|| guard.get(DEFAULT_PROVIDER_KEY))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no provider found for agent '{agent_id}'"))
    }

    pub async fn execute(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<ExecutionResult> {
        self.execute_with_subagent_prompt(config, request, tool_registry, llm_override, None)
            .await
    }

    pub async fn execute_with_subagent_prompt(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        subagent_prompt: Option<String>,
    ) -> anyhow::Result<ExecutionResult> {
        self.execute_with_subagent_prompt_and_runtime_quality_store(
            config,
            request,
            tool_registry,
            llm_override,
            subagent_prompt,
            None,
        )
        .await
    }

    pub async fn execute_with_subagent_prompt_and_runtime_quality_store(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        subagent_prompt: Option<String>,
        runtime_quality_store: Option<Arc<xiaolin_session::RuntimeQualityStore>>,
    ) -> anyhow::Result<ExecutionResult> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(512);
        let orchestrator = Arc::new(orchestrator::ToolOrchestrator::new());
        let approval_strategy = xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove;

        let summary = self
            .execute_unified_with_cost_store(
                config,
                request,
                tool_registry,
                tx,
                approval_strategy,
                llm_override,
                orchestrator,
                None,
                subagent_prompt,
                None,
                None,
                None,
                None,
                None,
                runtime_quality_store,
                None,
                None,
                None,
                None,
            )
            .await?;

        // Collect streamed content to reconstruct ExecutionResult
        let mut text = String::new();
        while let Ok(evt) = rx.try_recv() {
            if let AgentEvent::ContentDelta { ref delta, .. } = evt {
                if let Some(content) = delta
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                {
                    text.push_str(content);
                }
            }
        }

        let model = request
            .model
            .clone()
            .unwrap_or_else(|| config.model.model.clone());
        let usage = summary.usage.map(|u| xiaolin_core::types::Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            ..Default::default()
        });
        let response = ChatResponse {
            id: summary.turn_id.to_string(),
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            model,
            choices: vec![xiaolin_core::types::ChatChoice {
                index: 0,
                finish_reason: Some("stop".to_string()),
                message: ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(text)),
                    ..Default::default()
                },
            }],
            usage,
        };

        Ok(ExecutionResult {
            response,
            tool_calls_made: summary.tool_calls_made,
            iterations: summary.iterations,
        })
    }

    /// Streaming agentic loop: streams text deltas to the caller while handling
    /// tool calling iterations transparently.
    ///
    /// **Stream resume (best effort):** if the SSE stream yields an error after
    /// some text deltas (e.g. connection drop) and there is no in-flight tool-call
    /// assembly, the partial assistant text is appended to `messages` and the
    /// stream is re-opened on the same turn (bounded retries). The model may
    /// repeat a prefix of the answer; the goal is not to lose prior context.
    pub async fn execute_stream(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<TurnSummary> {
        let ctx = agent_context::AgentContext {
            config: config.clone(),
            request: request.clone(),
            tool_registry: tool_registry.clone(),
            step_tx: None,
            event_tx: Some(tx.clone()),
            llm_override,
            subagent_prompt: None,
            active_runs_context: None,
            mode_state: None,
            orchestrator: None,
            interaction_handle: None,
            approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove,
            runtime_registry: None,
            behavior_overrides: None,
            session_store: None,
            todo_store: None,
            goal_store: None,
            cost_store: None,
            runtime_quality_store: None,
            artifact_store: None,
            plan_file_path: None,
            message_queue: None,
            cancel_token: None,
        };
        Self::run_stream_to_completion(self.arc_self(), ctx, tx).await
    }

    /// Unified execution entry point for all callers.
    ///
    /// All entry points (Gateway WS, HTTP, CLI, Feishu, Tauri, SubAgent) should
    /// converge on this method. The `ApprovalStrategy` determines how tool
    /// approval is handled; the `RuntimeRegistry` is used internally by the
    /// orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_unified(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
        llm_override: Option<Arc<dyn LlmProvider>>,
        orchestrator: Arc<crate::runtime::orchestrator::ToolOrchestrator>,
        interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
        subagent_prompt: Option<String>,
        mode_state: Option<crate::builtin_tools::ExecutionModeState>,
        session_store: Option<Arc<xiaolin_session::SessionStore>>,
        todo_store: Option<crate::builtin_tools::TodoStore>,
        goal_store: Option<Arc<crate::builtin_tools::GoalStore>>,
    ) -> anyhow::Result<TurnSummary> {
        self.execute_unified_with_cost_store(
            config,
            request,
            tool_registry,
            tx,
            approval_strategy,
            llm_override,
            orchestrator,
            interaction_handle,
            subagent_prompt,
            mode_state,
            session_store,
            todo_store,
            goal_store,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_unified_with_cost_store(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
        llm_override: Option<Arc<dyn LlmProvider>>,
        orchestrator: Arc<crate::runtime::orchestrator::ToolOrchestrator>,
        interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
        subagent_prompt: Option<String>,
        mode_state: Option<crate::builtin_tools::ExecutionModeState>,
        session_store: Option<Arc<xiaolin_session::SessionStore>>,
        todo_store: Option<crate::builtin_tools::TodoStore>,
        goal_store: Option<Arc<crate::builtin_tools::GoalStore>>,
        cost_store: Option<Arc<xiaolin_session::CostStore>>,
        runtime_quality_store: Option<Arc<xiaolin_session::RuntimeQualityStore>>,
        artifact_store: Option<Arc<dyn xiaolin_session::ArtifactStore>>,
        behavior_overrides: Option<
            std::sync::Arc<dashmap::DashMap<String, xiaolin_core::agent_config::BehaviorConfig>>,
        >,
        message_queue: Option<Arc<crate::message_queue::MessageQueue>>,
        active_runs_context: Option<String>,
    ) -> anyhow::Result<TurnSummary> {
        let ctx = agent_context::AgentContext {
            config: config.clone(),
            request: request.clone(),
            tool_registry: tool_registry.clone(),
            step_tx: None,
            event_tx: Some(tx.clone()),
            llm_override,
            subagent_prompt,
            active_runs_context,
            mode_state,
            orchestrator: Some(orchestrator),
            interaction_handle,
            approval_strategy,
            runtime_registry: Some(self.cached_runtime_registry.clone()),
            behavior_overrides,
            session_store,
            todo_store,
            goal_store,
            cost_store,
            runtime_quality_store,
            artifact_store,
            plan_file_path: crate::builtin_tools::plan_mode::current_plan_context()
                .map(|pc| pc.store.plan_path(&pc.session_id)),
            message_queue,
            cancel_token: None,
        };
        Self::run_stream_to_completion(self.arc_self(), ctx, tx).await
    }

    /// Internal: consume `execute_as_stream`, forward AgentStep → AgentEvent to tx,
    /// and return the TurnSummary from the final TurnEnd step.
    async fn run_stream_to_completion(
        runtime: Arc<Self>,
        ctx: agent_context::AgentContext,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> anyhow::Result<TurnSummary> {
        use futures::StreamExt;

        let mut stream = std::pin::pin!(Self::execute_as_stream(runtime, ctx));
        let mut turn_summary: Option<TurnSummary> = None;
        let mut first_error: Option<String> = None;

        while let Some(step) = stream.next().await {
            if let agent_step::AgentStep::Error { ref message, .. } = step {
                if first_error.is_none() {
                    first_error = Some(message.clone());
                }
            }
            if let agent_step::AgentStep::TurnEnd { ref summary, .. } = step {
                turn_summary = Some(summary.clone());
            }
            for event in step.into_agent_events() {
                let _ = stream_engine::send_stream_event(&tx, event, false).await;
            }
        }

        turn_summary.ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                first_error.unwrap_or_else(|| "agent stream ended without TurnEnd".to_string())
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn try_self_iter_tool_recovery(
        &self,
        messages: &mut Vec<ChatMessage>,
        config: &AgentConfig,
        #[cfg(feature = "self-iter")] request: &ChatRequest,
        #[cfg(not(feature = "self-iter"))] _request: &ChatRequest,
        #[cfg(feature = "self-iter")] loop_iteration: u32,
        #[cfg(not(feature = "self-iter"))] _loop_iteration: u32,
        consecutive_errors: u32,
        max_errors: u32,
        failure_streak: &[ToolCallTrace],
        recovery_attempts: &mut u32,
    ) -> bool {
        let max_attempts = {
            #[cfg(feature = "self-iter")]
            {
                self.self_iter_max_recovery_attempts
            }
            #[cfg(not(feature = "self-iter"))]
            {
                3u32
            }
        };
        if *recovery_attempts >= max_attempts {
            return false;
        }
        let trigger = std::cmp::min(2, max_errors.max(1));
        if consecutive_errors < trigger || failure_streak.is_empty() {
            return false;
        }

        // Try advanced SelfIterEngine diagnosis first (when available),
        // then fall back to basic guidance.
        let guidance: String;

        #[cfg(feature = "self-iter")]
        {
            let advanced = self.self_iter_engine.as_ref().and_then(|engine| {
                let session = request
                    .session_id
                    .clone()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "default".to_string());
                let diagnoses = engine.diagnose_tool_failure_streak(
                    &config.agent_id,
                    &session,
                    loop_iteration,
                    failure_streak,
                );
                SelfIterEngine::format_recovery_guidance(&diagnoses)
            });
            match advanced {
                Some(g) => guidance = g,
                None => match format_basic_recovery_guidance(failure_streak) {
                    Some(g) => guidance = g,
                    None => return false,
                },
            }
        }
        #[cfg(not(feature = "self-iter"))]
        {
            match format_basic_recovery_guidance(failure_streak) {
                Some(g) => guidance = g,
                None => return false,
            }
        }

        inject_tool_recovery_guidance(messages, &guidance);
        *recovery_attempts += 1;
        tracing::info!(
            agent_id = %config.agent_id,
            recovery_attempt = *recovery_attempts,
            "tool recovery guidance injected into system prompt"
        );
        true
    }

    async fn finalize_injected_skills(&self, injected_skill_ids: &[String], success: bool) {
        let store: Arc<SkillStore> = match (*self.skill_store.load()).as_ref() {
            Some(s) => s.clone(),
            None => return,
        };
        for id in injected_skill_ids {
            if let Err(e) = store.record_usage(id, success).await {
                tracing::warn!(skill_id = %id, error = %e, "skill usage record failed");
            }
        }
    }

    async fn inject_relevant_skills(
        &self,
        messages: &mut Vec<ChatMessage>,
        request: &ChatRequest,
        injected_skill_ids: &mut Vec<String>,
        context_window: u32,
    ) -> anyhow::Result<()> {
        let store: Arc<SkillStore> = match (*self.skill_store.load()).as_ref() {
            Some(s) => s.clone(),
            None => return Ok(()),
        };
        let task = last_user_turn_text(&request.messages);
        let trimmed = task.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if trimmed.split_whitespace().count() < 3 && trimmed.len() < 12 {
            tracing::debug!(
                task = trimmed,
                "inject_relevant_skills: skipping trivial query"
            );
            return Ok(());
        }
        let skills = store.find_similar(&task, 16).await?;
        let deny_set: HashSet<String> = self.skills_deny.load().iter().cloned().collect();
        let allow_list = self.skills_allow.load();
        let allow_set: HashSet<String> = allow_list.iter().cloned().collect();
        let passes_allow = |id: &str| allow_set.is_empty() || allow_set.contains(id);
        let active: Vec<_> = skills
            .iter()
            .filter(|s| matches!(s.status, SkillStatus::Active))
            .filter(|s| !deny_set.contains(&s.id))
            .filter(|s| passes_allow(&s.id))
            .take(5)
            .cloned()
            .collect();
        let candidates: Vec<_> = skills
            .iter()
            .filter(|s| matches!(s.status, SkillStatus::Candidate))
            .filter(|s| !deny_set.contains(&s.id))
            .filter(|s| passes_allow(&s.id))
            .take(2)
            .cloned()
            .collect();
        if active.is_empty() && candidates.is_empty() {
            return Ok(());
        }
        for s in &active {
            injected_skill_ids.push(s.id.clone());
        }
        for s in &candidates {
            injected_skill_ids.push(s.id.clone());
        }

        let mut block = String::new();
        if !active.is_empty() {
            block.push_str(&format_skills_for_prompt(&active));
        }
        if !candidates.is_empty() {
            block.push_str(&format_candidate_skills_for_prompt(&candidates));
        }
        block.push_str(SKILL_MANAGEMENT_GUIDANCE);

        let budget_percent = **self.skills_context_budget_percent.load();
        if budget_percent > 0 {
            let char_budget = (context_window as usize) * (budget_percent as usize) / 100;
            let block_chars = block.chars().count();
            if block_chars > char_budget {
                block = block.chars().take(char_budget).collect();
                tracing::warn!(
                    char_budget,
                    block_chars,
                    "evolution skill injection truncated to context budget"
                );
            }
        }

        inject_user_context(messages, &block);

        let session_key = request.session_id.as_deref().unwrap_or("default");
        if let Err(e) = store
            .register_session_skills(session_key, injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "register_session_skills failed");
        }

        if let Some(usage_store) = (*self.skill_usage_store.load()).as_ref() {
            let ids: Vec<String> = injected_skill_ids.to_vec();
            let usage_store = usage_store.clone();
            let sess = request.session_id.clone();
            tokio::spawn(async move {
                let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
                if let Err(e) = usage_store
                    .record_injections(&id_refs, sess.as_deref())
                    .await
                {
                    tracing::warn!(error = %e, "failed to record skill injection events");
                }
            });
        }

        Ok(())
    }

    fn build_messages(&self, ctx: &agent_context::AgentContext) -> Vec<ChatMessage> {
        let config = &ctx.config;
        let user_messages = &ctx.request.messages;

        let mut messages = Vec::with_capacity(user_messages.len() + 1);

        let agent_prompt = config
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        // Event-driven invalidation (§5.2): the `mcp_instructions` section is
        // memoized for prefix stability. Only recompute it when the registry's
        // MCP-instructions version actually changed (connect/disconnect/update).
        let mcp_ver = ctx.tool_registry.mcp_instructions_version();
        let prev_ver = self
            .last_mcp_instructions_version
            .swap(mcp_ver, std::sync::atomic::Ordering::Relaxed);
        if prev_ver != mcp_ver {
            self.prompt_engine
                .invalidate_sections(&["mcp_instructions"]);
        }

        let prompt_ctx = self.build_prompt_context(ctx);
        let parts = self.prompt_engine.build_effective_prompt(
            &prompt_ctx,
            None,
            agent_prompt,
            None,
            ctx.subagent_prompt.as_deref(),
        );

        let system_text = parts.join("\n\n");
        push_system_messages_from_prompt(&mut messages, &system_text);

        let mut conversation: Vec<ChatMessage> = user_messages.to_vec();
        messages.append(&mut conversation);
        merge_leading_system_into_tier2(&mut messages);

        // Per-turn active sub-agent status: inject as `<system_context>` into the
        // last user message instead of the system prompt, so the cacheable system
        // prefix stays byte-stable even as `elapsed_ms` changes (prompt-cache D3).
        if let Some(ref arc) = ctx.active_runs_context {
            inject_user_context(&mut messages, arc);
        }

        if let Some(ref req_model) = ctx.request.model {
            if !req_model.is_empty() {
                let has_conflicting_identity = messages.iter().any(|m| {
                    if m.role != Role::Assistant {
                        return false;
                    }
                    if let Some(text) = m.text_content() {
                        let lower = text.to_lowercase();
                        (lower.contains("我是") || lower.contains("i am") || lower.contains("i'm"))
                            && !lower.contains(&req_model.to_lowercase())
                    } else {
                        false
                    }
                });
                if has_conflicting_identity {
                    if let Some(last_user_idx) = messages.iter().rposition(|m| m.role == Role::User)
                    {
                        let reminder = format!(
                            "[Model Switch Notice] The model has been switched. You are now {}. \
                             Disregard any previous assistant messages claiming a different model identity.",
                            req_model
                        );
                        messages.insert(
                            last_user_idx + 1,
                            ChatMessage {
                                role: Role::System,
                                content: Some(serde_json::Value::String(reminder)),
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }

        messages
    }

    fn build_prompt_context(&self, ctx: &agent_context::AgentContext) -> PromptContext {
        let tool_names = ctx.tool_registry.tool_names();
        let deferred_count = ctx.tool_registry.deferred_count();

        let mode = ctx
            .mode_state
            .as_ref()
            .map(|ms| ms.current_mode())
            .unwrap_or(ExecutionMode::Agent);

        let model_id = if let Some(ref req_model) = ctx.request.model {
            if !req_model.is_empty() {
                req_model.clone()
            } else {
                format!("{}/{}", ctx.config.model.provider, ctx.config.model.model)
            }
        } else {
            format!("{}/{}", ctx.config.model.provider, ctx.config.model.model)
        };

        let cwd = ctx
            .request
            .work_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let is_git = cwd.join(".git").exists();
        let platform = std::env::consts::OS.to_string();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();

        let pending_todo_summary = if mode == ExecutionMode::Agent {
            ctx.todo_store.as_ref().and_then(|ts| ts.pending_summary())
        } else {
            None
        };

        let (plan_file_path, plan_file_exists) =
            crate::builtin_tools::plan_mode::current_plan_context()
                .map(|pc| {
                    let path = pc.store.plan_path(&pc.session_id);
                    let exists = pc.store.plan_exists(&pc.session_id);
                    (Some(path.display().to_string()), exists)
                })
                .unwrap_or((None, false));

        PromptContext {
            agent_config: Arc::new(ctx.config.clone()),
            enabled_tools: tool_names,
            deferred_tool_count: deferred_count,
            model_id,
            cwd,
            is_git,
            platform,
            shell,
            execution_mode: mode,
            mcp_servers: ctx
                .tool_registry
                .mcp_instructions_snapshot()
                .into_iter()
                .map(|(id, instr)| prompt_engine::McpServerInfo {
                    id,
                    instructions: Some(instr),
                })
                .collect(),
            language_preference: ctx.request.response_language.clone(),
            token_budget: None,
            memory_prompt: None,
            session_start_date: date,
            pending_todo_summary,
            plan_file_path,
            plan_file_exists,
            system_base_prompt: Some(
                xiaolin_core::workspace::EMBEDDED_SYSTEM_BASE_PROMPT.to_string(),
            ),
        }
    }

    /// Load persisted ContentReplacementState from session store, or create fresh.
    /// On resume, collects all tool_use_ids from existing messages and loads persisted
    /// replacement records to reconstruct byte-identical state.
    async fn load_or_create_replacement_state(
        session_store: &Option<Arc<xiaolin_session::SessionStore>>,
        session_id: Option<&str>,
        messages: &[ChatMessage],
    ) -> ContentReplacementState {
        let Some(store) = session_store else {
            return ContentReplacementState::new();
        };
        let Some(sid) = session_id else {
            return ContentReplacementState::new();
        };

        let records = match store.load_replacement_records(sid).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, session_id = sid, "failed to load replacement records, starting fresh");
                return ContentReplacementState::new();
            }
        };

        if records.is_empty() {
            return ContentReplacementState::new();
        }

        let message_tool_use_ids: Vec<String> = messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.clone())
            .collect();

        let cr_records: Vec<tool_result_storage::ContentReplacementRecord> = records
            .into_iter()
            .map(|r| tool_result_storage::ContentReplacementRecord {
                tool_use_id: r.tool_use_id,
                replacement: r.replacement,
            })
            .collect();

        let state = reconstruct_state(&message_tool_use_ids, &cr_records);
        tracing::info!(
            session_id = sid,
            seen_ids = state.seen_ids.len(),
            replacements = state.replacements.len(),
            "reconstructed ContentReplacementState from persisted records"
        );
        state
    }

    /// Persist newly created replacement records to session store.
    /// Fails silently (logs warning) — the fallback is truncation on next turn.
    async fn persist_replacement_records(
        session_store: &Option<Arc<xiaolin_session::SessionStore>>,
        session_id: Option<&str>,
        records: &[tool_result_storage::ContentReplacementRecord],
    ) {
        if records.is_empty() {
            return;
        }
        let Some(store) = session_store else {
            return;
        };
        let Some(sid) = session_id else {
            return;
        };

        let rows: Vec<xiaolin_session::ContentReplacementRow> = records
            .iter()
            .map(|r| xiaolin_session::ContentReplacementRow {
                tool_use_id: r.tool_use_id.clone(),
                replacement: r.replacement.clone(),
            })
            .collect();

        if let Err(e) = store.save_replacement_records(sid, &rows).await {
            tracing::warn!(
                error = %e,
                session_id = sid,
                count = rows.len(),
                "failed to persist replacement records"
            );
        }
    }
}

#[cfg(test)]
mod stream_resume_tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::llm::CompletionParams;
    use async_trait::async_trait;
    use futures::stream::{self, StreamExt};
    use xiaolin_core::agent_config::{AgentConfig, AgentModelConfig, BehaviorConfig};
    use xiaolin_core::tool::ToolRegistry;
    use xiaolin_core::types::{
        ChatMessage, ChatRequest, ChatResponse, DeltaContent, Role, StreamChoice, StreamDelta,
    };

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            agent_id: "t1".into(),
            name: None,
            description: None,
            model: AgentModelConfig {
                provider: "openai".into(),
                model: "mock".into(),
                temperature: 0.0,
                max_tokens: None,
                context_window: None,
                cost_per_1k_input: None,
                cost_per_1k_output: None,
                supports_reasoning: None,
                capabilities: None,
                fallbacks: Vec::new(),
                max_concurrent_requests: 10,
            },
            system_prompt: Some("You are a test assistant.".into()),
            tools: Vec::new(),
            behavior: BehaviorConfig::default(),
            mcp_servers: Vec::new(),
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }
    }

    struct FlakyStreamProvider {
        calls: Arc<AtomicU32>,
    }

    fn stream_delta_text(text: &str) -> StreamDelta {
        StreamDelta {
            id: "id-m".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: Some(text.into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    fn stream_delta_stop() -> StreamDelta {
        StreamDelta {
            id: "id-m".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    #[async_trait]
    impl LlmProvider for FlakyStreamProvider {
        async fn chat_completion(&self, _: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used")
        }

        async fn chat_completion_stream(
            &self,
            _: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let s = if n == 0 {
                stream::iter(vec![
                    Ok(stream_delta_text("hello")),
                    Err(anyhow::anyhow!("simulated drop")),
                ])
                .boxed()
            } else {
                stream::iter(vec![
                    Ok(stream_delta_text(" world")),
                    Ok(stream_delta_stop()),
                ])
                .boxed()
            };
            Ok(s)
        }
    }

    #[tokio::test]
    async fn execute_stream_resumes_after_interrupt_with_partial_context() {
        let config = test_agent_config();
        let calls = Arc::new(AtomicU32::new(0));
        let provider: Arc<dyn LlmProvider> = Arc::new(FlakyStreamProvider {
            calls: calls.clone(),
        });
        let runtime = Arc::new(AgentRuntime::new(provider));
        runtime.init_self_arc();
        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        let req = ChatRequest {
            model: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: Some("hi".into()),
                ..Default::default()
            }],
            agent_id: None,
            session_id: None,
            stream: true,
            temperature: None,
            max_tokens: None,
            tools: None,
            slash_intent: None,
            work_dir: None,
            response_language: None,
            goal_mode: None,
        };

        let res = runtime
            .execute_stream(&config, &req, &registry, tx, None)
            .await;

        assert!(res.is_ok(), "{res:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "expected stream reconnect");

        let seen = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut s = String::new();
            while let Some(ev) = rx.recv().await {
                match ev {
                    AgentEvent::ContentDelta { delta, .. } => {
                        if let Some(c) = delta
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            s.push_str(c);
                        }
                    }
                    AgentEvent::TurnEnd { .. } => break,
                    AgentEvent::Error { message, .. } => {
                        panic!("unexpected stream error: {message}")
                    }
                    _ => {}
                }
            }
            s
        })
        .await
        .expect("timeout waiting for stream events");

        assert!(seen.contains("hello"), "concatenated deltas: {seen}");
        assert!(seen.contains("world"), "concatenated deltas: {seen}");
    }
}
