mod ask_question;
mod brief;
mod clipboard;
mod confirm;
pub mod coordinator;
pub mod goal;
mod identity;
mod media;
mod memory;
pub mod plan_file;
pub mod plan_mode;
mod request_permissions;
mod screenshot;
mod session;
pub mod skill;
mod snip;
mod task;
pub mod team;
pub mod terminal;
mod todo;
mod tool_search;
pub mod update_plan;
mod utility;
pub mod worker;
pub mod workflow;

#[cfg(feature = "browser")]
pub use xiaolin_tools_browser as browser;

use std::sync::Arc;

use xiaolin_core::bus::MessageBus;
use xiaolin_core::skill::SkillRegistry;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::workspace::AgentWorkspace;
use xiaolin_session::SessionStore;

pub use ask_question::{with_interaction_handle, with_steer_inbox, with_stream_context, AskQuestionTool, SteerInbox, STEER_INBOX};
pub(crate) use ask_question::{ASK_QUESTION_STREAM_KEY, TASK_INTERACTION_HANDLE};
pub use brief::BriefTool;
pub use xiaolin_tools_code::code_intel::{
    CodeSectionsTool, FileOutlineTool, FindReferencesTool, GoToDefinitionTool, UnifiedLspTool,
    WorkspaceSymbolsTool,
};
pub use clipboard::{register_clipboard_tools, ClipboardReadTool, ClipboardWriteTool};
pub use confirm::ConfirmTool;
pub use xiaolin_tools_fs::filesystem::{
    get_effective_work_dir, get_file_state_cache, set_code_graph_hook,
    with_additional_allowed_paths, with_file_access_mode, with_file_state_cache, with_work_dir,
};
pub use xiaolin_tools_fs::filesystem::{
    ApplyPatchTool, EditFileTool, GlobTool, ListDirectoryTool, MultiEditTool, ReadFileTool,
    SearchInFilesTool, WriteFileTool,
};
pub use goal::{
    ContinuationActivityResult, CreateGoalTool, GetGoalTool, Goal, GoalStatus, GoalStore,
    UpdateGoalTool,
};
pub use identity::{GetIdentityTool, SetIdentityTool, UnifiedIdentityTool};
pub use media::{ImageGenerateTool, TtsTool};
pub use memory::{MemorySearchTool, MemoryStoreTool, UnifiedMemoryTool};
pub use xiaolin_tools_network::{
    engine_by_id, BaiduEngine, BingEngine, BuiltinMetaEngine, GoogleEngine, HttpFetchTool,
    Search360Engine, SearchEngine, SearchResult, SearxngEngine, SogouEngine, TavilyEngine,
    WebFetchTool, WebSearchBackend, WebSearchTool, BUILTIN_ENGINE_IDS,
};
pub use xiaolin_tools_code::notebook::NotebookEditTool;
pub use plan_file::PlanFileStore;
pub use plan_mode::{
    current_plan_context, current_session_mode, with_session_mode, EnterPlanModeTool,
    ExecutionModeState, ExitPlanModeTool, PlanContext, SessionModeRegistry,
};
pub use request_permissions::RequestPermissionsTool;
pub use screenshot::{register_screenshot_tool, ScreenshotTool};
pub use session::{session_inbox_topic, SessionsSendTool, SessionsSpawnTool};
pub use xiaolin_tools_fs::shell::{
    has_binary_hijack_prefix, parse_sed_edit, sed_to_edit_suggestion, strip_safe_wrappers,
    validate_command_paths, validate_readonly_command, PermissionRule, SedEditInfo,
    ShellDefinitionStub,
};
pub use skill::{ListSkillsTool, ReadSkillTool, UnifiedSkillTool, WriteSkillTool};
pub use snip::SnipTool;
pub use task::{
    NoopTaskWorkFactory, TaskCreateTool, TaskGetTool, TaskInfo, TaskListTool, TaskManager,
    TaskManagerError, TaskStatus, TaskStopTool, TaskUpdateTool, TaskWorkFactory,
};
pub use xiaolin_tools_fs::terminal::TerminalCaptureTool;
pub use todo::{TodoItem, TodoReadTool, TodoStatus, TodoStore, TodoWriteTool};
pub use tool_search::ToolSearchTool;
pub use update_plan::{PlanStepStore, UpdatePlanTool};
pub use utility::{CurrentTimeTool, SleepTool};
pub use workflow::{WorkflowDefinition, WorkflowRun, WorkflowStatus, WorkflowStore, WorkflowTool};
pub use xiaolin_tools_fs::worktree::{EnterWorktreeTool, ExitWorktreeTool, WorktreeState};
pub use terminal::{register_terminal_tools, TerminalCloseTool, TerminalInputTool, TerminalOpenTool};

pub use xiaolin_tools_fs::exec_command;
pub use xiaolin_tools_fs::file_state_cache;
pub use xiaolin_tools_fs::shell;
pub use xiaolin_tools_fs::shell_path_validation;
pub use xiaolin_tools_fs::shell_readonly;
pub use xiaolin_tools_fs::shell_security;

#[cfg(feature = "browser")]
pub use browser::{register_browser_tool, BrowserTool};

/// Register all built-in tools into a registry.
pub fn register_builtin_tools(registry: &ToolRegistry) {
    register_builtin_tools_with_sandbox(registry, true);
}

/// Register built-in tools, optionally with sandbox enforcement on shell_exec.
pub fn register_builtin_tools_with_sandbox(registry: &ToolRegistry, sandboxed: bool) {
    register_builtin_tools_full(registry, sandboxed, None);
}

/// Register built-in tools with an optional `NetworkProxy` for managed
/// network routing in sandboxed shell execution.
pub fn register_builtin_tools_full(
    registry: &ToolRegistry,
    sandboxed: bool,
    network_proxy: Option<xiaolin_network_proxy::NetworkProxy>,
) {
    // ── Core eager tools (~15) ──────────────────────────────────────────────
    registry.register(Arc::new(HttpFetchTool::new()));
    registry.register(Arc::new(WebSearchTool::unconfigured()));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
    // Shell execution is now handled by RuntimeRegistry → orchestrator → ShellRuntime.
    // Register a definition-only stub so the LLM sees the tool schema.
    let _ = sandboxed;
    let _ = network_proxy;
    registry.register(Arc::new(xiaolin_tools_fs::shell::ShellDefinitionStub));
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(SearchInFilesTool));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(UnifiedLspTool));
    registry.register(Arc::new(ListDirectoryTool));
    registry.register(Arc::new(ScreenshotTool::new()));

    // ── Deferred tools (available via tool_search) ──────────────────────────
    registry.register_deferred(Arc::new(CurrentTimeTool));
    registry.register_deferred(Arc::new(SleepTool));
    registry.register_deferred(Arc::new(FileOutlineTool));
    registry.register_deferred(Arc::new(CodeSectionsTool));
    registry.register_deferred(Arc::new(MultiEditTool));
    registry.register_deferred(Arc::new(ApplyPatchTool));
    registry.register_deferred(Arc::new(NotebookEditTool));
    registry.register_deferred(Arc::new(RequestPermissionsTool));
    registry.register_deferred(Arc::new(TerminalCaptureTool::new()));
    register_clipboard_tools(registry);
}

/// Register web tools with a specific search backend configuration.
pub fn register_web_tools(registry: &ToolRegistry, backend: WebSearchBackend) {
    registry.register(Arc::new(WebSearchTool::new(backend)));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
}

/// Register media generation tools as deferred (requires API credentials).
pub fn register_media_tools(registry: &ToolRegistry, base_url: &str, api_key: &str) {
    registry.register_deferred(Arc::new(ImageGenerateTool::new(base_url, api_key)));
    registry.register_deferred(Arc::new(TtsTool::new(base_url, api_key)));
}

/// Register the unified skill tool (read-only: list + read + search).
pub fn register_skill_tools(registry: &ToolRegistry, skill_registry: Arc<SkillRegistry>) {
    registry.register(Arc::new(UnifiedSkillTool::readonly(skill_registry)));
}

/// Register the unified skill tool with write support (list + read + search + write).
pub fn register_skill_tools_full(
    registry: &ToolRegistry,
    skill_registry: Arc<SkillRegistry>,
    workspace: Arc<AgentWorkspace>,
    workspace_root: Option<std::path::PathBuf>,
) {
    let mut tool = UnifiedSkillTool::new(skill_registry, Some(workspace));
    if let Some(root) = workspace_root {
        tool = tool.with_workspace_root(root);
    }
    registry.register(Arc::new(tool));
}

/// Register the unified skill tool with write support and a reload callback
/// that re-scans skills after every successful write.
pub fn register_skill_tools_with_reload(
    registry: &ToolRegistry,
    skill_registry: Arc<SkillRegistry>,
    workspace: Arc<AgentWorkspace>,
    workspace_root: Option<std::path::PathBuf>,
    reload_callback: Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>,
) {
    let mut tool = UnifiedSkillTool::new(skill_registry, Some(workspace));
    if let Some(root) = workspace_root {
        tool = tool.with_workspace_root(root);
    }
    let tool = tool.with_reload_callback(reload_callback);
    registry.register(Arc::new(tool));
}

/// Register the unified identity tool for reading/writing SOUL.md, USER.md, AGENTS.md.
pub fn register_identity_tools(registry: &ToolRegistry, workspace: Arc<AgentWorkspace>) {
    registry.register(Arc::new(UnifiedIdentityTool::new(workspace)));
}

/// Register the ToolSearchTool. Must be called after the registry is wrapped
/// in `Arc`, since the tool needs a reference to search deferred tools.
pub fn register_tool_search(registry: &Arc<ToolRegistry>) {
    registry.register(Arc::new(ToolSearchTool::new(registry.clone())));
}

/// Register SnipTool with shared messages state. The runtime updates
/// this state before each tool iteration so SnipTool can mutate the
/// conversation in-place.
pub fn register_snip_tool(
    registry: &ToolRegistry,
    messages: std::sync::Arc<std::sync::Mutex<Vec<xiaolin_core::types::ChatMessage>>>,
) {
    registry.register(Arc::new(SnipTool::new(messages)));
}

/// Register BriefTool (send_user_message) with shared stream event channels.
pub fn register_brief_tool(
    registry: &ToolRegistry,
    stream_event_txs: std::sync::Arc<
        dashmap::DashMap<String, tokio::sync::mpsc::Sender<xiaolin_protocol::AgentEvent>>,
    >,
) {
    registry.register(Arc::new(BriefTool::new(stream_event_txs)));
}

pub fn register_todo_tools(registry: &ToolRegistry, store: TodoStore) {
    registry.register(Arc::new(TodoWriteTool::new(store.clone())));
    registry.register(Arc::new(TodoReadTool::new(store)));
}

pub fn register_update_plan_tool(
    registry: &ToolRegistry,
    stream_event_txs: std::sync::Arc<
        dashmap::DashMap<String, tokio::sync::mpsc::Sender<xiaolin_protocol::AgentEvent>>,
    >,
    store: PlanStepStore,
) {
    registry.register(Arc::new(UpdatePlanTool::new(stream_event_txs, store)));
}

/// Register task management tools (create, list, get, stop).
/// TaskList, TaskGet, and TaskStop are registered as deferred (available via ToolSearch).
pub fn register_task_tools(
    registry: &ToolRegistry,
    manager: Arc<TaskManager>,
    work_factory: Arc<dyn TaskWorkFactory>,
) {
    registry.register(Arc::new(TaskCreateTool::new(
        Arc::clone(&manager),
        work_factory,
    )));
    registry.register_deferred(Arc::new(TaskListTool::new(Arc::clone(&manager))));
    registry.register_deferred(Arc::new(TaskGetTool::new(Arc::clone(&manager))));
    registry.register_deferred(Arc::new(TaskUpdateTool::new(Arc::clone(&manager))));
    registry.register_deferred(Arc::new(TaskStopTool::new(manager)));
}

/// Register plan mode tools (enter/exit) with shared execution mode state.
/// Both tools self-declare `exposure() == Deferred`, so `register()` auto-adds
/// them to the deferred set. Mode-aware promotion is handled by `ToolProfile`.
pub fn register_plan_mode_tools(registry: &ToolRegistry, mode_state: ExecutionModeState) {
    registry.register(Arc::new(EnterPlanModeTool::new(mode_state.clone())));
    registry.register(Arc::new(ExitPlanModeTool::new(mode_state)));
}

pub fn register_session_tools(
    registry: &ToolRegistry,
    sessions: Arc<SessionStore>,
    bus: Arc<MessageBus>,
) {
    registry.register_deferred(Arc::new(SessionsSpawnTool::new(
        sessions.clone(),
        bus.clone(),
    )));
    registry.register_deferred(Arc::new(SessionsSendTool::new(sessions, bus)));
}

/// Register PTY interactive terminal tools (exec_command + write_stdin).
pub fn register_exec_command_tools(
    registry: &ToolRegistry,
    session_manager: Arc<xiaolin_tools_fs::exec_command::PtySessionManager>,
) {
    registry.register_deferred(Arc::new(
        xiaolin_tools_fs::exec_command::ExecCommandTool::new(session_manager.clone()),
    ));
    registry.register_deferred(Arc::new(
        xiaolin_tools_fs::exec_command::WriteStdinTool::new(session_manager),
    ));
}

/// Register goal management tools (get_goal, create_goal, update_goal).
/// Registered as direct (non-deferred) so the model can always call them
/// — the goal-mode prompt in chat.rs tells the model *when* to use them.
pub fn register_goal_tools(registry: &ToolRegistry, store: Arc<GoalStore>) {
    registry.register(Arc::new(GetGoalTool::new(store.clone())));
    registry.register(Arc::new(CreateGoalTool::new(store.clone())));
    registry.register(Arc::new(UpdateGoalTool::new(store)));
}
