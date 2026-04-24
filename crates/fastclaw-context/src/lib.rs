pub mod compressor;
pub mod engine;
pub mod manager;
pub mod user_profile;

pub use compressor::{
    estimate_messages_tokens, CompactionResult, CompactionStrategy, CompressorConfig,
    ContextCompactor, LlmLayerSummarizer, DEFAULT_IMPORTANCE_MAX_MESSAGES,
    DEFAULT_IMPORTANCE_RECENT_WINDOW, IMPORTANCE_ASSISTANT_WITH_TOOL_CALLS,
    IMPORTANCE_DEFAULT_CONVERSATION, IMPORTANCE_RECENT_MESSAGES, IMPORTANCE_SYSTEM,
    IMPORTANCE_TOOL_RESULTS,
};
pub use engine::{
    assemble_context, build_default_engine, AgentMemoryIngestHook, AgentPersonalityHook,
    AssembledContext, CompactionHook, ContextBudget, ContextEngine, ContextHook, ContextLayers,
    IngestInput, LayerTokenLimits, MemoryIngestHook, PersonalityHook, SystemReminderHook,
    DEFAULT_COMPACTION_THRESHOLD, DEFAULT_SYSTEM_REMINDER_INTERVAL_USER_TURNS,
    DEFAULT_SYSTEM_REMINDER_TEXT,
};
pub use manager::ContextManager;
pub use user_profile::{CommunicationStyle, UserProfile};
