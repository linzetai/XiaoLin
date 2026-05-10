pub mod agent_discovery;
pub(crate) mod autofix;
pub mod builtin_tools;
pub mod code_graph;
pub mod llm;
pub mod llm_plugin;
pub mod process_channel;
pub mod rpc;
mod runtime;
pub mod subagent;
pub mod subagent_manager;
pub mod symbol_index;

pub use builtin_tools::{
    ImageGenerateTool, MemorySearchTool,
    MemoryStoreTool, UnifiedMemoryTool,
    SearchEngine, SearxngEngine, TavilyEngine, TtsTool, WebFetchTool,
    WebSearchBackend, WebSearchTool,
    GoogleEngine, BaiduEngine, BingEngine, SogouEngine, Search360Engine,
    BuiltinMetaEngine, engine_by_id, BUILTIN_ENGINE_IDS,
};
pub use llm::{
    classify_llm_error, create_provider, create_provider_chain,
    create_provider_chain_with_plugins, create_provider_with_credentials,
    create_provider_with_plugins, patch_agent_context_windows, resolve_context_window,
    AnthropicProvider, CircuitBreaker, CircuitState,
    CompletionParams, FallbackProvider, LlmApiError, LlmErrorCode, LlmProvider, OpenAiProvider,
};
pub use llm_plugin::{LlmPluginRegistry, MiddlewareLlmProvider, ProcessLlmProvider};
pub use runtime::{AgentRuntime, ExecutionResult, SubAgentPromptContext, build_subagent_prompt_block};
pub use runtime::prompt_engine::{McpServerInfo, PromptContext, PromptEngine, PromptSection, SectionCompute};
pub use runtime::prompt_sections as prompt_sections;
pub use runtime::query_engine::QueryEngine;
pub use agent_discovery::{GetAgentInfoTool, ListAgentsTool};
pub use subagent::{SubAgentTool, SubAgentGetTool, SubAgentListTool};
pub use subagent_manager::SubAgentManager;
