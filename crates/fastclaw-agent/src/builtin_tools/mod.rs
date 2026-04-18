mod ask_question;
mod code_intel;
mod filesystem;
mod hub;
mod identity;
mod lsp_manager;
mod media;
mod memory;
mod network;
mod session;
mod shell;
mod skill;
mod utility;

#[cfg(feature = "browser")]
pub mod browser;

use std::sync::Arc;

use fastclaw_core::bus::MessageBus;
use fastclaw_core::skill::SkillRegistry;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::workspace::AgentWorkspace;
use fastclaw_session::SessionStore;

pub use filesystem::{
    ApplyPatchTool, EditFileTool, ListDirectoryTool, ReadFileTool, SearchInFilesTool, WriteFileTool,
};
pub use filesystem::with_file_access_mode;
pub use hub::{HubInstallTool, HubSearchTool};
pub use media::{ImageGenerateTool, TtsTool};
pub use memory::{MemorySearchTool, MemoryStoreTool};
pub use network::{
    DuckDuckGoEngine, HttpFetchTool, SearchEngine, SearchResult, SearxngEngine, TavilyEngine,
    WebFetchTool, WebSearchBackend, WebSearchTool,
};
pub use session::{session_inbox_topic, SessionsSendTool, SessionsSpawnTool};
pub use shell::{SandboxedShellTool, ShellSandboxConfig, ShellTool};
pub use identity::{GetIdentityTool, SetIdentityTool};
pub use skill::{ListSkillsTool, ReadSkillTool, WriteSkillTool};
pub use ask_question::{AskQuestionTool, with_stream_context};
pub use code_intel::{FindReferencesTool, GoToDefinitionTool, WorkspaceSymbolsTool};
pub use utility::{CalculatorTool, CurrentTimeTool};

#[cfg(feature = "browser")]
pub use browser::{register_browser_tool, BrowserTool};

/// Register all built-in tools into a registry.
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    register_builtin_tools_with_sandbox(registry, true);
}

/// Register built-in tools, optionally with sandbox enforcement on shell_exec.
pub fn register_builtin_tools_with_sandbox(registry: &mut ToolRegistry, sandboxed: bool) {
    registry.register(Arc::new(CurrentTimeTool));
    registry.register(Arc::new(CalculatorTool));
    registry.register(Arc::new(HttpFetchTool::new()));
    registry.register(Arc::new(WebSearchTool::with_defaults()));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
    if sandboxed {
        registry.register(Arc::new(SandboxedShellTool::new(
            ShellSandboxConfig::default(),
        )));
    } else {
        registry.register(Arc::new(ShellTool::new(30)));
    }
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(ApplyPatchTool));
    registry.register(Arc::new(SearchInFilesTool));
    registry.register(Arc::new(WorkspaceSymbolsTool));
    registry.register(Arc::new(GoToDefinitionTool));
    registry.register(Arc::new(FindReferencesTool));
    registry.register(Arc::new(ListDirectoryTool));
}

/// Register web tools with a specific search backend configuration.
pub fn register_web_tools(registry: &mut ToolRegistry, backend: WebSearchBackend) {
    registry.register(Arc::new(WebSearchTool::new(backend)));
    registry.register(Arc::new(WebFetchTool::with_defaults()));
}

/// Register media generation tools (requires API credentials).
pub fn register_media_tools(registry: &mut ToolRegistry, base_url: &str, api_key: &str) {
    registry.register(Arc::new(ImageGenerateTool::new(base_url, api_key)));
    registry.register(Arc::new(TtsTool::new(base_url, api_key)));
}

/// Register skill-related tools (list_skills, read_skill).
pub fn register_skill_tools(registry: &mut ToolRegistry, skill_registry: Arc<SkillRegistry>) {
    registry.register(Arc::new(ListSkillsTool::new(skill_registry.clone())));
    registry.register(Arc::new(ReadSkillTool::new(skill_registry)));
}

/// Register skill tools plus the write_skill tool (requires agent workspace).
pub fn register_skill_tools_full(
    registry: &mut ToolRegistry,
    skill_registry: Arc<SkillRegistry>,
    workspace: Arc<AgentWorkspace>,
) {
    registry.register(Arc::new(ListSkillsTool::new(skill_registry.clone())));
    registry.register(Arc::new(ReadSkillTool::new(skill_registry)));
    registry.register(Arc::new(WriteSkillTool::new(workspace)));
}

/// Register ClawHub marketplace tools.
pub fn register_hub_tools(
    registry: &mut ToolRegistry,
    hub: Arc<tokio::sync::Mutex<fastclaw_core::hub::HubClient>>,
) {
    registry.register(Arc::new(HubSearchTool::new(hub.clone())));
    registry.register(Arc::new(HubInstallTool::new(hub)));
}

/// Register identity tools (get_identity, set_identity) for reading/writing SOUL.md, USER.md, AGENTS.md.
pub fn register_identity_tools(registry: &mut ToolRegistry, workspace: Arc<AgentWorkspace>) {
    registry.register(Arc::new(GetIdentityTool::new(workspace.clone())));
    registry.register(Arc::new(SetIdentityTool::new(workspace)));
}

pub fn register_session_tools(
    registry: &mut ToolRegistry,
    sessions: Arc<SessionStore>,
    bus: Arc<MessageBus>,
) {
    registry.register(Arc::new(SessionsSpawnTool::new(
        sessions.clone(),
        bus.clone(),
    )));
    registry.register(Arc::new(SessionsSendTool::new(sessions, bus)));
}
