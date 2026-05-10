use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{FastClawError, FastClawResult};

/// Functional domain groups for tools. Used to selectively expose
/// relevant tools based on task context, reducing model selection cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolGroup {
    /// File system operations: read, write, edit, glob, list
    File,
    /// Code intelligence: LSP, symbols, search, outline
    Code,
    /// Web and network: fetch, search, APIs
    Web,
    /// System operations: shell, process, terminal
    System,
    /// Communication: sessions, messages, channels
    Communication,
    /// Memory and knowledge: store, search, recall
    Memory,
    /// Meta/utility: time, sleep, plan mode, todo
    Utility,
    /// Browser automation
    Browser,
    /// Git version control
    Git,
    /// Task and workflow management
    Task,
}

/// Categorizes a tool by the nature of its operation.
/// Used for concurrent scheduling (read-only tools run in parallel)
/// and for permission decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolKind {
    /// Pure reads: read_file, list_dir, etc. Safe to run concurrently.
    Read,
    /// Text search: grep, glob, workspace_symbols. Safe to run concurrently.
    Search,
    /// Network fetch: web_fetch, web_search, http_fetch. Safe to run concurrently.
    Fetch,
    /// File writes: write_file, edit_file, apply_patch. Must be serialized.
    Edit,
    /// Shell/process execution: shell_exec. Must be serialized.
    Execute,
    /// Informational: calculator, current_time. Safe to run concurrently.
    Think,
    /// Other/uncategorized tools.
    Other,
}

impl ToolKind {
    /// Whether this kind of tool is safe to execute concurrently with others.
    pub fn is_concurrency_safe(&self) -> bool {
        matches!(self, Self::Read | Self::Search | Self::Fetch | Self::Think)
    }
}

/// JSON Schema describing a tool's parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameterSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

/// OpenAI-compatible tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameterSchema,
}

/// Structured error types for tool failures.
/// Helps the agent understand *why* a tool failed and pick the right recovery strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorType {
    // ── General ──
    InvalidToolParams,
    Unknown,
    ExecutionFailed,
    ExecutionDenied,

    // ── File system ──
    FileNotFound,
    FileWriteFailure,
    ReadContentFailure,
    AttemptToCreateExistingFile,
    FileTooLarge,
    PermissionDenied,
    NoSpaceLeft,
    TargetIsDirectory,
    PathNotInWorkspace,
    SearchPathNotFound,
    SearchPathNotADirectory,

    // ── Edit ──
    EditPreparationFailure,
    EditNoOccurrenceFound,
    EditMultipleOccurrences,
    EditNoChange,

    // ── Search / glob / ls ──
    GlobExecutionError,
    GrepExecutionError,
    LsExecutionError,
    PathIsNotADirectory,

    // ── Shell ──
    ShellExecuteError,

    // ── Network ──
    WebFetchFailed,
    HttpFetchFailed,

    // ── LSP ──
    LspUnavailable,
    LspRequestFailed,

    // ── MCP ──
    McpToolError,

    // ── Truncation ──
    OutputTruncated,
}

impl std::fmt::Display for ToolErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

/// An image to be sent to the LLM as a multimodal content part.
#[derive(Debug, Clone)]
pub struct ToolImage {
    /// MIME type, e.g. "image/png", "image/jpeg".
    pub mime_type: String,
    /// Raw image bytes (will be base64-encoded when sent to the LLM).
    pub data: Vec<u8>,
}

/// Result of a tool execution.
///
/// `output` is what the LLM sees (may be summarized/truncated by the runtime).
/// `display_output` is an optional richer representation for the UI (images, tables, full data).
/// When `display_output` is `None`, the UI falls back to `output`.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    /// Richer output for the UI. Falls back to `output` when `None`.
    pub display_output: Option<String>,
    /// Structured error classification when `success` is `false`.
    pub error_type: Option<ToolErrorType>,
    /// When `true`, the runtime should pause and ask the user for confirmation
    /// before retrying this tool call. Used by the dangerous-ops-policy `confirm` mode.
    pub needs_confirmation: bool,
    /// Optional structured metadata for the UI (e.g. file info, diff stats).
    /// Not sent to the LLM; consumed by frontend components for richer rendering.
    pub metadata: Option<serde_json::Value>,
    /// Images to include as multimodal content parts in the LLM message.
    /// When non-empty, the runtime constructs a content array with both text
    /// and image_url parts so the LLM can visually interpret the images.
    pub images: Vec<ToolImage>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            display_output: None,
            error_type: None,
            needs_confirmation: false,
            metadata: None,
            images: vec![],
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: error.into(),
            display_output: None,
            error_type: Some(ToolErrorType::Unknown),
            needs_confirmation: false,
            metadata: None,
            images: vec![],
        }
    }

    /// Error with a specific structured error type.
    pub fn typed_err(error_type: ToolErrorType, message: impl Into<String>) -> Self {
        Self {
            success: false,
            output: message.into(),
            display_output: None,
            error_type: Some(error_type),
            needs_confirmation: false,
            metadata: None,
            images: vec![],
        }
    }

    /// Build a result with separate LLM and UI outputs.
    pub fn ok_split(llm_output: impl Into<String>, display: impl Into<String>) -> Self {
        Self {
            success: true,
            output: llm_output.into(),
            display_output: Some(display.into()),
            error_type: None,
            needs_confirmation: false,
            metadata: None,
            images: vec![],
        }
    }

    /// Build a result with text output and images for multimodal LLM consumption.
    pub fn ok_with_images(output: impl Into<String>, images: Vec<ToolImage>) -> Self {
        Self {
            success: true,
            output: output.into(),
            display_output: None,
            error_type: None,
            needs_confirmation: false,
            metadata: None,
            images,
        }
    }

    /// Convenience: the content the UI should display (prefers `display_output`).
    pub fn ui_output(&self) -> &str {
        self.display_output.as_deref().unwrap_or(&self.output)
    }

    /// A dangerous operation was detected and requires user confirmation before proceeding.
    /// The runtime will automatically present a confirmation dialog to the user.
    /// If approved, the tool is re-executed with `"confirmed": true` injected.
    pub fn needs_confirm(description: impl Into<String>) -> Self {
        let desc = description.into();
        Self {
            success: false,
            output: format!("⚠️ Dangerous operation — awaiting user confirmation.\n{desc}"),
            display_output: None,
            error_type: None,
            needs_confirmation: true,
            metadata: None,
            images: vec![],
        }
    }
}

/// Callback for tools to report intermediate progress during execution.
/// Send messages through this channel to emit `ToolProgress` stream events.
pub type ProgressSender = tokio::sync::mpsc::Sender<ToolProgressUpdate>;

/// A progress update sent by a tool during execution.
#[derive(Debug, Clone)]
pub struct ToolProgressUpdate {
    /// Human-readable progress message
    pub message: String,
    /// Optional numeric progress (0.0 to 1.0)
    pub progress: Option<f64>,
    /// Optional partial output accumulated so far
    pub partial_output: Option<String>,
}

/// Trait all tools must implement.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> ToolParameterSchema;

    /// Tool category for concurrent scheduling and permission decisions.
    /// Defaults to `Other`; override in tool implementations.
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn execute(&self, arguments: &str) -> ToolResult;

    /// Execute with a progress reporting channel. Override this in tools that
    /// benefit from streaming progress (e.g., shell_exec, web_fetch, browser).
    /// Default implementation ignores the sender and calls `execute`.
    async fn execute_with_progress(
        &self,
        arguments: &str,
        _progress: ProgressSender,
    ) -> ToolResult {
        self.execute(arguments).await
    }

    /// Whether this tool supports progress reporting.
    /// When `true`, the executor will call `execute_with_progress` instead of `execute`.
    fn supports_progress(&self) -> bool {
        false
    }

    /// Functional domain group for selective tool exposure.
    /// Tools can belong to multiple groups but return their primary group here.
    /// Default is `Utility`. Override in tools to enable group-based filtering.
    fn group(&self) -> ToolGroup {
        ToolGroup::Utility
    }

    /// Additional keywords that help the tool-search system match this tool
    /// against free-text queries. Default is empty (name + description used).
    fn search_hint(&self) -> &str {
        ""
    }

    /// Deferred tools are not included in the initial prompt's tool list.
    /// They become available only after activation via `ToolSearchTool`.
    fn is_deferred(&self) -> bool {
        false
    }

    /// Maximum characters of tool result output before persistence to disk.
    /// Default: 100_000. Return `usize::MAX` to opt out of both per-result
    /// persistence and per-message budget enforcement (use sparingly).
    fn max_result_size_chars(&self) -> usize {
        100_000
    }

    /// Rich behavioral guidance sent to the LLM as the tool's description.
    /// Override to provide detailed usage instructions, anti-patterns, examples,
    /// and constraints beyond the short `description()` shown in the UI.
    /// Default returns `description()`.
    fn prompt(&self) -> String {
        self.description().to_string()
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: self.name().to_string(),
                description: self.prompt(),
                parameters: self.parameters_schema(),
            },
        }
    }
}

/// Registry holding all available tools.
///
/// Uses interior `RwLock` so tools can be dynamically registered/unregistered
/// through a shared `Arc<ToolRegistry>` without external mutability.
///
/// Tool definitions are cached and only rebuilt when the registry changes (version bump).
pub struct ToolRegistry {
    tools: std::sync::RwLock<HashMap<String, Arc<dyn Tool>>>,
    deferred: std::sync::RwLock<HashSet<String>>,
    channel_scoped: std::sync::RwLock<HashSet<String>>,
    version: std::sync::atomic::AtomicU64,
    def_cache: std::sync::RwLock<(u64, Arc<Vec<ToolDefinition>>)>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let ver = self.version.load(std::sync::atomic::Ordering::Relaxed);
        let cache = self.def_cache.read().expect("def_cache poisoned");
        Self {
            tools: std::sync::RwLock::new(guard.clone()),
            deferred: std::sync::RwLock::new(deferred.clone()),
            channel_scoped: std::sync::RwLock::new(ch_scoped.clone()),
            version: std::sync::atomic::AtomicU64::new(ver),
            def_cache: std::sync::RwLock::new(cache.clone()),
        }
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::sync::RwLock::new(HashMap::new()),
            deferred: std::sync::RwLock::new(HashSet::new()),
            channel_scoped: std::sync::RwLock::new(HashSet::new()),
            version: std::sync::atomic::AtomicU64::new(0),
            def_cache: std::sync::RwLock::new((u64::MAX, Arc::new(Vec::new()))),
        }
    }

    fn bump_version(&self) {
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        if guard.contains_key(&name) {
            tracing::warn!(tool = %name, "duplicate tool name – overwriting previous registration");
        }
        guard.insert(name, tool);
        drop(guard);
        self.bump_version();
    }

    /// Remove all tools whose name starts with `prefix`. Returns the number removed.
    pub fn unregister_by_prefix(&self, prefix: &str) -> usize {
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        let before = guard.len();
        guard.retain(|name, _| !name.starts_with(prefix));
        let removed = before - guard.len();
        drop(guard);
        if removed > 0 {
            self.bump_version();
        }
        removed
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.get(name).cloned()
    }

    /// Returns cached tool definitions. Rebuilt only when tools are registered/unregistered.
    pub fn definitions(&self) -> Arc<Vec<ToolDefinition>> {
        let current_ver = self.version.load(std::sync::atomic::Ordering::Relaxed);
        {
            let cache = self.def_cache.read().expect("def_cache poisoned");
            if cache.0 == current_ver {
                return cache.1.clone();
            }
        }
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        let defs: Vec<ToolDefinition> = guard.values().map(|t| t.to_definition()).collect();
        let arc = Arc::new(defs);
        if let Ok(mut cache) = self.def_cache.write() {
            *cache = (current_ver, arc.clone());
        }
        arc
    }

    /// Returns only definitions whose name starts with `mcp_`, using the cached definitions.
    pub fn mcp_definitions(&self) -> Vec<ToolDefinition> {
        let all = self.definitions();
        all.iter()
            .filter(|td| td.function.name.starts_with("mcp_"))
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.is_empty()
    }

    pub fn len(&self) -> usize {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.len()
    }

    /// Return the names of all registered tools (eager + deferred).
    pub fn tool_names(&self) -> HashSet<String> {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.keys().cloned().collect()
    }

    /// Register a tool as deferred. Deferred tools are stored in the
    /// registry but excluded from `eager_definitions()`. They become
    /// visible after `activate_deferred()`.
    pub fn register_deferred(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.register(tool);
        let mut guard = self.deferred.write().expect("deferred set poisoned");
        guard.insert(name);
    }

    /// Register a tool as channel-scoped. Channel-scoped tools are stored in
    /// the registry (so `get()` can find them for execution) but excluded from
    /// `eager_definitions()`. They are only injected into requests originating
    /// from the corresponding channel via `request.tools`.
    pub fn register_channel_scoped(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.register(tool);
        let mut guard = self.channel_scoped.write().expect("channel_scoped poisoned");
        guard.insert(name);
    }

    /// Returns definitions for channel-scoped tools only.
    pub fn channel_scoped_definitions(&self) -> Vec<ToolDefinition> {
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| ch_scoped.contains(t.name()))
            .map(|t| t.to_definition())
            .collect()
    }

    /// Returns definitions for tools that are **not** deferred and **not** channel-scoped.
    pub fn eager_definitions(&self) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| {
                let n = t.name();
                !deferred.contains(n) && !ch_scoped.contains(n)
            })
            .map(|t| t.to_definition())
            .collect()
    }

    /// Returns eager tool definitions filtered to only include tools from the specified groups.
    /// Useful for reducing the tool set exposed to the model based on task context.
    pub fn eager_definitions_for_groups(&self, groups: &[ToolGroup]) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| {
                let n = t.name();
                !deferred.contains(n) && !ch_scoped.contains(n) && groups.contains(&t.group())
            })
            .map(|t| t.to_definition())
            .collect()
    }

    /// Returns the set of all groups present among eager (non-deferred, non-channel-scoped) tools.
    pub fn available_groups(&self) -> HashSet<ToolGroup> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| {
                let n = t.name();
                !deferred.contains(n) && !ch_scoped.contains(n)
            })
            .map(|t| t.group())
            .collect()
    }

    /// Search deferred tools by matching `query` against name, description,
    /// and `search_hint`. Uses a scoring approach:
    /// - All query words match (exact substring) → highest score
    /// - Partial word matches (prefix ≥3 chars) → medium score
    /// - At least one word matches → included with lower score
    /// Results are sorted by score descending; tools with zero matches are excluded.
    pub fn search_deferred(&self, query: &str) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        let q = query.to_lowercase();
        let query_words: Vec<&str> = q.split_whitespace().collect();
        if query_words.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(u32, ToolDefinition)> = tools
            .values()
            .filter(|t| deferred.contains(t.name()))
            .filter_map(|t| {
                let haystack = format!(
                    "{} {} {}",
                    t.name(),
                    t.description(),
                    t.search_hint()
                )
                .to_lowercase();
                let haystack_words: Vec<&str> = haystack.split_whitespace().collect();

                let mut score: u32 = 0;
                let mut matched_words = 0u32;

                for &qw in &query_words {
                    if haystack.contains(qw) {
                        score += 10;
                        matched_words += 1;
                    } else if qw.len() >= 3 {
                        let has_prefix = haystack_words.iter().any(|hw| hw.starts_with(qw));
                        if has_prefix {
                            score += 5;
                            matched_words += 1;
                        }
                    }
                }

                if matched_words == 0 {
                    return None;
                }

                if matched_words == query_words.len() as u32 {
                    score += 20;
                }

                if t.name().to_lowercase().contains(&q) {
                    score += 15;
                }

                Some((score, t.to_definition()))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, def)| def).collect()
    }

    /// Move a deferred tool into the eager set so it appears in
    /// `eager_definitions()` going forward. Returns `true` if the tool
    /// was found in the deferred set.
    pub fn activate_deferred(&self, name: &str) -> bool {
        let mut guard = self.deferred.write().expect("deferred set poisoned");
        let removed = guard.remove(name);
        if removed {
            drop(guard);
            self.bump_version();
        }
        removed
    }

    /// Number of tools currently in the deferred set.
    pub fn deferred_count(&self) -> usize {
        let guard = self.deferred.read().expect("deferred set poisoned");
        guard.len()
    }

    /// Execute a registered tool by name.
    ///
    /// Returns [`FastClawError::ToolNotFound`] when the name is missing.
    pub async fn execute_named(&self, name: &str, arguments: &str) -> FastClawResult<ToolResult> {
        let tool = {
            let guard = self.tools.read().expect("ToolRegistry poisoned");
            guard
                .get(name)
                .cloned()
                .ok_or_else(|| FastClawError::ToolNotFound(name.to_string()))?
        };
        Ok(tool.execute(arguments).await)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tool Lifecycle Hooks ────────────────────────────────────────────

/// Context passed to tool hooks before or after execution.
#[derive(Debug, Clone)]
pub struct ToolHookContext {
    pub tool_name: String,
    pub tool_kind: ToolKind,
    pub call_id: String,
    pub arguments: String,
    pub agent_id: String,
}

/// Result modifications a hook can request before tool execution.
#[derive(Debug, Default)]
pub struct PreToolAction {
    /// If set, the tool call is blocked and this message is returned to the LLM instead.
    pub block_reason: Option<String>,
    /// If set, replaces the original arguments string.
    pub modified_arguments: Option<String>,
}

/// Information passed to post-tool hooks.
#[derive(Debug, Clone)]
pub struct PostToolInfo {
    pub success: bool,
    pub output_len: usize,
    pub latency_ms: u64,
}

/// Trait for hooks that observe or modify tool execution lifecycle.
#[async_trait]
pub trait ToolHook: Send + Sync {
    fn name(&self) -> &str;

    /// Called before a tool is executed. Can block or modify the call.
    async fn pre_tool_use(
        &self,
        _ctx: &ToolHookContext,
    ) -> PreToolAction {
        PreToolAction::default()
    }

    /// Called after a tool completes. Useful for logging, metrics, or follow-up actions.
    async fn post_tool_use(
        &self,
        _ctx: &ToolHookContext,
        _info: &PostToolInfo,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTool {
        name: &'static str,
        hint: &'static str,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str { self.name }
        fn description(&self) -> &str { "A fake tool for testing" }
        fn parameters_schema(&self) -> ToolParameterSchema {
            ToolParameterSchema {
                schema_type: "object".into(),
                properties: HashMap::new(),
                required: vec![],
            }
        }
        fn search_hint(&self) -> &str { self.hint }
        async fn execute(&self, _arguments: &str) -> ToolResult {
            ToolResult::ok("ok")
        }
    }

    fn make_tool(name: &'static str, hint: &'static str) -> Arc<dyn Tool> {
        Arc::new(FakeTool { name, hint })
    }

    #[test]
    fn deferred_not_in_eager_definitions() {
        let reg = ToolRegistry::new();
        reg.register(make_tool("eager_a", ""));
        reg.register_deferred(make_tool("deferred_b", ""));

        let eager = reg.eager_definitions();
        assert_eq!(eager.len(), 1);
        assert_eq!(eager[0].function.name, "eager_a");
    }

    #[test]
    fn search_deferred_matches_name_description_hint() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("web_fetch", "http download"));
        reg.register_deferred(make_tool("grep_tool", "regex search"));
        reg.register(make_tool("eager_x", ""));

        let results = reg.search_deferred("http");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function.name, "web_fetch");

        let results = reg.search_deferred("regex");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function.name, "grep_tool");

        let results = reg.search_deferred("eager");
        assert!(results.is_empty(), "eager tools not in deferred search");
    }

    #[test]
    fn activate_moves_to_eager() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("lazy_tool", ""));

        assert!(reg.eager_definitions().is_empty());
        assert!(reg.activate_deferred("lazy_tool"));
        assert_eq!(reg.eager_definitions().len(), 1);
        assert_eq!(reg.eager_definitions()[0].function.name, "lazy_tool");
    }

    #[test]
    fn activate_nonexistent_returns_false() {
        let reg = ToolRegistry::new();
        assert!(!reg.activate_deferred("nope"));
    }

    #[test]
    fn deferred_tool_still_accessible_via_get() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("hidden", ""));
        assert!(reg.get("hidden").is_some());
    }

    #[test]
    fn search_deferred_multi_word_query() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("web_fetch", "http download curl"));
        let results = reg.search_deferred("http curl");
        assert_eq!(results.len(), 1);
        // Partial match: "http" matches but "missing" doesn't → still returned (partial)
        let results = reg.search_deferred("http missing");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_deferred_no_match_returns_empty() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("web_fetch", "http download"));
        let results = reg.search_deferred("completely_unrelated_xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn search_deferred_prefix_match() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("notebook_edit", "jupyter notebook"));
        // "note" (4 chars) should prefix-match "notebook"
        let results = reg.search_deferred("note");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function.name, "notebook_edit");
    }

    #[test]
    fn search_deferred_sorted_by_relevance() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("task_list", "list running tasks"));
        reg.register_deferred(make_tool("task_stop", "stop cancel terminate task"));
        // "task list" → task_list should rank higher (all words match + name match)
        let results = reg.search_deferred("task list");
        assert!(results.len() >= 1);
        assert_eq!(results[0].function.name, "task_list");
    }
}
