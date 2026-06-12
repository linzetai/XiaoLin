pub mod agent_discovery;
pub(crate) mod autofix;
pub mod builtin_tools;
pub mod code_graph;
pub mod guardian;
pub mod llm;
pub mod llm_plugin;
pub mod message_queue;
pub mod process_channel;
pub mod reactive_loop;
pub mod rpc;
pub mod runtime;
pub mod session_bridge;
pub mod sidechain;
pub mod spawn_controller;
pub mod subagent;
pub mod subagent_manager;
pub use xiaolin_tools_code::symbol_index;

pub use agent_discovery::{GetAgentInfoTool, ListAgentsTool};
pub use guardian::{GuardianAssessment, GuardianReviewer};
pub use builtin_tools::{
    engine_by_id, BaiduEngine, BingEngine, BuiltinMetaEngine, GoogleEngine, ImageGenerateTool,
    MemorySearchTool, MemoryStoreTool, Search360Engine, SearchEngine, SearxngEngine, SogouEngine,
    TavilyEngine, TtsTool, UnifiedMemoryTool, WebFetchTool, WebSearchBackend, WebSearchTool,
    BUILTIN_ENGINE_IDS,
};
pub use llm::{
    classify_llm_error, create_provider, create_provider_chain, create_provider_chain_with_plugins,
    create_provider_with_credentials, create_provider_with_plugins, patch_agent_context_windows,
    resolve_context_window, AnthropicProvider, CircuitBreaker, CircuitState, CompletionParams,
    FallbackProvider, LlmApiError, LlmErrorCode, LlmProvider, OpenAiProvider,
};
pub use llm_plugin::{LlmPluginRegistry, MiddlewareLlmProvider, ProcessLlmProvider};
pub use runtime::orchestrator::{map_tool_to_pending_action, ToolOrchestrator};
pub use runtime::prompt_engine::{
    McpServerInfo, PromptContext, PromptEngine, PromptSection, SectionCompute,
};
pub use runtime::prompt_sections;
pub use runtime::query_engine::QueryEngine;
pub use runtime::{
    build_subagent_prompt_block, AgentRuntime, ExecutionResult, SubAgentPromptContext,
};
pub use session_bridge::RuntimeTurnExecutor;
pub use spawn_controller::{
    ActiveAgentInfo, ConcurrencySnapshot, RwState, SessionSnapshot, SlotEvent, SpawnConfig,
    SpawnController, SpawnControllerError,
};
pub use subagent::{
    with_subagent_session_id, ResumeSubagentTool, SendMessageTool, SubAgentGetTool,
    SubAgentListTool, SubAgentTool, TaskStopTool, WaitAgentTool,
};
pub use subagent_manager::{SubAgentInheritedContext, SubAgentManager};
