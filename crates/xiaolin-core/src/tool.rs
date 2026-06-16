use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{XiaoLinError, XiaoLinResult};

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

/// Controls whether a tool is included in the model-visible tool list.
///
/// - `Direct`: always in the initial tool list sent to the LLM.
/// - `Deferred`: omitted from the initial list; discoverable via `ToolSearchTool`.
///
/// Tools self-declare their exposure via `Tool::exposure()`. The `ToolRegistry`
/// can further override exposure at runtime via `ToolProfile` promote/demote rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolExposure {
    Direct,
    Deferred,
}

/// A named set of exposure overrides applied to the tool pool based on context
/// (execution mode, sub-agent type, etc.).
///
/// - `promote`: tools with `Deferred` exposure that should be treated as `Direct`.
/// - `demote`: tools with `Direct` exposure that should be hidden from the model.
#[derive(Debug, Clone, Default)]
pub struct ToolProfile {
    pub promote: Vec<String>,
    pub demote: Vec<String>,
}

impl ToolProfile {
    pub fn plan_mode() -> Self {
        Self {
            promote: vec!["exit_plan_mode".into()],
            demote: vec!["enter_plan_mode".into()],
        }
    }

    pub fn readonly() -> Self {
        Self {
            promote: vec![],
            demote: vec![
                "write_file".into(),
                "edit_file".into(),
                "multi_edit".into(),
                "shell_exec".into(),
                "shell".into(),
            ],
        }
    }
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
    #[deprecated(note = "use Tool::supports_parallel() instead for per-tool concurrency control")]
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
            metadata: None,
            images,
        }
    }

    /// Convenience: the content the UI should display (prefers `display_output`).
    pub fn ui_output(&self) -> &str {
        self.display_output.as_deref().unwrap_or(&self.output)
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

    /// Whether this tool can safely execute concurrently with other parallel tools.
    /// Tools returning `true` acquire a shared (read) lock; tools returning `false`
    /// acquire an exclusive (write) lock, serializing all concurrent execution.
    /// Default is `false` (conservative). Override for read-only / stateless tools.
    fn supports_parallel(&self) -> bool {
        false
    }

    /// Whether this tool is included in the initial model-visible tool list.
    /// Override to return `ToolExposure::Deferred` for tools that should only
    /// become available after discovery via `ToolSearchTool`.
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }

    /// Whether this tool must remain in the eager set even when the deferred
    /// pipeline bulk-demotes tools (e.g. MCP server tools over token budget).
    /// Default is `false`. MCP tools with `_meta.alwaysLoad: true` return `true`.
    fn force_eager(&self) -> bool {
        false
    }

    /// Deprecated: use `exposure()` instead.
    fn is_deferred(&self) -> bool {
        self.exposure() == ToolExposure::Deferred
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
    /// Per-MCP-server instructions captured from `InitializeResult`.
    /// Key: server ID, Value: instructions text (if provided).
    mcp_instructions: std::sync::RwLock<HashMap<String, String>>,
    /// Cache of per-tool serialized JSON char counts, keyed by registry version.
    /// Avoids re-serializing tool definitions just to estimate token counts.
    json_sizes_cache: std::sync::RwLock<(u64, HashMap<String, usize>)>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let ver = self.version.load(std::sync::atomic::Ordering::Relaxed);
        let cache = self.def_cache.read().expect("def_cache poisoned");
        let mcp_instr = self.mcp_instructions.read().expect("mcp_instructions poisoned");
        let json_sizes = self.json_sizes_cache.read().expect("json_sizes_cache poisoned");
        Self {
            tools: std::sync::RwLock::new(guard.clone()),
            deferred: std::sync::RwLock::new(deferred.clone()),
            channel_scoped: std::sync::RwLock::new(ch_scoped.clone()),
            version: std::sync::atomic::AtomicU64::new(ver),
            def_cache: std::sync::RwLock::new(cache.clone()),
            mcp_instructions: std::sync::RwLock::new(mcp_instr.clone()),
            json_sizes_cache: std::sync::RwLock::new(json_sizes.clone()),
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
            mcp_instructions: std::sync::RwLock::new(HashMap::new()),
            json_sizes_cache: std::sync::RwLock::new((u64::MAX, HashMap::new())),
        }
    }

    fn bump_version(&self) {
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Current registry version. Incremented on every register/unregister/activate.
    /// Callers can snapshot this value and compare later to detect changes.
    pub fn version(&self) -> u64 {
        self.version.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let is_deferred = tool.exposure() == ToolExposure::Deferred;
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        if guard.contains_key(&name) {
            tracing::warn!(tool = %name, "duplicate tool name – overwriting previous registration");
        }
        guard.insert(name.clone(), tool);
        drop(guard);
        let mut def_guard = self.deferred.write().expect("deferred set poisoned");
        if is_deferred {
            def_guard.insert(name);
        } else {
            def_guard.remove(&name);
        }
        drop(def_guard);
        self.bump_version();
    }

    /// Remove all tools whose name starts with `prefix`. Returns the number removed.
    ///
    /// Also cleans the `deferred` and `channel_scoped` sets so stale entries
    /// don't accumulate across hot-reloads.
    pub fn unregister_by_prefix(&self, prefix: &str) -> usize {
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        let before = guard.len();
        let removed_names: Vec<String> = guard
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        guard.retain(|name, _| !name.starts_with(prefix));
        let removed = before - guard.len();
        drop(guard);
        if removed > 0 {
            let mut def_guard = self.deferred.write().expect("deferred set poisoned");
            for name in &removed_names {
                def_guard.remove(name);
            }
            drop(def_guard);
            let mut ch_guard = self
                .channel_scoped
                .write()
                .expect("channel_scoped poisoned");
            for name in &removed_names {
                ch_guard.remove(name);
            }
            drop(ch_guard);
            self.bump_version();
        }
        removed
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.get(name).cloned()
    }

    /// Store instructions for an MCP server. Overwrites any previous value.
    pub fn set_mcp_instructions(&self, server_id: &str, instructions: Option<&str>) {
        let mut guard = self.mcp_instructions.write().expect("mcp_instructions poisoned");
        match instructions {
            Some(instr) if !instr.trim().is_empty() => {
                guard.insert(server_id.to_string(), instr.trim().to_string());
            }
            _ => {
                guard.remove(server_id);
            }
        }
    }

    /// Remove MCP instructions for a server (e.g. on disconnect).
    pub fn remove_mcp_instructions(&self, server_id: &str) {
        let mut guard = self.mcp_instructions.write().expect("mcp_instructions poisoned");
        guard.remove(server_id);
    }

    /// Get a snapshot of all MCP server instructions. Sorted by key.
    pub fn mcp_instructions_snapshot(&self) -> Vec<(String, String)> {
        let guard = self.mcp_instructions.read().expect("mcp_instructions poisoned");
        let mut pairs: Vec<(String, String)> = guard
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
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

    /// Estimate total JSON chars for a set of tool definitions using cached sizes.
    ///
    /// On cache hit (same registry version), looks up pre-computed per-tool sizes.
    /// On cache miss, serializes each definition once and populates the cache.
    /// Avoids re-serializing tool definitions on every LLM call just for token estimation.
    pub fn estimated_json_chars(&self, defs: &[ToolDefinition]) -> usize {
        let current_ver = self.version.load(std::sync::atomic::Ordering::Relaxed);
        {
            let cache = self.json_sizes_cache.read().expect("json_sizes_cache poisoned");
            if cache.0 == current_ver {
                let total: usize = defs
                    .iter()
                    .map(|td| cache.1.get(&td.function.name).copied().unwrap_or(0))
                    .sum();
                if total > 0 {
                    return total;
                }
            }
        }

        let mut new_sizes = HashMap::new();
        let mut total = 0usize;
        for td in defs {
            let size = serde_json::to_string(td).map(|s| s.len()).unwrap_or(0);
            new_sizes.insert(td.function.name.clone(), size);
            total += size;
        }

        if let Ok(mut cache) = self.json_sizes_cache.write() {
            if cache.0 != current_ver {
                *cache = (current_ver, new_sizes);
            } else {
                cache.1.extend(new_sizes);
            }
        }
        total
    }

    /// Returns only definitions whose name starts with `mcp__`, using the cached definitions.
    pub fn mcp_definitions(&self) -> Vec<ToolDefinition> {
        let all = self.definitions();
        all.iter()
            .filter(|td| td.function.name.starts_with("mcp__"))
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
        let mut guard = self
            .channel_scoped
            .write()
            .expect("channel_scoped poisoned");
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
    /// and `search_hint`. Uses BM25-inspired scoring:
    ///  - Term frequency with saturation (k1=1.2) — diminishing returns
    ///  - Inverse document frequency — rarer terms weight more
    ///  - Document length normalization (b=0.75) — shorter docs score higher
    ///  - Prefix matching bonus for query terms ≥3 chars
    ///  - Name match bonus for exact query in tool name
    ///
    /// Results are sorted by score descending; tools with zero matches are excluded.
    pub fn search_deferred(&self, query: &str) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        let q = query.to_lowercase();
        let query_words: Vec<&str> = q.split_whitespace().collect();
        if query_words.is_empty() {
            return Vec::new();
        }

        // Collect deferred tool documents for BM25 scoring
        let deferred_tools: Vec<_> = tools
            .values()
            .filter(|t| deferred.contains(t.name()))
            .collect();

        let n = deferred_tools.len() as f64;
        if n == 0.0 {
            return Vec::new();
        }

        // Build haystacks and compute avg doc length
        let haystacks: Vec<String> = deferred_tools
            .iter()
            .map(|t| format!("{} {} {}", t.name(), t.description(), t.search_hint()).to_lowercase())
            .collect();

        let doc_lengths: Vec<f64> = haystacks
            .iter()
            .map(|h| h.split_whitespace().count() as f64)
            .collect();
        let avg_dl = doc_lengths.iter().sum::<f64>() / n;

        // IDF for each query word: how many docs contain it
        let doc_freqs: Vec<f64> = query_words
            .iter()
            .map(|&qw| {
                haystacks
                    .iter()
                    .filter(|h| h.contains(qw) || (qw.len() >= 3 && h.split_whitespace().any(|w| w.starts_with(qw))))
                    .count() as f64
            })
            .collect();

        // BM25 parameters
        let k1: f64 = 1.2;
        let b: f64 = 0.75;

        let mut scored: Vec<(f64, ToolDefinition)> = deferred_tools
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let haystack = &haystacks[i];
                let haystack_words: Vec<&str> = haystack.split_whitespace().collect();
                let dl = doc_lengths[i];

                let mut total_score: f64 = 0.0;
                let mut matched_terms = 0u32;

                for (qi, &qw) in query_words.iter().enumerate() {
                    // Count term frequency
                    let tf = if haystack.contains(qw) {
                        haystack.matches(qw).count() as f64
                    } else if qw.len() >= 3 && haystack_words.iter().any(|hw| hw.starts_with(qw)) {
                        // Prefix match counts as 0.5 TF
                        0.5
                    } else {
                        continue;
                    };

                    matched_terms += 1;

                    // IDF: log((N - df + 0.5) / (df + 0.5) + 1)
                    let df = doc_freqs[qi];
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

                    // BM25 TF saturation with length normalization
                    let tf_norm = (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * dl / avg_dl));

                    total_score += idf * tf_norm;
                }

                if matched_terms == 0 {
                    return None;
                }

                // Bonus: all query words matched
                if matched_terms == query_words.len() as u32 {
                    total_score *= 1.5;
                }

                // Bonus: tool name contains the full query
                if t.name().to_lowercase().contains(&q) {
                    total_score += 3.0;
                }

                Some((total_score, t.to_definition()))
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(_, def)| def).collect()
    }

    /// Returns definitions filtered by a `ToolProfile`. Profile rules override
    /// the tool's own `exposure()`:
    ///
    /// - Tools in `profile.promote` are included even if normally deferred.
    /// - Tools in `profile.demote` are excluded even if normally direct.
    /// - Channel-scoped tools are always excluded (injected separately).
    pub fn definitions_with_profile(&self, profile: &ToolProfile) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let ch_scoped = self.channel_scoped.read().expect("channel_scoped poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| {
                let n = t.name();
                if ch_scoped.contains(n) {
                    return false;
                }
                if profile.demote.iter().any(|d| d == n) {
                    return false;
                }
                if profile.promote.iter().any(|p| p == n) {
                    return true;
                }
                !deferred.contains(n)
            })
            .map(|t| t.to_definition())
            .collect()
    }

    /// Demote all tools matching `prefix` into the deferred set, except those
    /// where `force_eager()` returns `true`. Returns the number of tools demoted.
    ///
    /// Used by the deferred pipeline to bulk-defer an MCP server's tools when
    /// the total tool token budget exceeds the threshold.
    pub fn demote_to_deferred_by_prefix(&self, prefix: &str) -> usize {
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        let to_demote: Vec<String> = tools
            .iter()
            .filter(|(name, tool)| name.starts_with(prefix) && !tool.force_eager())
            .map(|(name, _)| name.clone())
            .collect();
        drop(tools);

        if to_demote.is_empty() {
            return 0;
        }
        let mut def_guard = self.deferred.write().expect("deferred set poisoned");
        let mut count = 0;
        for name in &to_demote {
            if def_guard.insert(name.clone()) {
                count += 1;
            }
        }
        drop(def_guard);
        if count > 0 {
            self.bump_version();
        }
        count
    }

    /// Names of all tools currently in the deferred set.
    pub fn deferred_tool_names(&self) -> Vec<String> {
        let guard = self.deferred.read().expect("deferred set poisoned");
        guard.iter().cloned().collect()
    }

    /// Returns definitions for MCP tools (name starts with `mcp__`) that are
    /// **not** in the deferred set. Used by prompt injection to describe only
    /// the eager MCP tools available to the model.
    pub fn eager_mcp_definitions(&self) -> Vec<ToolDefinition> {
        let deferred = self.deferred.read().expect("deferred set poisoned");
        let tools = self.tools.read().expect("ToolRegistry poisoned");
        tools
            .values()
            .filter(|t| {
                let n = t.name();
                n.starts_with("mcp__") && !deferred.contains(n)
            })
            .map(|t| t.to_definition())
            .collect()
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
    /// Returns [`XiaoLinError::ToolNotFound`] when the name is missing.
    pub async fn execute_named(&self, name: &str, arguments: &str) -> XiaoLinResult<ToolResult> {
        let tool = {
            let guard = self.tools.read().expect("ToolRegistry poisoned");
            guard
                .get(name)
                .cloned()
                .ok_or_else(|| XiaoLinError::ToolNotFound(name.to_string()))?
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
    async fn pre_tool_use(&self, _ctx: &ToolHookContext) -> PreToolAction {
        PreToolAction::default()
    }

    /// Called after a tool completes. Useful for logging, metrics, or follow-up actions.
    async fn post_tool_use(&self, _ctx: &ToolHookContext, _info: &PostToolInfo) {}
}

// ─── Tool Contributor (Extension Plugin Pattern) ─────────────────────

/// Context provided to ToolContributors when collecting tools.
#[derive(Debug, Clone)]
pub struct ContributorContext {
    pub agent_id: String,
    pub channel_id: Option<String>,
}

/// Trait for extension modules that contribute tools to the registry at runtime.
///
/// Implementations can dynamically provide tools based on configuration, session
/// state, or external plugin discovery. Tools returned by `contribute_tools` are
/// registered into the `ToolRegistry` during initialization or reload.
#[async_trait]
pub trait ToolContributor: Send + Sync {
    /// Unique identifier for this contributor (e.g. "memory", "feishu", "browser").
    fn name(&self) -> &str;

    /// Whether this contributor is currently enabled. Disabled contributors are
    /// skipped during tool collection.
    fn is_enabled(&self) -> bool {
        true
    }

    /// Return the tools this contributor provides. Called during registry
    /// construction or when tools need to be refreshed.
    fn contribute_tools(&self, ctx: &ContributorContext) -> Vec<Arc<dyn Tool>>;

    /// Whether the contributed tools should be registered as deferred (hidden
    /// from initial prompt until discovered via tool_search).
    fn is_deferred(&self) -> bool {
        false
    }

    /// Whether the contributed tools are channel-scoped (only injected for
    /// requests from a specific channel).
    fn is_channel_scoped(&self) -> bool {
        false
    }
}

/// Registry that collects tools from all registered ToolContributors and
/// registers them into a ToolRegistry.
pub struct ContributorRegistry {
    contributors: Vec<Arc<dyn ToolContributor>>,
}

impl ContributorRegistry {
    pub fn new() -> Self {
        Self {
            contributors: Vec::new(),
        }
    }

    pub fn register(&mut self, contributor: Arc<dyn ToolContributor>) {
        self.contributors.push(contributor);
    }

    /// Collect tools from all enabled contributors and register them in the
    /// provided ToolRegistry. Returns the count of tools registered.
    pub fn apply_to_registry(&self, registry: &ToolRegistry, ctx: &ContributorContext) -> usize {
        let mut count = 0;
        for contributor in &self.contributors {
            if !contributor.is_enabled() {
                continue;
            }
            let tools = contributor.contribute_tools(ctx);
            for tool in tools {
                if contributor.is_channel_scoped() {
                    registry.register_channel_scoped(tool);
                } else if contributor.is_deferred() {
                    registry.register_deferred(tool);
                } else {
                    registry.register(tool);
                }
                count += 1;
            }
        }
        count
    }

    pub fn contributor_count(&self) -> usize {
        self.contributors.len()
    }
}

impl Default for ContributorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Dynamic Tools (Config-Defined External Tools) ───────────────────

/// How a dynamic tool is executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicExecutor {
    /// Route execution to an MCP server by server_id.
    Mcp { server_id: String },
    /// Execute via HTTP request to an external endpoint.
    Http { url: String, method: String },
    /// Emit an event on a channel, letting the external client handle execution.
    Event { channel: String },
}

/// A tool defined dynamically via configuration or API, rather than compiled Rust code.
/// Supports deferred registration and flexible execution backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub deferred: bool,
    pub executor: DynamicExecutor,
}

/// Backend that executes dynamic tools based on their executor type.
#[async_trait]
pub trait DynamicExecutorBackend: Send + Sync {
    async fn execute_mcp(&self, server_id: &str, tool_name: &str, arguments: &str) -> ToolResult;
    async fn execute_http(&self, url: &str, method: &str, arguments: &str) -> ToolResult;
    async fn execute_event(&self, channel: &str, tool_name: &str, arguments: &str) -> ToolResult;
}

/// A concrete Tool implementation backed by a DynamicToolSpec and an executor backend.
pub struct DynamicTool {
    pub spec: DynamicToolSpec,
    backend: Arc<dyn DynamicExecutorBackend>,
}

impl DynamicTool {
    pub fn new(spec: DynamicToolSpec, backend: Arc<dyn DynamicExecutorBackend>) -> Self {
        Self { spec, backend }
    }
}

#[async_trait]
impl Tool for DynamicTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let props = self
            .spec
            .parameters_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let required = self
            .spec
            .parameters_schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required,
        }
    }

    fn exposure(&self) -> ToolExposure {
        if self.spec.deferred {
            ToolExposure::Deferred
        } else {
            ToolExposure::Direct
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        match &self.spec.executor {
            DynamicExecutor::Mcp { server_id } => {
                self.backend.execute_mcp(server_id, &self.spec.name, arguments).await
            }
            DynamicExecutor::Http { url, method } => {
                self.backend.execute_http(url, method, arguments).await
            }
            DynamicExecutor::Event { channel } => {
                self.backend.execute_event(channel, &self.spec.name, arguments).await
            }
        }
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
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "A fake tool for testing"
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            ToolParameterSchema {
                schema_type: "object".into(),
                properties: HashMap::new(),
                required: vec![],
            }
        }
        fn search_hint(&self) -> &str {
            self.hint
        }
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
        assert!(!results.is_empty());
        assert_eq!(results[0].function.name, "task_list");
    }

    // ── ToolProfile tests ────────────────────────────────────────

    struct FakeDeferredTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for FakeDeferredTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "deferred tool"
        }
        fn exposure(&self) -> ToolExposure {
            ToolExposure::Deferred
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            ToolParameterSchema {
                schema_type: "object".into(),
                properties: HashMap::new(),
                required: vec![],
            }
        }
        async fn execute(&self, _arguments: &str) -> ToolResult {
            ToolResult::ok("ok")
        }
    }

    #[test]
    fn tool_profile_plan_mode_promotes_exit() {
        let p = ToolProfile::plan_mode();
        assert!(p.promote.contains(&"exit_plan_mode".to_string()));
        assert!(p.demote.contains(&"enter_plan_mode".to_string()));
    }

    #[test]
    fn tool_profile_default_is_empty() {
        let p = ToolProfile::default();
        assert!(p.promote.is_empty());
        assert!(p.demote.is_empty());
    }

    #[test]
    fn definitions_with_profile_promotes_deferred_tool() {
        let reg = ToolRegistry::new();
        reg.register(make_tool("read_file", ""));
        let deferred: Arc<dyn Tool> = Arc::new(FakeDeferredTool { name: "exit_plan_mode" });
        reg.register(deferred);

        let default_defs = reg.definitions_with_profile(&ToolProfile::default());
        let names: Vec<_> = default_defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"exit_plan_mode"), "deferred should be hidden by default");

        let plan_defs = reg.definitions_with_profile(&ToolProfile::plan_mode());
        let names: Vec<_> = plan_defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"exit_plan_mode"), "promote should make deferred visible");
    }

    #[test]
    fn unregister_by_prefix_cleans_deferred_set() {
        let reg = ToolRegistry::new();
        reg.register_deferred(make_tool("mcp__srv__a", ""));
        reg.register_deferred(make_tool("mcp__srv__b", ""));
        reg.register(make_tool("eager_x", ""));

        assert_eq!(reg.deferred_count(), 2);
        let removed = reg.unregister_by_prefix("mcp__srv__");
        assert_eq!(removed, 2);
        assert_eq!(reg.deferred_count(), 0, "deferred set should be cleaned");
        assert_eq!(reg.len(), 1);
        assert!(reg.get("eager_x").is_some());
    }

    #[test]
    fn version_increments_on_mutations() {
        let reg = ToolRegistry::new();
        let v0 = reg.version();
        reg.register(make_tool("a", ""));
        let v1 = reg.version();
        assert!(v1 > v0);
        reg.register_deferred(make_tool("b", ""));
        let v2 = reg.version();
        assert!(v2 > v1);
        reg.unregister_by_prefix("a");
        let v3 = reg.version();
        assert!(v3 > v2);
        reg.activate_deferred("b");
        let v4 = reg.version();
        assert!(v4 > v3);
    }

    #[test]
    fn definitions_with_profile_demotes_direct_tool() {
        let reg = ToolRegistry::new();
        reg.register(make_tool("enter_plan_mode", ""));
        reg.register(make_tool("read_file", ""));

        let plan_defs = reg.definitions_with_profile(&ToolProfile::plan_mode());
        let names: Vec<_> = plan_defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"enter_plan_mode"), "demote should hide direct tool");
    }
}
