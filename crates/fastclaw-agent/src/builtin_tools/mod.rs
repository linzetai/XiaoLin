mod ask_question;
mod brief;
mod confirm;
mod code_intel;
mod filesystem;
mod identity;
mod lsp_manager;
mod media;
mod memory;
mod network;
mod notebook;
mod plan_mode;
mod session;
mod task;
mod terminal;
mod shell;
pub mod shell_path_validation;
pub mod shell_readonly;
pub mod shell_security;
#[allow(dead_code)]
pub mod shell_snapshot;
#[allow(dead_code)]
pub mod coordinator;
mod skill;
mod todo;
mod snip;
mod tool_search;
mod utility;
pub mod workflow;
pub mod worktree;

#[cfg(feature = "browser")]
pub mod browser;

use std::sync::Arc;

use fastclaw_core::bus::MessageBus;
use fastclaw_core::skill::SkillRegistry;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::workspace::AgentWorkspace;
use fastclaw_session::SessionStore;

pub use filesystem::{
    ApplyPatchTool, EditFileTool, GlobTool, ListDirectoryTool, MultiEditTool, ReadFileTool, SearchInFilesTool, WriteFileTool,
};
pub use filesystem::{get_effective_work_dir, with_additional_allowed_paths, with_file_access_mode, with_file_state_cache, with_work_dir};
pub use media::{ImageGenerateTool, TtsTool};
pub use memory::{MemorySearchTool, MemoryStoreTool, UnifiedMemoryTool};
pub use network::{
    HttpFetchTool, SearchEngine, SearchResult, SearxngEngine, TavilyEngine,
    WebFetchTool, WebSearchBackend, WebSearchTool,
    GoogleEngine, BaiduEngine, BingEngine, SogouEngine, Search360Engine,
    BuiltinMetaEngine, engine_by_id, BUILTIN_ENGINE_IDS,
};
pub use session::{session_inbox_topic, SessionsSendTool, SessionsSpawnTool};
pub use shell::{
    SandboxedShellTool, ShellSandboxConfig, ShellTool,
    validate_readonly_command, validate_command_paths,
    PermissionRule, strip_safe_wrappers, has_binary_hijack_prefix,
    SedEditInfo, parse_sed_edit, sed_to_edit_suggestion,
};
pub use identity::{GetIdentityTool, SetIdentityTool, UnifiedIdentityTool};
pub use skill::{ListSkillsTool, ReadSkillTool, UnifiedSkillTool, WriteSkillTool};
pub use ask_question::{AskQuestionTool, with_stream_context};
pub use brief::BriefTool;
pub use confirm::ConfirmTool;
pub use todo::{TodoStore, TodoWriteTool, TodoStatus, TodoItem};
pub use code_intel::{FindReferencesTool, GoToDefinitionTool, UnifiedLspTool, WorkspaceSymbolsTool, FileOutlineTool, CodeChunkTool};
pub use notebook::NotebookEditTool;
pub use task::{
    NoopTaskWorkFactory, TaskCreateTool, TaskGetTool, TaskInfo, TaskListTool, TaskManager,
    TaskManagerError, TaskStatus, TaskStopTool, TaskUpdateTool, TaskWorkFactory,
};
pub use plan_mode::{EnterPlanModeTool, ExitPlanModeTool, ExecutionModeState, VerifyPlanExecutionTool};
pub use snip::SnipTool;
pub use terminal::TerminalCaptureTool;
pub use tool_search::ToolSearchTool;
pub use utility::{CalculatorTool, CurrentTimeTool, SleepTool};
pub use workflow::{WorkflowStore, WorkflowTool, WorkflowDefinition, WorkflowRun, WorkflowStatus};
pub use worktree::{EnterWorktreeTool, ExitWorktreeTool, WorktreeState};

#[cfg(feature = "browser")]
pub use browser::{register_browser_tool, BrowserTool};

/// Register all built-in tools into a registry.
pub fn register_builtin_tools(registry: &ToolRegistry) {
    register_builtin_tools_with_sandbox(registry, true);
}

/// Register built-in tools, optionally with sandbox enforcement on shell_exec.
pub fn register_builtin_tools_with_sandbox(registry: &ToolRegistry, sandboxed: bool) {
    registry.register(Arc::new(CurrentTimeTool));
    registry.register(Arc::new(CalculatorTool));
    registry.register(Arc::new(SleepTool));
    registry.register(Arc::new(HttpFetchTool::new()));
    registry.register(Arc::new(WebSearchTool::unconfigured()));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
    if sandboxed {
        registry.register(Arc::new(SandboxedShellTool::new(
            ShellSandboxConfig::default(),
        )));
    } else {
        registry.register(Arc::new(ShellTool::new(300)));
    }
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(ApplyPatchTool));
    registry.register(Arc::new(SearchInFilesTool));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(UnifiedLspTool));
    registry.register(Arc::new(FileOutlineTool));
    registry.register(Arc::new(CodeChunkTool));
    registry.register(Arc::new(MultiEditTool));
    registry.register(Arc::new(ListDirectoryTool));

    registry.register_deferred(Arc::new(NotebookEditTool));
    registry.register_deferred(Arc::new(TerminalCaptureTool::new()));
}

/// Register web tools with a specific search backend configuration.
pub fn register_web_tools(registry: &ToolRegistry, backend: WebSearchBackend) {
    registry.register(Arc::new(WebSearchTool::new(backend)));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
}

/// Register media generation tools (requires API credentials).
pub fn register_media_tools(registry: &ToolRegistry, base_url: &str, api_key: &str) {
    registry.register(Arc::new(ImageGenerateTool::new(base_url, api_key)));
    registry.register(Arc::new(TtsTool::new(base_url, api_key)));
}

/// Register the unified skill tool (read-only: list + read).
pub fn register_skill_tools(registry: &ToolRegistry, skill_registry: Arc<SkillRegistry>) {
    registry.register(Arc::new(UnifiedSkillTool::readonly(skill_registry)));
}

/// Register the unified skill tool with write support (list + read + write).
pub fn register_skill_tools_full(
    registry: &ToolRegistry,
    skill_registry: Arc<SkillRegistry>,
    workspace: Arc<AgentWorkspace>,
) {
    registry.register(Arc::new(UnifiedSkillTool::new(skill_registry, Some(workspace))));
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
    messages: std::sync::Arc<std::sync::Mutex<Vec<fastclaw_core::types::ChatMessage>>>,
) {
    registry.register(Arc::new(SnipTool::new(messages)));
}

/// Register BriefTool (send_user_message) with shared stream event channels.
pub fn register_brief_tool(
    registry: &ToolRegistry,
    stream_event_txs: std::sync::Arc<dashmap::DashMap<String, tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>>,
) {
    registry.register(Arc::new(BriefTool::new(stream_event_txs)));
}

pub fn register_todo_tools(registry: &ToolRegistry, store: TodoStore) {
    registry.register(Arc::new(TodoWriteTool::new(store)));
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

/// Register plan mode tools (enter/exit/verify) with shared execution mode state.
pub fn register_plan_mode_tools(registry: &ToolRegistry, mode_state: ExecutionModeState) {
    registry.register_deferred(Arc::new(EnterPlanModeTool::new(mode_state.clone())));
    registry.register_deferred(Arc::new(ExitPlanModeTool::new(mode_state.clone())));
    registry.register_deferred(Arc::new(VerifyPlanExecutionTool::new(mode_state)));
}

pub fn register_session_tools(
    registry: &ToolRegistry,
    sessions: Arc<SessionStore>,
    bus: Arc<MessageBus>,
) {
    registry.register(Arc::new(SessionsSpawnTool::new(
        sessions.clone(),
        bus.clone(),
    )));
    registry.register(Arc::new(SessionsSendTool::new(sessions, bus)));
}

