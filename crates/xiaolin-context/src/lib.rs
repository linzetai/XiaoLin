pub mod budget;
pub mod cached_microcompact;
pub mod collapse;
pub mod compressor;
pub mod engine;
pub mod keyword_interceptor;
mod model_context;
pub mod pipeline;
pub mod reactive;
pub mod snip;

pub use budget::{BudgetDecision, StopReason, TokenBudgetTracker};
pub use collapse::{
    project as project_collapses, CollapseEngine, CollapseEngineConfig, CollapseMode,
    CollapseOverlapError, CollapseResult, CollapseSpan, CollapseStore, CollapseSummarizer,
};
pub use compressor::{
    ensure_valid_assistant_messages, estimate_messages_tokens, CompactionResult,
    CompactionStrategy, CompressorConfig, ContextCompactor, LlmLayerSummarizer,
    DEFAULT_IMPORTANCE_MAX_MESSAGES, DEFAULT_IMPORTANCE_RECENT_WINDOW,
    IMPORTANCE_ASSISTANT_WITH_TOOL_CALLS, IMPORTANCE_DEFAULT_CONVERSATION,
    IMPORTANCE_RECENT_MESSAGES, IMPORTANCE_SYSTEM, IMPORTANCE_TOOL_RESULTS,
};
pub use engine::{
    assemble_context, build_default_engine, AgentMemoryIngestHook, AgentPersonalityHook,
    AssembledContext, CompactionHook, ContentFilterHook, ContextBudget, ContextEngine, ContextHook,
    ContextLayers, IngestInput, LayerTokenLimits, MemoryIngestHook, PersonalityHook,
    SandboxAwarenessHook, SystemReminderHook, DEFAULT_COMPACTION_THRESHOLD,
    DEFAULT_MAX_TOOL_RESULT_CHARS, DEFAULT_SYSTEM_REMINDER_INTERVAL_USER_TURNS,
    DEFAULT_SYSTEM_REMINDER_TEXT,
};
pub use keyword_interceptor::MemoryKeywordInterceptor;
pub use model_context::{
    has_explicit_output_limit, infer_context_window_from_model, infer_output_limit_from_model,
    model_supports_vision, model_supports_vision_with_caps, normalize_model_name, TokenLimitType,
};
pub use pipeline::{
    AutoCompactCircuitBreaker, CompactTracking, CompactionMetadata, ContextPipeline, PipelineConfig,
};
pub use cached_microcompact::{
    CachedMicrocompactConfig, CachedMicrocompactResult, CachedMicrocompactor,
};
pub use reactive::{ReactiveCompactResult, ReactiveCompactor, ReactiveCompactorConfig};
pub use snip::{group_by_api_round, ApiRound, SnipCompactor, SnipCompactorConfig, SnipResult};
