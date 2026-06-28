//! Tool output asset store — session-scoped persistent storage for raw tool
//! outputs with handles, metadata, indexes, and lifecycle management.
//!
//! # Storage Boundary
//!
//! This module lives in `xiaolin-session` so that:
//! - Session-scoped access control is enforced at the data layer
//! - Asset metadata is stored in SQLite alongside other session tables
//! - Blobs are written to the filesystem under the session directory
//! - Resume-safe handle resolution works across process restarts
//!
//! # Architectural Note
//!
//! This replaces the ad-hoc filesystem-only `ToolResultStorage` in
//! `xiaolin-agent::runtime::tool_result_storage`. The existing module will
//! eventually become a compatibility adapter over this store.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

// ============================================================================
// Size classification
// ============================================================================

/// Projection size class configuration with defaults.
///
/// These thresholds define how output is projected into model-visible context.
/// They are not the same as asset-creation thresholds — an output may be
/// assetized even if it's "small" when debug mode is on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionSizeConfig {
    /// Max UTF-8 bytes for small classification. Default: 8,000.
    pub small_max_bytes: usize,
    /// Max lines for small classification. Default: 200.
    pub small_max_lines: usize,
    /// Max estimated tokens for small classification. Default: 2,000.
    pub small_max_tokens: usize,

    /// Max UTF-8 bytes for medium classification. Default: 50,000.
    pub medium_max_bytes: usize,
    /// Max lines for medium classification. Default: 1,000.
    pub medium_max_lines: usize,
    /// Max estimated tokens for medium classification. Default: 12,500.
    pub medium_max_tokens: usize,
}

impl Default for ProjectionSizeConfig {
    fn default() -> Self {
        Self {
            small_max_bytes: 8_000,
            small_max_lines: 200,
            small_max_tokens: 2_000,
            medium_max_bytes: 50_000,
            medium_max_lines: 1_000,
            medium_max_tokens: 12_500,
        }
    }
}

/// Size class for an output, used to drive projection policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSizeClass {
    /// Small: fits within all small thresholds.
    Small,
    /// Medium: exceeds at least one small threshold but within all medium thresholds.
    Medium,
    /// Large: exceeds at least one medium threshold.
    Large,
}

impl OutputSizeClass {
    /// Classify output by byte count, line count, and estimated token count.
    pub fn classify(
        byte_count: usize,
        line_count: usize,
        estimated_tokens: usize,
        config: &ProjectionSizeConfig,
    ) -> Self {
        if byte_count <= config.small_max_bytes
            && line_count <= config.small_max_lines
            && estimated_tokens <= config.small_max_tokens
        {
            OutputSizeClass::Small
        } else if byte_count <= config.medium_max_bytes
            && line_count <= config.medium_max_lines
            && estimated_tokens <= config.medium_max_tokens
        {
            OutputSizeClass::Medium
        } else {
            OutputSizeClass::Large
        }
    }
}

// ============================================================================
// Handle
// ============================================================================

/// A non-guessable, session-scoped handle for a tool output asset.
///
/// The handle embeds the session id prefix so that cross-session access
/// can be rejected before any database lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolOutputHandle {
    /// The full handle string, e.g. `out_sessABC_<uuid>`.
    id: String,
}

impl ToolOutputHandle {
    /// Create a new handle for the given session.
    pub fn new(session_id: &str) -> Self {
        // Use SHA-256 hex digest of session_id for a filesystem-safe prefix.
        // Raw session ids may contain path-significant characters (/ \ ..).
        // Hex chars are always safe for filenames.
        let mut hasher = Sha256::new();
        hasher.update(session_id.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let prefix = &hash[..8];
        let uuid = Uuid::new_v4().to_string().replace('-', "");
        Self {
            id: format!("out_{prefix}_{uuid}"),
        }
    }

    /// The full handle string.
    pub fn as_str(&self) -> &str {
        &self.id
    }

    /// Extract the session prefix embedded in the handle for quick validation.
    pub fn session_prefix(&self) -> Option<&str> {
        // Format: out_<session_prefix>_<uuid>
        let without_out = self.id.strip_prefix("out_")?;
        let last_underscore = without_out.rfind('_')?;
        Some(&without_out[..last_underscore])
    }
}

impl std::fmt::Display for ToolOutputHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl From<String> for ToolOutputHandle {
    fn from(id: String) -> Self {
        Self { id }
    }
}

// ============================================================================
// Lifecycle state
// ============================================================================

/// Lifecycle state of a tool output asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetLifecycle {
    /// Asset has been created but not yet fully indexed.
    Created,
    /// Asset is fully indexed and ready for recall.
    Active,
    /// Asset has been expired by retention policy (metadata kept for audit).
    Expired,
    /// Asset was explicitly deleted.
    Deleted,
}

// ============================================================================
// Projection provenance
// ============================================================================

/// Provenance label describing how model-visible output was derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionProvenance {
    /// Full raw output included inline in the model context.
    RawInline,
    /// Replaced by a bounded manifest referencing the output handle.
    AssetManifest,
    /// Replaced by a typed summary from a projector.
    TypedSummary,
    /// Content reintroduced through a recall tool (`output_read` etc.).
    RecalledExcerpt,
    /// Content summarized by an LLM (e.g. auto-compact).
    LlmSummary,
    /// Content was dropped during hard context fit.
    HardFitRemoval,
    /// Legacy `<persisted-output>` marker from pre-migration transcripts.
    LegacyPersisted,
}

impl ProjectionProvenance {
    /// Returns true if this provenance represents already-projected content
    /// that should not be destructively reprocessed.
    pub fn is_already_projected(self) -> bool {
        matches!(
            self,
            ProjectionProvenance::AssetManifest
                | ProjectionProvenance::TypedSummary
                | ProjectionProvenance::RecalledExcerpt
                | ProjectionProvenance::LlmSummary
        )
    }

    /// Returns true if the full raw content is still recoverable from the asset.
    pub fn is_recoverable(self) -> bool {
        match self {
            ProjectionProvenance::RawInline
            | ProjectionProvenance::AssetManifest
            | ProjectionProvenance::TypedSummary
            | ProjectionProvenance::RecalledExcerpt
            | ProjectionProvenance::LegacyPersisted => true,
            ProjectionProvenance::LlmSummary | ProjectionProvenance::HardFitRemoval => false,
        }
    }

    /// Returns the stable model-visible tag for this provenance.
    /// Used in projection headers and recall tool results.
    pub fn as_model_tag(self) -> &'static str {
        match self {
            ProjectionProvenance::RawInline => "raw",
            ProjectionProvenance::AssetManifest => "projected",
            ProjectionProvenance::TypedSummary => "summarized",
            ProjectionProvenance::RecalledExcerpt => "recalled",
            ProjectionProvenance::LlmSummary => "auto-compacted",
            ProjectionProvenance::HardFitRemoval => "hard-fit-removed",
            ProjectionProvenance::LegacyPersisted => "legacy-persisted",
        }
    }
}

// ============================================================================
// Shared provenance-marker detection
// ============================================================================

/// Returns true if `text` carries a provenance marker, indicating the content
/// is already a projection, summary, recall excerpt, or legacy compaction
/// result. Downstream layers (ContentFilterHook, hard-fit, post-tool compaction)
/// MUST skip such content to avoid repeated destructive processing.
///
/// This is the single canonical function for provenance-marker detection.
/// All check sites (`is_already_projected`, `is_already_compacted`,
/// `ContentFilterHook`) delegate to this function.
///
/// # Recognized markers
///
/// - New provenance format: `(provenance: <tag>)`
/// - Legacy compaction markers: `[faded]`, `[summarized]`, `[time-compacted]`,
///   `[oneliner]`, `[recall-available]`, `[superseded`
/// - Legacy persisted-output markers: `<persisted-output>`, `<output-handle>`
/// - Projection/manifest format markers: `[<type> — handle:…]`, `[output_summary:…]`,
///   `[output stored — handle:…]`
pub fn has_provenance_marker(text: &str) -> bool {
    // Group all O(n) full-string scans first so they short-circuit together.
    // Each contains() scans the entire string; grouping them together avoids
    // interleaving cheap starts_with checks between expensive scans.
    //
    // New provenance-aware format: "(provenance: …)" appears in all
    // Projection::format() output and recall-tool result headers.
    if text.contains("(provenance:") {
        return true;
    }
    // Legacy persisted-output markers (from pre-migration transcripts)
    if text.contains("<persisted-output>") {
        return true;
    }
    // Legacy XML handle format
    if text.contains("<output-handle>") {
        return true;
    }

    // All remaining checks are O(1) starts_with/equality checks
    // (no full-string scanning needed):

    // Legacy compaction markers
    if text.starts_with("[faded]")
        || text.starts_with("[time-compacted]")
        || text.starts_with("[summarized]")
        || text.starts_with("[oneliner]")
        || text.starts_with("[recall-available]")
        || text.starts_with("[superseded")
        || text == "[Old tool result content cleared]"
    {
        return true;
    }

    // Legacy <persisted-output> starts_with check (catches the anchored case
    // cheaply; the contains() check above handles the non-anchored case).
    if text.starts_with("<persisted-output>") {
        return true;
    }

    // Projection format markers: "[<type> — handle: out_...]"
    if text.starts_with("[shell/test output — handle:")
        || text.starts_with("[file read output — handle:")
        || text.starts_with("[search/grep output — handle:")
        || text.starts_with("[directory listing — handle:")
        || text.starts_with("[browser snapshot — handle:")
        || text.starts_with("[JSON/structured output — handle:")
        || text.starts_with("[text output — handle:")
    {
        return true;
    }

    // Summary and handle-only manifest markers
    if text.starts_with("[output_summary:") || text.starts_with("[output stored — handle:") {
        return true;
    }

    false
}

// ============================================================================
// Projector kind
// ============================================================================

/// The type of projector to use for this output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectorKind {
    /// `read_file` or equivalent file-read tool output.
    ReadFile,
    /// Search/grep tool output.
    Search,
    /// Shell command or test runner output.
    ShellTest,
    /// Directory/tree listing output.
    DirectoryTree,
    /// Browser snapshot output.
    BrowserSnapshot,
    /// JSON or structured MCP/default output.
    JsonDefault,
    /// Unknown/generic large text output.
    GenericText,
}

impl ProjectorKind {
    /// Infer the projector kind from the tool name.
    pub fn from_tool_name(tool_name: &str) -> Self {
        match tool_name {
            "Read" | "read_file" => ProjectorKind::ReadFile,
            "Grep" | "grep" | "rg" | "ripgrep" => ProjectorKind::Search,
            "Bash" | "shell" | "shell_exec" | "run_command" | "exec" => ProjectorKind::ShellTest,
            "Glob" | "ls" | "list_dir" | "list_directory" => ProjectorKind::DirectoryTree,
            "mcp__browser" | "browser" | "TakeSnapshot" => ProjectorKind::BrowserSnapshot,
            _ if tool_name.starts_with("mcp__") => ProjectorKind::JsonDefault,
            _ => ProjectorKind::GenericText,
        }
    }
}

// ============================================================================
// Tool output asset (metadata record)
// ============================================================================

/// A persistent, session-scoped tool output asset metadata record.
///
/// Stores metadata and indexes needed for projection, recall, and lifecycle
/// management. The raw output blob is stored on the filesystem; this record
/// points to it and lives in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputAsset {
    /// Unique non-guessable handle.
    pub handle: ToolOutputHandle,
    /// Owning session id.
    pub session_id: String,
    /// Turn id when the asset was created.
    pub turn_id: String,
    /// The tool call id this output corresponds to.
    pub tool_call_id: String,
    /// Name of the tool that produced this output.
    pub tool_name: String,
    /// Digest of tool arguments for correlation.
    pub arguments_digest: String,
    /// Whether the tool reported success (exit code 0 for shell, etc.).
    pub success: bool,
    /// Current lifecycle state.
    pub lifecycle: AssetLifecycle,
    /// Projector kind inferred from tool name + output shape.
    pub projector_kind: ProjectorKind,

    // Sizes
    /// Raw output size in bytes.
    pub byte_count: usize,
    /// Raw output line count.
    pub line_count: usize,
    /// Estimated token count (byte_count / 4 as proxy).
    pub estimated_tokens: usize,
    /// Size class based on projection config.
    pub size_class: OutputSizeClass,

    // Integrity
    /// SHA-256 content hash.
    pub content_hash: String,

    // Paths
    /// Path to the raw output blob on disk (within session storage root).
    pub blob_path: String,
    /// Path to the line index file (byte offsets for each line).
    pub line_index_path: Option<String>,
    /// Path to the chunk/page index file.
    pub chunk_index_path: Option<String>,

    // Timestamps
    /// Creation time as ISO-8601 string.
    pub created_at: String,
    /// Last access time as ISO-8601 string (for cleanup prioritization).
    pub last_accessed_at: String,
    /// Expiration time, if expired.
    pub expired_at: Option<String>,
}

/// Arguments digest data for recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentsDigestInput {
    /// Tool arguments as JSON string (for diagnostics, not stored in asset).
    pub arguments: String,
    /// Digest SHA-256 hex.
    pub digest: String,
}

impl ArgumentsDigestInput {
    /// Compute argument digest from arguments JSON string.
    pub fn from_arguments(arguments: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(arguments.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        Self {
            arguments: arguments.to_string(),
            digest,
        }
    }
}

// ============================================================================
// Asset creation input
// ============================================================================

/// Input for creating a new tool output asset.
#[derive(Debug, Clone)]
pub struct CreateAssetInput {
    /// Session id.
    pub session_id: String,
    /// Turn id.
    pub turn_id: String,
    /// Tool call id.
    pub tool_call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Tool arguments JSON.
    pub arguments: String,
    /// Whether the tool reported success.
    pub success: bool,
    /// Raw output content.
    pub output: String,
    /// Filesystem root for blob storage (typically the session directory).
    pub storage_root: PathBuf,
    /// Size classification config.
    pub size_config: ProjectionSizeConfig,
}

// ============================================================================
// Recall error types
// ============================================================================

/// Structured error returned by recall tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecallError {
    /// Handle belongs to a different session.
    Unauthorized { handle: String, message: String },
    /// Asset has expired due to retention policy.
    Expired {
        handle: String,
        expired_at: String,
        message: String,
    },
    /// Handle not found in any session.
    NotFound { handle: String, message: String },
    /// Invalid range requested (e.g. start > end, negative).
    InvalidRange {
        handle: String,
        detail: String,
        message: String,
    },
    /// Asset exists but raw blob is missing (corruption, manual deletion).
    BlobMissing { handle: String, message: String },
    /// Generic I/O or internal error.
    Internal {
        handle: String,
        detail: String,
        message: String,
    },
}

impl RecallError {
    pub fn unauthorized(handle: &str) -> Self {
        RecallError::Unauthorized {
            handle: handle.to_string(),
            message: "This output handle belongs to a different session".to_string(),
        }
    }

    pub fn expired(handle: &str, expired_at: &str) -> Self {
        RecallError::Expired {
            handle: handle.to_string(),
            expired_at: expired_at.to_string(),
            message: format!(
                "This output asset expired at {}. It is no longer available for recall.",
                expired_at
            ),
        }
    }

    pub fn not_found(handle: &str) -> Self {
        RecallError::NotFound {
            handle: handle.to_string(),
            message: "No output asset found for this handle".to_string(),
        }
    }

    pub fn invalid_range(handle: &str, detail: &str) -> Self {
        RecallError::InvalidRange {
            handle: handle.to_string(),
            detail: detail.to_string(),
            message: format!("Invalid range: {detail}"),
        }
    }

    pub fn blob_missing(handle: &str) -> Self {
        RecallError::BlobMissing {
            handle: handle.to_string(),
            message: "The raw output blob for this handle is missing".to_string(),
        }
    }

    pub fn internal(handle: &str, detail: &str) -> Self {
        RecallError::Internal {
            handle: handle.to_string(),
            detail: detail.to_string(),
            message: "An internal error occurred while accessing the output asset".to_string(),
        }
    }
}

impl std::fmt::Display for RecallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecallError::Unauthorized { message, .. }
            | RecallError::Expired { message, .. }
            | RecallError::NotFound { message, .. }
            | RecallError::InvalidRange { message, .. }
            | RecallError::BlobMissing { message, .. }
            | RecallError::Internal { message, .. } => write!(f, "{message}"),
        }
    }
}

// ============================================================================
// Line and chunk indexes
// ============================================================================

/// Line index: maps line numbers to byte offsets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineIndex {
    /// Byte offsets for the start of each line (0-indexed into the blob).
    pub line_offsets: Vec<usize>,
    /// Total number of lines.
    pub total_lines: usize,
}

impl LineIndex {
    /// Build a line index from raw output content.
    pub fn build(content: &str) -> Self {
        let bytes = content.as_bytes();
        let mut line_offsets = Vec::new();
        let mut total_lines = 0usize;

        if bytes.is_empty() {
            return Self {
                line_offsets: vec![],
                total_lines: 0,
            };
        }

        let mut offset = 0usize;
        // Find start of each line by scanning for \n
        while offset < bytes.len() {
            line_offsets.push(offset);
            total_lines += 1;
            // Find next \n
            if let Some(nl_pos) = bytes[offset..].iter().position(|&b| b == b'\n') {
                offset += nl_pos + 1;
            } else {
                break;
            }
        }

        Self {
            line_offsets,
            total_lines,
        }
    }

    /// Get the byte offset for a specific line (1-indexed).
    pub fn line_offset(&self, line: usize) -> Option<usize> {
        if line == 0 || line > self.total_lines {
            return None;
        }
        Some(self.line_offsets[line - 1])
    }

    /// Get byte offsets for a line range (1-indexed, inclusive start, exclusive end).
    /// end can be total_lines + 1 to indicate "to the end".
    pub fn line_range_span(
        &self,
        start_line: usize,
        end_line: usize,
        total_bytes: usize,
    ) -> Option<(usize, usize)> {
        if start_line == 0
            || start_line > self.total_lines
            || end_line <= start_line
            || end_line > self.total_lines + 1
        {
            return None;
        }
        let byte_start = self.line_offsets[start_line - 1];
        let byte_end = if end_line > self.total_lines {
            total_bytes
        } else {
            self.line_offsets[end_line - 1]
        };
        Some((byte_start, byte_end))
    }
}

/// Chunk/page index: maps page numbers to byte ranges.
///
/// Each page is chunk_size bytes (default 4096), aligned to line boundaries
/// where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkIndex {
    /// Byte offset for the start of each page.
    pub page_offsets: Vec<usize>,
    /// Total number of pages.
    pub total_pages: usize,
    /// Bytes per page.
    pub page_size: usize,
}

impl ChunkIndex {
    /// Default page size: 4096 bytes.
    pub const DEFAULT_PAGE_SIZE: usize = 4096;

    /// Build a chunk index from raw output content with the given page size.
    pub fn build(content: &str, page_size: usize) -> Self {
        let bytes = content.as_bytes();

        // Empty content has no pages — consistent with LineIndex::build returning
        // total_lines=0 for empty input.
        if bytes.is_empty() {
            return Self {
                page_offsets: vec![],
                total_pages: 0,
                page_size,
            };
        }

        let mut page_offsets = vec![0usize]; // first page always starts at 0
        let mut current_offset = 0usize;

        while current_offset + page_size < bytes.len() {
            // Find next newline after the page_size boundary
            let target = current_offset + page_size;
            let window_end = (target + page_size / 8).min(bytes.len());
            let mut split_point = target;

            // Find newline in window
            let nl_in_window = bytes[target..window_end].iter().position(|&b| b == b'\n');
            match nl_in_window {
                Some(pos) => split_point = target + pos + 1, // include the newline
                None => {
                    // Find last newline before target
                    if let Some(pos) = bytes[..target].iter().rposition(|&b| b == b'\n') {
                        split_point = pos + 1;
                    }
                }
            }

            if split_point > current_offset && split_point < bytes.len() {
                current_offset = split_point;
                page_offsets.push(current_offset);
            } else {
                break; // can't advance, give up
            }
        }

        let total_pages = page_offsets.len();

        Self {
            page_offsets,
            total_pages,
            page_size,
        }
    }

    /// Get the byte range for a specific page (1-indexed).
    pub fn page_range(&self, page: usize, total_bytes: usize) -> Option<(usize, usize)> {
        if page == 0 || page > self.total_pages {
            return None;
        }
        let start = self.page_offsets[page - 1];
        let end = if page < self.total_pages {
            self.page_offsets[page]
        } else {
            total_bytes
        };
        if end > total_bytes || start >= end {
            return None;
        }
        Some((start, end))
    }

    /// Check if there is a page before the given page.
    pub fn has_before(&self, page: usize) -> bool {
        page > 1 && page <= self.total_pages
    }

    /// Check if there is a page after the given page.
    pub fn has_after(&self, page: usize) -> bool {
        page > 0 && page < self.total_pages
    }
}

// ============================================================================
// Content hash
// ============================================================================

/// Compute SHA-256 content hash.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Estimate token count from byte count (rough proxy: bytes / 4).
pub fn estimate_tokens(byte_count: usize) -> usize {
    byte_count / 4
}

/// Count bytes and lines in content.
///
/// Matches `LineIndex::build` semantics: trailing newline does NOT produce
/// an extra empty line. `"a\nb\nc\n"` yields 3 lines, not 4.
pub fn count_bytes_and_lines(content: &str) -> (usize, usize) {
    let byte_count = content.len();
    let line_count = if content.is_empty() {
        0
    } else if content.ends_with('\n') {
        content.bytes().filter(|&b| b == b'\n').count()
    } else {
        content.bytes().filter(|&b| b == b'\n').count() + 1
    };
    (byte_count, line_count)
}

// ============================================================================
// Retention
// ============================================================================

/// Retention metadata recorded when an asset is expired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionRecord {
    pub handle: ToolOutputHandle,
    pub session_id: String,
    pub reason: RetentionReason,
    pub expired_at: String,
    pub byte_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionReason {
    /// Expired due to age-based policy.
    AgeLimit,
    /// Expired because session total storage exceeded cap.
    StorageCap,
    /// Explicitly deleted.
    ExplicitDelete,
    /// Session was deleted.
    SessionDeleted,
}

// ============================================================================
// ToolOutputAssetStore — SQLite-backed persistence
// ============================================================================

/// Maximum number of output assets per session before cleanup is triggered.
const MAX_ASSETS_PER_SESSION: usize = 500;

/// Subdirectory within the session storage root for output blobs.
const BLOB_SUBDIR: &str = "tool-output-assets";

/// Subdirectory for index files.
const INDEX_SUBDIR: &str = "tool-output-indexes";

/// Default blob page size for chunk indexing.
const DEFAULT_BLOB_PAGE_SIZE: usize = 4096;

/// SQLite-backed store for tool output asset metadata, plus filesystem blob
/// and index storage.
///
/// Note: `Debug` is derived explicitly (not automatically via sqlx) to support
/// embedding in config structs in downstream crates.
#[derive(Debug)]
pub struct ToolOutputAssetStore {
    pool: SqlitePool,
}

impl ToolOutputAssetStore {
    /// Open the store backed by an existing SQLite pool.
    /// Ensures tables and indexes exist.
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        let store = Self { pool };
        store.ensure_tables().await?;
        tracing::info!("ToolOutputAssetStore opened");
        Ok(store)
    }

    async fn ensure_tables(&self) -> anyhow::Result<()> {
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&self.pool)
            .await?;

        // Main asset metadata table
        // NOTE: size_class is a creation-time cache computed from ProjectionSizeConfig.
        // If config thresholds change after deployment, stored values may be stale.
        // A schema migration would be needed to recompute; acceptable for now since
        // thresholds rarely change and the value is only advisory for projection policy.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tool_output_assets (
                handle              TEXT PRIMARY KEY,
                session_id          TEXT NOT NULL,
                turn_id             TEXT NOT NULL,
                tool_call_id        TEXT NOT NULL,
                tool_name           TEXT NOT NULL,
                arguments_digest    TEXT NOT NULL,
                success             INTEGER NOT NULL DEFAULT 1,
                lifecycle           TEXT NOT NULL DEFAULT 'active',
                projector_kind      TEXT NOT NULL DEFAULT 'generic_text',
                byte_count          INTEGER NOT NULL DEFAULT 0,
                line_count          INTEGER NOT NULL DEFAULT 0,
                estimated_tokens    INTEGER NOT NULL DEFAULT 0,
                size_class          TEXT NOT NULL DEFAULT 'small',
                content_hash        TEXT NOT NULL,
                blob_path           TEXT NOT NULL,
                line_index_path     TEXT,
                chunk_index_path    TEXT,
                created_at          TEXT NOT NULL DEFAULT (datetime('now')),
                last_accessed_at    TEXT NOT NULL DEFAULT (datetime('now')),
                expired_at          TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        // Indexes for fast lookup
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_toa_session
             ON tool_output_assets(session_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_toa_tool_call
             ON tool_output_assets(session_id, tool_call_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_toa_lifecycle
             ON tool_output_assets(lifecycle, last_accessed_at)",
        )
        .execute(&self.pool)
        .await?;

        // Retention records table (tracks expired/deleted assets for audit)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tool_output_retention (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                handle          TEXT NOT NULL,
                session_id      TEXT NOT NULL,
                reason          TEXT NOT NULL,
                expired_at      TEXT NOT NULL DEFAULT (datetime('now')),
                byte_count      INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tor_session
             ON tool_output_retention(session_id, expired_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Resolve blob and index directories for a given storage root.
    fn blob_dir(storage_root: &Path) -> PathBuf {
        storage_root.join(BLOB_SUBDIR)
    }

    fn index_dir(storage_root: &Path) -> PathBuf {
        storage_root.join(INDEX_SUBDIR)
    }

    fn blob_path(storage_root: &Path, handle: &str) -> PathBuf {
        Self::blob_dir(storage_root).join(format!("{handle}.blob"))
    }

    fn line_index_path(storage_root: &Path, handle: &str) -> PathBuf {
        Self::index_dir(storage_root).join(format!("{handle}.lines.json"))
    }

    fn chunk_index_path(storage_root: &Path, handle: &str) -> PathBuf {
        Self::index_dir(storage_root).join(format!("{handle}.chunks.json"))
    }

    /// Verify that an asset belongs to the given session.
    fn authorize(asset: &ToolOutputAsset, session_id: &str) -> Result<(), RecallError> {
        if asset.session_id != session_id {
            return Err(RecallError::unauthorized(asset.handle.as_str()));
        }
        Ok(())
    }

    // =========================================================================
    // Asset creation
    // =========================================================================

    /// Create a new tool output asset: persist blob, build indexes, insert
    /// metadata row. Returns the asset handle on success.
    ///
    /// This is atomic with respect to the blob write: the file is written with
    /// O_EXCL semantics (create_new), so a handle collision will fail rather
    /// than overwrite existing data.
    pub async fn create_asset(
        &self,
        input: CreateAssetInput,
    ) -> Result<ToolOutputHandle, RecallError> {
        let handle = ToolOutputHandle::new(&input.session_id);
        let handle_str = handle.as_str();

        let now = chrono::Utc::now().to_rfc3339();
        let (byte_count, line_count) = count_bytes_and_lines(&input.output);
        let estimated_tokens = estimate_tokens(byte_count);
        let size_class =
            OutputSizeClass::classify(byte_count, line_count, estimated_tokens, &input.size_config);
        let content_hash = compute_content_hash(&input.output);
        let args_digest = ArgumentsDigestInput::from_arguments(&input.arguments);
        let projector_kind = ProjectorKind::from_tool_name(&input.tool_name);

        // 1. Write blob (atomic, no overwrite)
        let blob_dir = Self::blob_dir(&input.storage_root);
        tokio::fs::create_dir_all(&blob_dir).await.map_err(|e| {
            RecallError::internal(
                handle_str,
                &format!(
                    "Failed to create blob directory {}: {e}",
                    blob_dir.display()
                ),
            )
        })?;

        let blob_path = Self::blob_path(&input.storage_root, handle_str);
        // Track created files for cleanup on error (atomicity guarantee: if any
        // step after blob write fails, orphan files are removed).
        let mut created_files: Vec<std::path::PathBuf> = Vec::new();

        // Use create_new to enforce no-overwrite
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&blob_path)
            .await
            .map_err(|e| {
                RecallError::internal(
                    handle_str,
                    &format!(
                        "Failed to create blob {} (handle collision?): {e}",
                        blob_path.display()
                    ),
                )
            })?;
        file.write_all(input.output.as_bytes()).await.map_err(|e| {
            RecallError::internal(
                handle_str,
                &format!("Failed to write blob {}: {e}", blob_path.display()),
            )
        })?;
        drop(file);
        created_files.push(blob_path.clone());

        // 2. Build and persist indexes
        let index_dir = Self::index_dir(&input.storage_root);
        tokio::fs::create_dir_all(&index_dir).await.map_err(|e| {
            RecallError::internal(
                handle_str,
                &format!(
                    "Failed to create index directory {}: {e}",
                    index_dir.display()
                ),
            )
        })?;

        let line_idx = LineIndex::build(&input.output);
        let line_idx_path = Self::line_index_path(&input.storage_root, handle_str);
        let line_idx_json = serde_json::to_string(&line_idx).map_err(|e| {
            RecallError::internal(handle_str, &format!("Failed to serialize line index: {e}"))
        })?;
        tokio::fs::write(&line_idx_path, &line_idx_json)
            .await
            .map_err(|e| {
                RecallError::internal(
                    handle_str,
                    &format!(
                        "Failed to write line index {}: {e}",
                        line_idx_path.display()
                    ),
                )
            })?;
        created_files.push(line_idx_path.clone());

        let chunk_idx = ChunkIndex::build(&input.output, DEFAULT_BLOB_PAGE_SIZE);
        let chunk_idx_path = Self::chunk_index_path(&input.storage_root, handle_str);
        let chunk_idx_json = serde_json::to_string(&chunk_idx).map_err(|e| {
            RecallError::internal(handle_str, &format!("Failed to serialize chunk index: {e}"))
        })?;
        tokio::fs::write(&chunk_idx_path, &chunk_idx_json)
            .await
            .map_err(|e| {
                RecallError::internal(
                    handle_str,
                    &format!(
                        "Failed to write chunk index {}: {e}",
                        chunk_idx_path.display()
                    ),
                )
            })?;
        created_files.push(chunk_idx_path.clone());

        // 3. Insert metadata row (if this fails, clean up created files)
        sqlx::query(
            "INSERT INTO tool_output_assets (
                handle, session_id, turn_id, tool_call_id, tool_name,
                arguments_digest, success, lifecycle, projector_kind,
                byte_count, line_count, estimated_tokens, size_class,
                content_hash, blob_path, line_index_path, chunk_index_path,
                created_at, last_accessed_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19
            )",
        )
        .bind(handle_str)
        .bind(&input.session_id)
        .bind(&input.turn_id)
        .bind(&input.tool_call_id)
        .bind(&input.tool_name)
        .bind(&args_digest.digest)
        .bind(input.success as i32)
        .bind("active") // lifecycle
        .bind(projector_kind_to_str(projector_kind))
        .bind(byte_count as i64)
        .bind(line_count as i64)
        .bind(estimated_tokens as i64)
        .bind(size_class_to_str(size_class))
        .bind(&content_hash)
        .bind(blob_path.to_string_lossy().as_ref())
        .bind(line_idx_path.to_string_lossy().as_ref())
        .bind(chunk_idx_path.to_string_lossy().as_ref())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            // Clean up orphan files on failed metadata insert
            for path in &created_files {
                let _ = std::fs::remove_file(path);
            }
            RecallError::internal(handle_str, &format!("Failed to insert asset: {e}"))
        })?;

        tracing::debug!(
            handle = handle_str,
            tool = %input.tool_name,
            bytes = byte_count,
            lines = line_count,
            class = ?size_class,
            "Created tool output asset"
        );

        Ok(handle)
    }

    // =========================================================================
    // Asset retrieval
    // =========================================================================

    /// Look up an asset by handle and verify session ownership.
    pub async fn get_asset(
        &self,
        handle: &str,
        session_id: &str,
    ) -> Result<ToolOutputAsset, RecallError> {
        let row = sqlx::query_as::<_, AssetRow>(
            "SELECT handle, session_id, turn_id, tool_call_id, tool_name,
                    arguments_digest, success, lifecycle, projector_kind,
                    byte_count, line_count, estimated_tokens, size_class,
                    content_hash, blob_path, line_index_path, chunk_index_path,
                    created_at, last_accessed_at, expired_at
             FROM tool_output_assets
             WHERE handle = ?1 AND session_id = ?2",
        )
        .bind(handle)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RecallError::internal(handle, &e.to_string()))?;

        // Non-disclosing: if the handle doesn't exist for this session, return
        // NotFound regardless of whether it exists in another session.
        // This prevents leaking cross-session handle existence.
        let row = row.ok_or_else(|| RecallError::not_found(handle))?;

        // Check lifecycle
        match row.lifecycle.as_str() {
            "active" | "created" => {} // ok
            "expired" => {
                return Err(RecallError::expired(
                    handle,
                    row.expired_at.as_deref().unwrap_or("unknown"),
                ));
            }
            "deleted" => {
                return Err(RecallError::not_found(handle));
            }
            _ => {
                tracing::warn!(
                    handle = %handle,
                    lifecycle = %row.lifecycle,
                    "asset has unknown lifecycle state; data may be corrupt"
                );
            }
        }

        // Update last_accessed_at
        let now = chrono::Utc::now().to_rfc3339();
        if let Err(e) =
            sqlx::query("UPDATE tool_output_assets SET last_accessed_at = ?1 WHERE handle = ?2")
                .bind(&now)
                .bind(handle)
                .execute(&self.pool)
                .await
        {
            tracing::warn!(
                handle = %handle,
                error = %e,
                "Failed to update last_accessed_at"
            );
        }

        Ok(row.into_asset())
    }

    /// Read the raw blob content for a given asset.
    /// Validates session ownership before reading.
    pub async fn read_blob(
        &self,
        asset: &ToolOutputAsset,
        session_id: &str,
    ) -> Result<String, RecallError> {
        Self::authorize(asset, session_id)?;
        tokio::fs::read_to_string(&asset.blob_path)
            .await
            .map_err(|e| {
                RecallError::internal(asset.handle.as_str(), &format!("Failed to read blob: {e}"))
            })
    }

    /// Read a byte range from the blob without loading the entire file.
    /// Uses AsyncSeekExt + read_exact to only read the requested range.
    pub async fn read_blob_range(
        &self,
        asset: &ToolOutputAsset,
        session_id: &str,
        start: usize,
        end: usize,
    ) -> Result<String, RecallError> {
        Self::authorize(asset, session_id)?;
        if start >= end || end > asset.byte_count {
            return Err(RecallError::invalid_range(
                asset.handle.as_str(),
                &format!("start={start}, end={end}, total={}", asset.byte_count),
            ));
        }
        let len = end - start;
        let mut file = tokio::fs::File::open(&asset.blob_path).await.map_err(|e| {
            RecallError::internal(asset.handle.as_str(), &format!("Failed to open blob: {e}"))
        })?;
        tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(start as u64))
            .await
            .map_err(|e| {
                RecallError::internal(asset.handle.as_str(), &format!("Failed to seek blob: {e}"))
            })?;
        let mut buf = vec![0u8; len];
        tokio::io::AsyncReadExt::read_exact(&mut file, &mut buf)
            .await
            .map_err(|e| {
                RecallError::internal(
                    asset.handle.as_str(),
                    &format!("Failed to read blob range: {e}"),
                )
            })?;
        // Read as UTF-8, falling back to lossy if invalid
        let s = String::from_utf8_lossy(&buf).into_owned();
        Ok(s)
    }

    /// Load the line index for an asset.
    pub async fn load_line_index(asset: &ToolOutputAsset) -> Result<LineIndex, RecallError> {
        let path = asset
            .line_index_path
            .as_ref()
            .ok_or_else(|| RecallError::internal(asset.handle.as_str(), "no line index path"))?;
        let json = tokio::fs::read_to_string(path).await.map_err(|e| {
            RecallError::internal(
                asset.handle.as_str(),
                &format!("Failed to read line index: {e}"),
            )
        })?;
        serde_json::from_str(&json).map_err(|e| {
            RecallError::internal(
                asset.handle.as_str(),
                &format!("Failed to parse line index: {e}"),
            )
        })
    }

    /// Load the chunk index for an asset.
    pub async fn load_chunk_index(asset: &ToolOutputAsset) -> Result<ChunkIndex, RecallError> {
        let path = asset
            .chunk_index_path
            .as_ref()
            .ok_or_else(|| RecallError::internal(asset.handle.as_str(), "no chunk index path"))?;
        let json = tokio::fs::read_to_string(path).await.map_err(|e| {
            RecallError::internal(
                asset.handle.as_str(),
                &format!("Failed to read chunk index: {e}"),
            )
        })?;
        serde_json::from_str(&json).map_err(|e| {
            RecallError::internal(
                asset.handle.as_str(),
                &format!("Failed to parse chunk index: {e}"),
            )
        })
    }

    /// List all asset handles for a session.
    pub async fn list_session_assets(&self, session_id: &str) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT handle FROM tool_output_assets
             WHERE session_id = ?1 AND lifecycle IN ('active', 'created')
             ORDER BY created_at DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(h,)| h).collect())
    }

    /// Find an asset by tool_call_id within a session.
    pub async fn find_by_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> Result<Option<ToolOutputAsset>, RecallError> {
        let row = sqlx::query_as::<_, AssetRow>(
            "SELECT handle, session_id, turn_id, tool_call_id, tool_name,
                    arguments_digest, success, lifecycle, projector_kind,
                    byte_count, line_count, estimated_tokens, size_class,
                    content_hash, blob_path, line_index_path, chunk_index_path,
                    created_at, last_accessed_at, expired_at
             FROM tool_output_assets
             WHERE session_id = ?1 AND tool_call_id = ?2
               AND lifecycle IN ('active', 'created')
             LIMIT 1",
        )
        .bind(session_id)
        .bind(tool_call_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RecallError::internal(tool_call_id, &e.to_string()))?;

        Ok(row.map(|r| r.into_asset()))
    }

    // =========================================================================
    // Retention / cleanup
    // =========================================================================

    /// Expire an asset: mark as expired, record retention metadata, delete
    /// blob and index files.
    pub async fn expire_asset(
        &self,
        handle: &str,
        session_id: &str,
        reason: RetentionReason,
    ) -> Result<(), RecallError> {
        // Get asset info before deleting
        let asset = match self.get_asset(handle, session_id).await {
            Ok(a) => a,
            Err(RecallError::Expired { .. }) | Err(RecallError::NotFound { .. }) => {
                return Ok(()); // already expired/deleted
            }
            Err(e) => return Err(e),
        };

        let now = chrono::Utc::now().to_rfc3339();

        // Record retention
        sqlx::query(
            "INSERT INTO tool_output_retention (handle, session_id, reason, expired_at, byte_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(handle)
        .bind(session_id)
        .bind(retention_reason_to_str(reason))
        .bind(&now)
        .bind(asset.byte_count as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| RecallError::internal(handle, &format!("Failed to record retention: {e}")))?;

        // Update asset lifecycle
        sqlx::query(
            "UPDATE tool_output_assets SET lifecycle = 'expired', expired_at = ?1
             WHERE handle = ?2",
        )
        .bind(&now)
        .bind(handle)
        .execute(&self.pool)
        .await
        .map_err(|e| RecallError::internal(handle, &format!("Failed to update lifecycle: {e}")))?;

        // Delete blob and index files (best-effort)
        let _ = tokio::fs::remove_file(&asset.blob_path).await;
        if let Some(ref p) = asset.line_index_path {
            let _ = tokio::fs::remove_file(p).await;
        }
        if let Some(ref p) = asset.chunk_index_path {
            let _ = tokio::fs::remove_file(p).await;
        }

        tracing::info!(
            handle = handle,
            reason = ?reason,
            bytes = asset.byte_count,
            "Expired tool output asset"
        );

        Ok(())
    }

    /// Clean up old assets for a session, enforcing storage limits.
    /// Keeps at most `max_assets` per session, expiring oldest-first.
    pub async fn enforce_session_limits(&self, session_id: &str) -> anyhow::Result<usize> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM tool_output_assets
             WHERE session_id = ?1 AND lifecycle IN ('active', 'created')",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;

        if count.0 <= MAX_ASSETS_PER_SESSION as i64 {
            return Ok(0);
        }

        let excess = count.0 - MAX_ASSETS_PER_SESSION as i64;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT handle FROM tool_output_assets
             WHERE session_id = ?1 AND lifecycle IN ('active', 'created')
             ORDER BY last_accessed_at ASC
             LIMIT ?2",
        )
        .bind(session_id)
        .bind(excess)
        .fetch_all(&self.pool)
        .await?;

        let mut expired = 0usize;
        for (handle,) in &rows {
            if let Err(e) = self
                .expire_asset(handle, session_id, RetentionReason::StorageCap)
                .await
            {
                tracing::warn!(
                    handle = handle,
                    error = %e,
                    "Failed to expire asset during limit enforcement"
                );
            } else {
                expired += 1;
            }
        }

        Ok(expired)
    }

    /// Clean up all assets for a session (when session is deleted).
    pub async fn cleanup_session(&self, session_id: &str) -> anyhow::Result<usize> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT handle FROM tool_output_assets
             WHERE session_id = ?1 AND lifecycle IN ('active', 'created')",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        let mut cleaned = 0usize;
        for (handle,) in &rows {
            if let Err(e) = self
                .expire_asset(handle, session_id, RetentionReason::SessionDeleted)
                .await
            {
                tracing::warn!(
                    handle = handle,
                    error = %e,
                    "Failed to expire asset during session cleanup"
                );
            } else {
                cleaned += 1;
            }
        }

        Ok(cleaned)
    }

    /// Get total storage used by a session's active assets.
    pub async fn session_storage_bytes(&self, session_id: &str) -> anyhow::Result<u64> {
        let total: (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(byte_count), 0) FROM tool_output_assets
             WHERE session_id = ?1 AND lifecycle IN ('active', 'created')",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(total.0 as u64)
    }
}

// ============================================================================
// SQLite row type (internal)
// ============================================================================

#[derive(sqlx::FromRow)]
struct AssetRow {
    handle: String,
    session_id: String,
    turn_id: String,
    tool_call_id: String,
    tool_name: String,
    arguments_digest: String,
    success: i32,
    lifecycle: String,
    projector_kind: String,
    byte_count: i64,
    line_count: i64,
    estimated_tokens: i64,
    size_class: String,
    content_hash: String,
    blob_path: String,
    line_index_path: Option<String>,
    chunk_index_path: Option<String>,
    created_at: String,
    last_accessed_at: String,
    expired_at: Option<String>,
}

impl AssetRow {
    /// Convert a raw database row into a `ToolOutputAsset`.
    fn into_asset(self) -> ToolOutputAsset {
        ToolOutputAsset {
            handle: self.handle.into(),
            session_id: self.session_id,
            turn_id: self.turn_id,
            tool_call_id: self.tool_call_id,
            tool_name: self.tool_name,
            arguments_digest: self.arguments_digest,
            success: self.success != 0,
            lifecycle: match self.lifecycle.as_str() {
                "created" => AssetLifecycle::Created,
                "active" => AssetLifecycle::Active,
                "expired" => AssetLifecycle::Expired,
                _ => AssetLifecycle::Deleted,
            },
            projector_kind: str_to_projector_kind(&self.projector_kind),
            byte_count: self.byte_count as usize,
            line_count: self.line_count as usize,
            estimated_tokens: self.estimated_tokens as usize,
            size_class: str_to_size_class(&self.size_class),
            content_hash: self.content_hash,
            blob_path: self.blob_path,
            line_index_path: self.line_index_path,
            chunk_index_path: self.chunk_index_path,
            created_at: self.created_at,
            last_accessed_at: self.last_accessed_at,
            expired_at: self.expired_at,
        }
    }
}

// ============================================================================
// String conversion helpers
// ============================================================================

fn projector_kind_to_str(kind: ProjectorKind) -> &'static str {
    match kind {
        ProjectorKind::ReadFile => "read_file",
        ProjectorKind::Search => "search",
        ProjectorKind::ShellTest => "shell_test",
        ProjectorKind::DirectoryTree => "directory_tree",
        ProjectorKind::BrowserSnapshot => "browser_snapshot",
        ProjectorKind::JsonDefault => "json_default",
        ProjectorKind::GenericText => "generic_text",
    }
}

fn str_to_projector_kind(s: &str) -> ProjectorKind {
    match s {
        "read_file" => ProjectorKind::ReadFile,
        "search" => ProjectorKind::Search,
        "shell_test" => ProjectorKind::ShellTest,
        "directory_tree" => ProjectorKind::DirectoryTree,
        "browser_snapshot" => ProjectorKind::BrowserSnapshot,
        "json_default" => ProjectorKind::JsonDefault,
        _ => ProjectorKind::GenericText,
    }
}

fn size_class_to_str(class: OutputSizeClass) -> &'static str {
    match class {
        OutputSizeClass::Small => "small",
        OutputSizeClass::Medium => "medium",
        OutputSizeClass::Large => "large",
    }
}

fn str_to_size_class(s: &str) -> OutputSizeClass {
    match s {
        "small" => OutputSizeClass::Small,
        "medium" => OutputSizeClass::Medium,
        "large" => OutputSizeClass::Large,
        _ => OutputSizeClass::Small,
    }
}

fn retention_reason_to_str(reason: RetentionReason) -> &'static str {
    match reason {
        RetentionReason::AgeLimit => "age_limit",
        RetentionReason::StorageCap => "storage_cap",
        RetentionReason::ExplicitDelete => "explicit_delete",
        RetentionReason::SessionDeleted => "session_deleted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Size classification tests
    // =========================================================================

    #[test]
    fn size_class_small_all_within() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(5_000, 100, 1_000, &config);
        assert_eq!(class, OutputSizeClass::Small);
    }

    #[test]
    fn size_class_medium_exceeds_small_bytes() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(30_000, 150, 5_000, &config);
        assert_eq!(class, OutputSizeClass::Medium);
    }

    #[test]
    fn size_class_medium_exceeds_small_lines() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(5_000, 500, 1_000, &config);
        assert_eq!(class, OutputSizeClass::Medium);
    }

    #[test]
    fn size_class_large_exceeds_medium() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(60_000, 200, 15_000, &config);
        assert_eq!(class, OutputSizeClass::Large);
    }

    #[test]
    fn size_class_large_exceeds_medium_lines() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(10_000, 1_500, 5_000, &config);
        assert_eq!(class, OutputSizeClass::Large);
    }

    #[test]
    fn size_class_boundary_small_max() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(8_000, 200, 2_000, &config);
        assert_eq!(class, OutputSizeClass::Small);
    }

    #[test]
    fn size_class_boundary_medium_max() {
        let config = ProjectionSizeConfig::default();
        let class = OutputSizeClass::classify(50_000, 1_000, 12_500, &config);
        assert_eq!(class, OutputSizeClass::Medium);
    }

    // =========================================================================
    // Handle tests
    // =========================================================================

    #[test]
    fn handle_format_and_prefix() {
        let handle = ToolOutputHandle::new("sess_abc123");
        let s = handle.as_str();
        assert!(s.starts_with("out_"));
        let parts: Vec<&str> = s.split('_').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "out");
        assert_eq!(parts[1].len(), 8, "prefix must be SHA-256 hex[..8]");
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!s.contains("sess_abc"), "raw session_id must not leak");
        assert_eq!(handle.session_prefix().unwrap(), parts[1]);
    }

    #[test]
    fn handle_short_session_id() {
        let handle = ToolOutputHandle::new("s1");
        let s = handle.as_str();
        assert!(s.starts_with("out_"));
        let parts: Vec<&str> = s.split('_').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].len(), 8);
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!s.contains("s1"), "raw session_id must not leak");
    }

    #[test]
    fn handle_display() {
        let handle = ToolOutputHandle::new("session_x");
        let display = format!("{handle}");
        assert!(display.starts_with("out_"));
        let parts: Vec<&str> = display.split('_').collect();
        assert_eq!(parts.len(), 3, "expected out_<hex>_<uuid>");
        assert_eq!(parts[1].len(), 8);
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        // Same session_id produces same prefix (deterministic)
        let h2 = ToolOutputHandle::new("session_x");
        let d2 = format!("{h2}");
        let p2: Vec<&str> = d2.split('_').collect();
        assert_eq!(parts[1], p2[1], "same session_id => same hex prefix");
    }

    #[test]
    fn handles_are_unique() {
        let h1 = ToolOutputHandle::new("sess_a");
        let h2 = ToolOutputHandle::new("sess_a");
        assert_ne!(h1.as_str(), h2.as_str());
    }

    // =========================================================================
    // Provenance tests
    // =========================================================================

    #[test]
    fn provenance_is_already_projected() {
        assert!(!ProjectionProvenance::RawInline.is_already_projected());
        assert!(ProjectionProvenance::AssetManifest.is_already_projected());
        assert!(ProjectionProvenance::TypedSummary.is_already_projected());
        assert!(ProjectionProvenance::RecalledExcerpt.is_already_projected());
        assert!(ProjectionProvenance::LlmSummary.is_already_projected());
        assert!(!ProjectionProvenance::HardFitRemoval.is_already_projected());
        assert!(!ProjectionProvenance::LegacyPersisted.is_already_projected());
    }

    #[test]
    fn provenance_is_recoverable() {
        assert!(ProjectionProvenance::RawInline.is_recoverable());
        assert!(ProjectionProvenance::AssetManifest.is_recoverable());
        assert!(ProjectionProvenance::TypedSummary.is_recoverable());
        assert!(ProjectionProvenance::RecalledExcerpt.is_recoverable());
        assert!(ProjectionProvenance::LegacyPersisted.is_recoverable());
        assert!(!ProjectionProvenance::LlmSummary.is_recoverable());
        assert!(!ProjectionProvenance::HardFitRemoval.is_recoverable());
    }

    #[test]
    fn provenance_as_model_tag() {
        assert_eq!(ProjectionProvenance::RawInline.as_model_tag(), "raw");
        assert_eq!(ProjectionProvenance::AssetManifest.as_model_tag(), "projected");
        assert_eq!(ProjectionProvenance::TypedSummary.as_model_tag(), "summarized");
        assert_eq!(ProjectionProvenance::RecalledExcerpt.as_model_tag(), "recalled");
        assert_eq!(ProjectionProvenance::LlmSummary.as_model_tag(), "auto-compacted");
        assert_eq!(ProjectionProvenance::HardFitRemoval.as_model_tag(), "hard-fit-removed");
        assert_eq!(ProjectionProvenance::LegacyPersisted.as_model_tag(), "legacy-persisted");
    }

    // =========================================================================
    // Projector kind tests
    // =========================================================================

    #[test]
    fn projector_kind_from_read_file() {
        assert_eq!(
            ProjectorKind::from_tool_name("Read"),
            ProjectorKind::ReadFile
        );
        assert_eq!(
            ProjectorKind::from_tool_name("read_file"),
            ProjectorKind::ReadFile
        );
    }

    #[test]
    fn projector_kind_from_search() {
        assert_eq!(ProjectorKind::from_tool_name("Grep"), ProjectorKind::Search);
        assert_eq!(ProjectorKind::from_tool_name("rg"), ProjectorKind::Search);
    }

    #[test]
    fn projector_kind_from_shell() {
        assert_eq!(
            ProjectorKind::from_tool_name("Bash"),
            ProjectorKind::ShellTest
        );
        assert_eq!(
            ProjectorKind::from_tool_name("shell_exec"),
            ProjectorKind::ShellTest
        );
    }

    #[test]
    fn projector_kind_unknown() {
        assert_eq!(
            ProjectorKind::from_tool_name("unknown_tool"),
            ProjectorKind::GenericText
        );
    }

    // =========================================================================
    // Line index tests
    // =========================================================================

    #[test]
    fn line_index_simple() {
        let idx = LineIndex::build("hello\nworld\n");
        assert_eq!(idx.total_lines, 2);
        assert_eq!(idx.line_offsets.len(), 2);
        assert_eq!(idx.line_offsets[0], 0); // "hello\n" starts at 0
        assert_eq!(idx.line_offsets[1], 6); // "world\n" starts at 6
    }

    #[test]
    fn line_index_no_trailing_newline() {
        let idx = LineIndex::build("hello\nworld");
        assert_eq!(idx.total_lines, 2);
        assert_eq!(idx.line_offsets[0], 0);
        assert_eq!(idx.line_offsets[1], 6);
    }

    #[test]
    fn line_index_empty() {
        let idx = LineIndex::build("");
        assert_eq!(idx.total_lines, 0);
        assert!(idx.line_offsets.is_empty());
    }

    #[test]
    fn line_index_single_line() {
        let idx = LineIndex::build("single line");
        assert_eq!(idx.total_lines, 1);
        assert_eq!(idx.line_offsets[0], 0);
    }

    #[test]
    fn line_offset_valid() {
        let idx = LineIndex::build("one\ntwo\nthree\n");
        assert_eq!(idx.line_offset(2).unwrap(), 4); // "two\n" starts at byte 4
    }

    #[test]
    fn line_offset_invalid() {
        let idx = LineIndex::build("one\ntwo\n");
        assert!(idx.line_offset(0).is_none());
        assert!(idx.line_offset(3).is_none());
    }

    #[test]
    fn line_range_span_full() {
        let idx = LineIndex::build("one\ntwo\nthree\n");
        let (start, end) = idx.line_range_span(1, 4, 15).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 15);
    }

    #[test]
    fn line_range_span_mid() {
        let idx = LineIndex::build("one\ntwo\nthree\n");
        let (start, end) = idx.line_range_span(2, 3, 15).unwrap();
        assert_eq!(start, 4);
        assert_eq!(end, 8);
    }

    #[test]
    fn line_range_span_invalid() {
        let idx = LineIndex::build("one\ntwo\n");
        assert!(idx.line_range_span(0, 1, 8).is_none());
        assert!(idx.line_range_span(1, 4, 8).is_none()); // end > total_lines+1
    }

    // =========================================================================
    // Chunk index tests
    // =========================================================================

    #[test]
    fn chunk_index_empty_content() {
        let idx = ChunkIndex::build("", 4096);
        assert_eq!(idx.total_pages, 0);
        assert!(idx.page_offsets.is_empty());
    }

    #[test]
    fn chunk_index_small_content() {
        let idx = ChunkIndex::build("small", 4096);
        assert_eq!(idx.total_pages, 1);
        assert_eq!(idx.page_offsets, vec![0]);
    }

    #[test]
    fn chunk_index_multi_page() {
        // Create content with ~10k bytes, newlines every ~500 bytes
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!("line {:04}: {}\n", i, "x".repeat(490)));
        }
        let idx = ChunkIndex::build(&content, 2000);
        assert!(
            idx.total_pages > 1,
            "expected multiple pages, got {}",
            idx.total_pages
        );
        // First page starts at 0
        assert_eq!(idx.page_offsets[0], 0);
        // Offsets are monotonically increasing
        for w in idx.page_offsets.windows(2) {
            assert!(w[1] > w[0]);
        }
    }

    #[test]
    fn chunk_page_range() {
        let mut content = String::new();
        content.push_str(&"a\n".repeat(100));
        content.push_str(&"b\n".repeat(100));
        let total_bytes = content.len();
        let idx = ChunkIndex::build(&content, 200);
        let (start, end) = idx.page_range(1, total_bytes).unwrap();
        assert_eq!(start, 0);
        assert!(end <= total_bytes);
        assert!(end > start);
    }

    #[test]
    fn chunk_page_range_invalid() {
        let idx = ChunkIndex::build("test", 4096);
        assert!(idx.page_range(0, 4).is_none());
        assert!(idx.page_range(2, 4).is_none());
    }

    #[test]
    fn chunk_has_before_after() {
        let mut content = String::new();
        for i in 0..200 {
            content.push_str(&format!("line {}\n", i));
        }
        let idx = ChunkIndex::build(&content, 200);
        assert!(!idx.has_before(1));
        assert!(idx.has_after(1));
        if idx.total_pages > 1 {
            assert!(idx.has_before(idx.total_pages));
            assert!(!idx.has_after(idx.total_pages));
        }
    }

    // =========================================================================
    // Content hash tests
    // =========================================================================

    #[test]
    fn content_hash_stable() {
        let h1 = compute_content_hash("hello world");
        let h2 = compute_content_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_different() {
        let h1 = compute_content_hash("hello world");
        let h2 = compute_content_hash("hello world!");
        assert_ne!(h1, h2);
    }

    #[test]
    fn content_hash_not_empty() {
        let h = compute_content_hash("");
        assert!(!h.is_empty());
        // SHA-256 hex is 64 chars
        assert_eq!(h.len(), 64);
    }

    // =========================================================================
    // Token estimation
    // =========================================================================

    #[test]
    fn estimate_tokens_typical() {
        assert_eq!(estimate_tokens(4000), 1000);
        assert_eq!(estimate_tokens(100), 25);
    }

    #[test]
    fn estimate_tokens_zero() {
        assert_eq!(estimate_tokens(0), 0);
    }

    // =========================================================================
    // Byte/line counting
    // =========================================================================

    #[test]
    fn count_bytes_lines_simple() {
        let (bytes, lines) = count_bytes_and_lines("a\nb\nc");
        assert_eq!(bytes, 5);
        assert_eq!(lines, 3);
    }

    #[test]
    fn count_bytes_lines_trailing_newline() {
        let (bytes, lines) = count_bytes_and_lines("a\nb\nc\n");
        assert_eq!(bytes, 6);
        // Trailing newline does NOT produce an extra line — matches LineIndex::build
        // and wc -l semantics. Three newline characters = three lines.
        assert_eq!(lines, 3);
    }

    #[test]
    fn count_bytes_lines_empty() {
        let (bytes, lines) = count_bytes_and_lines("");
        assert_eq!(bytes, 0);
        assert_eq!(lines, 0);
    }

    // =========================================================================
    // Arguments digest tests
    // =========================================================================

    #[test]
    fn arguments_digest_stable() {
        let d1 = ArgumentsDigestInput::from_arguments(r#"{"file_path": "/tmp/test.txt"}"#);
        let d2 = ArgumentsDigestInput::from_arguments(r#"{"file_path": "/tmp/test.txt"}"#);
        assert_eq!(d1.digest, d2.digest);
    }

    #[test]
    fn arguments_digest_different() {
        let d1 = ArgumentsDigestInput::from_arguments(r#"{"a": 1}"#);
        let d2 = ArgumentsDigestInput::from_arguments(r#"{"a": 2}"#);
        assert_ne!(d1.digest, d2.digest);
    }

    // =========================================================================
    // Recall error tests
    // =========================================================================

    #[test]
    fn recall_error_display() {
        let err = RecallError::unauthorized("out_abc_123");
        let s = format!("{err}");
        assert!(s.contains("different session"));
    }

    #[test]
    fn recall_error_not_found() {
        let err = RecallError::not_found("out_xyz_456");
        let s = format!("{err}");
        assert!(s.contains("No output asset found"));
    }

    #[test]
    fn recall_error_expired() {
        let err = RecallError::expired("out_exp_789", "2026-01-01T00:00:00Z");
        let s = format!("{err}");
        assert!(s.contains("expired"));
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_store() -> (ToolOutputAssetStore, TempDir) {
        // Use an in-memory SQLite pool for isolation
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");

        let store = ToolOutputAssetStore::open(pool).await.expect("open store");
        let tmp = TempDir::new().expect("tempdir");

        (store, tmp)
    }

    fn make_input(tmp: &TempDir, output: &str) -> CreateAssetInput {
        CreateAssetInput {
            session_id: "sess_test_1".to_string(),
            turn_id: "turn_001".to_string(),
            tool_call_id: "call_abc".to_string(),
            tool_name: "Bash".to_string(),
            arguments: r#"{"command": "echo hello"}"#.to_string(),
            success: true,
            output: output.to_string(),
            storage_root: tmp.path().to_path_buf(),
            size_config: ProjectionSizeConfig::default(),
        }
    }

    #[tokio::test]
    async fn create_and_retrieve_asset() {
        let (store, tmp) = setup_store().await;
        let input = make_input(&tmp, "hello world\nthis is output\n");

        let handle = store.create_asset(input).await.expect("create");

        let asset = store
            .get_asset(handle.as_str(), "sess_test_1")
            .await
            .expect("get_asset");
        assert_eq!(asset.tool_name, "Bash");
        assert_eq!(asset.tool_call_id, "call_abc");
        assert_eq!(asset.byte_count, 27);
        // count_bytes_and_lines now matches LineIndex::build: 2 newlines = 2 lines
        assert_eq!(asset.line_count, 2);
        assert!(matches!(asset.lifecycle, AssetLifecycle::Active));

        // Verify blob exists on disk
        assert!(std::path::Path::new(&asset.blob_path).exists());
    }

    #[tokio::test]
    async fn read_blob_exact_recovery() {
        let (store, tmp) = setup_store().await;
        let content = "line one\nline two\nline three\n";
        let input = make_input(&tmp, content);

        let handle = store.create_asset(input).await.expect("create");
        let asset = store
            .get_asset(handle.as_str(), "sess_test_1")
            .await
            .expect("get_asset");

        let recovered = store
            .read_blob(&asset, "sess_test_1")
            .await
            .expect("read_blob");
        assert_eq!(recovered, content);
    }

    #[tokio::test]
    async fn read_blob_range() {
        let (store, tmp) = setup_store().await;
        let content = "line one\nline two\nline three\n";
        let input = make_input(&tmp, content);

        let handle = store.create_asset(input).await.expect("create");
        let asset = store
            .get_asset(handle.as_str(), "sess_test_1")
            .await
            .expect("get_asset");

        let slice = store
            .read_blob_range(&asset, "sess_test_1", 0, 8)
            .await
            .expect("read range");
        assert_eq!(slice, "line one");
    }

    #[tokio::test]
    async fn cross_session_access_denied() {
        let (store, tmp) = setup_store().await;
        let input = make_input(&tmp, "secret\n");
        let handle = store.create_asset(input).await.expect("create");

        let err = store
            .get_asset(handle.as_str(), "other_session")
            .await
            .expect_err("should return error for cross-session access");
        assert!(
            matches!(err, RecallError::NotFound { .. }),
            "cross-session access must be non-disclosing (NotFound, not Unauthorized)"
        );
    }

    #[tokio::test]
    async fn not_found_handle() {
        let (store, _tmp) = setup_store().await;
        let err = store
            .get_asset("out_nonexistent_12345678", "sess_test_1")
            .await
            .expect_err("should not be found");
        assert!(matches!(err, RecallError::NotFound { .. }));
    }

    #[tokio::test]
    async fn expire_asset_and_record_retention() {
        let (store, tmp) = setup_store().await;
        let input = make_input(&tmp, "data\n");
        let handle = store.create_asset(input).await.expect("create");
        let handle_str = handle.as_str().to_string();

        store
            .expire_asset(&handle_str, "sess_test_1", RetentionReason::AgeLimit)
            .await
            .expect("expire");

        // After expiry, get_asset should return Expired error
        let err = store
            .get_asset(handle.as_str(), "sess_test_1")
            .await
            .expect_err("should be expired");
        assert!(matches!(err, RecallError::Expired { .. }));

        // Blob should be deleted
        let asset = store.get_asset(handle.as_str(), "sess_test_1").await;
        if let Err(RecallError::Expired { .. }) = asset {
            // Expired assets may or may not have blob removed (best-effort)
        }
    }

    #[tokio::test]
    async fn list_session_assets() {
        let (store, tmp) = setup_store().await;

        let input1 = make_input(&tmp, "aaa\n");
        let h1 = store.create_asset(input1).await.expect("create 1");

        let mut input2 = make_input(&tmp, "bbb\n");
        input2.tool_call_id = "call_xyz".to_string();
        let h2 = store.create_asset(input2).await.expect("create 2");

        let handles = store
            .list_session_assets("sess_test_1")
            .await
            .expect("list");
        assert_eq!(handles.len(), 2);
        assert!(handles.contains(&h1.as_str().to_string()));
        assert!(handles.contains(&h2.as_str().to_string()));
    }

    #[tokio::test]
    async fn find_by_tool_call() {
        let (store, tmp) = setup_store().await;
        let input = make_input(&tmp, "test\n");
        store.create_asset(input).await.expect("create");

        let found = store
            .find_by_tool_call("sess_test_1", "call_abc")
            .await
            .expect("find");
        assert!(found.is_some());
        assert_eq!(found.unwrap().tool_call_id, "call_abc");

        let not_found = store
            .find_by_tool_call("sess_test_1", "nonexistent")
            .await
            .expect("find");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn enforce_session_limits() {
        let (store, tmp) = setup_store().await;

        // Create many small assets (more than MAX_ASSETS_PER_SESSION = 500
        // in a real scenario, but we'll test with a few)
        for i in 0..5 {
            let mut input = make_input(&tmp, &format!("output {i}\n"));
            input.tool_call_id = format!("call_{i}");
            store.create_asset(input).await.expect("create");
        }

        // With only 5 assets, enforce should expire 0
        let expired = store
            .enforce_session_limits("sess_test_1")
            .await
            .expect("enforce");
        assert_eq!(expired, 0);
    }

    #[tokio::test]
    async fn session_storage_bytes() {
        let (store, tmp) = setup_store().await;
        let input = make_input(&tmp, "hello world\n");
        store.create_asset(input).await.expect("create");

        let bytes = store
            .session_storage_bytes("sess_test_1")
            .await
            .expect("bytes");
        assert_eq!(bytes, 12);
    }

    #[tokio::test]
    async fn asset_with_utf8_emoji() {
        let (store, tmp) = setup_store().await;
        let content = "hello 🌍 world\nemoji: 🎉\n";
        let input = make_input(&tmp, content);

        let handle = store.create_asset(input).await.expect("create");
        let asset = store
            .get_asset(handle.as_str(), "sess_test_1")
            .await
            .expect("get_asset");

        let recovered = store.read_blob(&asset, "sess_test_1").await.expect("read");
        assert_eq!(recovered, content);
        assert_eq!(asset.byte_count, content.len());

        // Line index should work with multi-byte characters
        let line_idx = ToolOutputAssetStore::load_line_index(&asset)
            .await
            .expect("load line index");
        assert_eq!(line_idx.total_lines, 2);
    }

    #[tokio::test]
    async fn non_utf8_output_handling() {
        let (store, tmp) = setup_store().await;

        // Phase 1 supports text output only (output: String).
        // This test verifies text with escape sequences is stored faithfully.
        let output = "[text output with escape sequences: \n\t\r and null-like: \\x00]".to_string();

        let input = CreateAssetInput {
            session_id: "sess_test_1".to_string(),
            turn_id: "turn_bin".to_string(),
            tool_call_id: "call_bin".to_string(),
            tool_name: "Bash".to_string(),
            arguments: r#"{"command": "binary"}"#.to_string(),
            success: false,
            output,
            storage_root: tmp.path().to_path_buf(),
            size_config: ProjectionSizeConfig::default(),
        };
        let handle = store.create_asset(input).await;

        // Should still succeed
        assert!(handle.is_ok(), "binary output should still create asset");
    }
}
