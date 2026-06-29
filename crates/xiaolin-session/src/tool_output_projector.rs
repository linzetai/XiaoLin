//! Typed output projectors — deterministic, model-visible summaries of
//! tool output assets.
//!
//! Each projector accepts asset metadata plus optional raw output content
//! and produces a [`Projection`] with a typed summary, bounded excerpt,
//! output handle, and recall guidance.
//!
//! # Design
//!
//! The projector trait lives in `xiaolin-session` to avoid crate dependency
//! cycles. It depends only on the data types already defined in
//! `tool_output_store` — it does NOT access the database or filesystem.
//! Raw content access is provided by the caller via `&str`.
//!
//! # Stability
//!
//! Projection output MUST be deterministic for the same input (no volatile
//! timestamps, blob paths, or random identifiers in model-visible text).
//! This is critical for prompt-cache stability.

use crate::tool_output_store::{ProjectionProvenance, ProjectorKind, ToolOutputAsset};

// ============================================================================
// Excerpt bounds
// ============================================================================

/// Max total bytes for any excerpt (per-line + per-excerpt cap).
const EXCERPT_MAX_BYTES: usize = 4_000;
/// Max bytes per single excerpt line (truncate longer lines with "...").
const EXCERPT_MAX_LINE_BYTES: usize = 500;
/// Max failure blocks collected by find_failure_blocks.
const FAILURE_BLOCKS_MAX: usize = 20;
/// Max total bytes for failure blocks excerpt.
const FAILURE_BLOCKS_MAX_BYTES: usize = 2_000;

/// Truncate a line to at most `max_bytes` bytes, respecting UTF-8 character
/// boundaries. Appends "..." if truncated.
fn truncate_line(line: &str, max_bytes: usize) -> String {
    if line.len() <= max_bytes {
        return line.to_string();
    }
    // Find the last char boundary at or before max_bytes - 3 (reserve room for "...")
    let cutoff = (max_bytes.saturating_sub(3)).min(line.len());
    let idx = line.floor_char_boundary(cutoff);
    format!("{}...", &line[..idx])
}

// ============================================================================
// Projection output
// ============================================================================

/// The result of projecting a tool output asset into model-visible text.
///
/// Composed of: a typed summary (always present), an optional bounded
/// excerpt, the output handle, and recall guidance.
#[derive(Debug, Clone)]
pub struct Projection {
    /// Human-readable type label (e.g. "shell/test output").
    pub type_label: &'static str,
    /// Typed summary lines: tool identity, size, status, key fields.
    pub summary_lines: Vec<String>,
    /// Bounded excerpt from the raw output (optional, for medium/small).
    pub excerpt: Option<String>,
    /// The output handle for recall.
    pub handle: String,
    /// The projection provenance describing how this output was derived.
    pub provenance: ProjectionProvenance,
    /// Suggested recall tools and usage guidance.
    pub recall_guidance: Vec<String>,
    /// Whether the output indicates a failure (shell exit != 0, test fail).
    pub is_failure: bool,
}

impl Projection {
    /// Format the projection as a single string suitable for model context.
    /// The format is deterministic: fields appear in a stable order with
    /// no timestamps or random values.
    pub fn format(&self) -> String {
        let mut out = String::new();

        // Header line with type, provenance, and handle
        out.push_str(&format!(
            "[{} (provenance: {}) — handle: {}]\n",
            self.type_label,
            self.provenance.as_model_tag(),
            self.handle
        ));

        // Summary lines
        for line in &self.summary_lines {
            out.push_str(&format!("- {line}\n"));
        }

        // Excerpt
        if let Some(ref excerpt) = self.excerpt {
            out.push_str("\n--- excerpt ---\n");
            out.push_str(excerpt);
            if !excerpt.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("--- end excerpt ---\n");
        }

        // Recall guidance
        if !self.recall_guidance.is_empty() {
            out.push('\n');
            for guidance in &self.recall_guidance {
                out.push_str(&format!("{guidance}\n"));
            }
        }

        out
    }

    /// Estimate the token count of the formatted output.
    /// Uses a cheap field-length heuristic to avoid allocating the full
    /// formatted string; this is a rough proxy (byte/4) consistent with
    /// `estimate_tokens` in `tool_output_store`.
    pub fn estimated_tokens(&self) -> usize {
        let mut sum: usize = 0;
        // Header: "[<type_label> (provenance: <provenance>) — handle: <handle>]\n"
        sum +=
            30 + self.type_label.len() + self.provenance.as_model_tag().len() + self.handle.len();
        // Summary lines: "- <line>\n" each
        for line in &self.summary_lines {
            sum += 2 + line.len() + 1;
        }
        // Excerpt
        if let Some(ref e) = self.excerpt {
            sum += "\n--- excerpt ---\n".len() + e.len() + "\n--- end excerpt ---\n".len();
        }
        // Recall guidance: "<guidance>\n" each
        if !self.recall_guidance.is_empty() {
            sum += 1; // leading newline
            for g in &self.recall_guidance {
                sum += g.len() + 1;
            }
        }
        sum / 4
    }
}

// ============================================================================
// Projector trait
// ============================================================================

/// Trait for projecting raw tool output into a bounded, typed model-visible
/// representation.
///
/// Implementations are stateless: they accept asset metadata and the raw
/// output string, and return a [`Projection`].
pub trait OutputProjector: Send + Sync {
    /// The [`ProjectorKind`] this projector handles.
    fn kind(&self) -> ProjectorKind;

    /// Human-readable type label for this projector (e.g. "shell/test output").
    fn type_label(&self) -> &'static str;

    /// Produce a projection from asset metadata and raw output.
    ///
    /// `raw_output` may be empty for very large assets where the caller
    /// decides not to load the full content.
    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection;

    /// Produce a projection from asset metadata only (no raw content loaded).
    /// Used as a fallback for very large assets.
    ///
    /// The default implementation produces a safe metadata-only summary that
    /// never reports misleading counts (e.g. "0 matches" when we simply don't
    /// have the data).
    fn project_metadata_only(&self, asset: &ToolOutputAsset) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = vec![
            format!(
                "Tool: {} (metadata only — raw content not loaded)",
                asset.tool_name
            ),
            format!(
                "Size: {} bytes, {} lines, ~{} tokens",
                asset.byte_count, asset.line_count, asset.estimated_tokens
            ),
        ];
        if !asset.success {
            summary_lines.push("Status: FAILED".to_string());
        }
        Projection {
            type_label: self.type_label(),
            summary_lines,
            excerpt: None,
            handle,
            provenance: ProjectionProvenance::TypedSummary,
            recall_guidance: vec![
                format!(
                    "output_read handle={} start_line=1 end_line=500 — read content",
                    asset.handle.as_str()
                ),
                format!(
                    "output_search handle={} pattern=<keyword> — search within content",
                    asset.handle.as_str()
                ),
                format!(
                    "output_tail handle={} lines=50 — view the end of output",
                    asset.handle.as_str()
                ),
            ],
            is_failure: !asset.success,
        }
    }
}

// ============================================================================
// === Projector implementations ===
// ============================================================================

// ============================================================================
// 1. ReadFile projector
// ============================================================================

/// Projector for file-read tool outputs (`read_file`, `Read`).
///
/// Produces a projection with path hint, byte/line counts, a bounded head excerpt
/// (30 lines), and recall guidance for reading additional line ranges or searching.
pub struct ReadFileProjector;

impl OutputProjector for ReadFileProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::ReadFile
    }

    fn type_label(&self) -> &'static str {
        "file read output"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        summary_lines.push(format!("Tool: {} (file read)", asset.tool_name));
        summary_lines.push(format!(
            "Size: {}, {} lines, ~{} tokens",
            format_bytes(asset.byte_count),
            asset.line_count,
            asset.estimated_tokens
        ));

        // Excerpt: bounded head of the file content (byte-capped)
        let excerpt = if !raw_output.is_empty() {
            let lines: Vec<&str> = raw_output.lines().collect();
            let max_excerpt_lines = 30usize;
            let mut result: Vec<String> = Vec::new();
            let mut total_bytes: usize = 0;
            let mut stopped_early = false;
            let limit = lines.len().min(max_excerpt_lines);

            for l in &lines[..limit] {
                let formatted = format!("  {}", truncate_line(l, EXCERPT_MAX_LINE_BYTES));
                if total_bytes + formatted.len() > EXCERPT_MAX_BYTES {
                    stopped_early = true;
                    break;
                }
                total_bytes += formatted.len();
                result.push(formatted);
            }

            let omitted = if stopped_early {
                lines.len() - result.len()
            } else if lines.len() > max_excerpt_lines {
                lines.len() - max_excerpt_lines
            } else {
                0
            };

            let mut shown = result.join("\n");
            if omitted > 0 {
                shown.push_str(&format!("\n  ... ({} more lines)", omitted));
            }
            Some(shown)
        } else {
            None
        };

        // Recall guidance
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — read the first 500 lines"
        ));
        if asset.line_count > 500 {
            recall_guidance.push(format!(
                "output_read handle={handle} start_line={} end_line={} — read the last 500 lines",
                asset.line_count.saturating_sub(499),
                asset.line_count
            ));
        }
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<keyword> — search within the file"
        ));

        Projection {
            type_label: "file read output",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure: false,
        }
    }
}

// ============================================================================
// 2. Search/grep projector
// ============================================================================

/// Projector for search/grep tool outputs (`Grep`, `rg`, `ripgrep`).
///
/// Parses structured `filename:lineno:content` format to extract file
/// distribution and representative matches. Falls back to line-by-line
/// counting when the output doesn't follow expected patterns.
pub struct SearchProjector;

/// Struct holding parsed search output stats.
struct SearchStats {
    total_matches: usize,
    files_matched: Vec<String>,
    match_lines: Vec<String>,
}

/// Try to parse structured search output (grep/rg results).
/// Falls back gracefully if the output doesn't follow expected patterns.
fn parse_grep_output(raw: &str) -> SearchStats {
    let lines: Vec<&str> = raw.lines().collect();
    let mut file_set = std::collections::HashSet::new();
    let mut files = Vec::new();
    let mut matches = Vec::new();
    let mut total = 0usize;

    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // grep -n format: filename:lineno:content
        // rg format: filename:lineno:content
        // Windows paths: C:\Users\...\file.rs:lineno:content
        let search_from = if line.len() >= 3
            && line.as_bytes()[0].is_ascii_alphabetic()
            && line.as_bytes()[1] == b':'
        {
            // Skip past the drive letter colon (e.g. "C:")
            2
        } else {
            0
        };
        if let Some(colon_pos) = line[search_from..].find(':').map(|p| p + search_from) {
            let prefix = &line[..colon_pos];
            // Check if the prefix looks like a filename (not just a number)
            if !prefix.chars().all(|c| c.is_ascii_digit()) {
                if let Some(second_colon) = line[colon_pos + 1..].find(':') {
                    let lineno_part = &line[colon_pos + 1..colon_pos + 1 + second_colon];
                    if lineno_part.chars().all(|c| c.is_ascii_digit()) {
                        if file_set.insert(prefix.to_string()) {
                            files.push(prefix.to_string());
                        }
                        matches.push(line.to_string());
                        total += 1;
                        continue;
                    }
                }
            }
        }
        // If we can't parse, still count as a match
        matches.push(line.to_string());
        total += 1;
    }

    SearchStats {
        total_matches: total,
        files_matched: files,
        match_lines: matches,
    }
}

impl OutputProjector for SearchProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::Search
    }

    fn type_label(&self) -> &'static str {
        "search/grep output"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        summary_lines.push(format!("Tool: {} (search/grep)", asset.tool_name));
        summary_lines.push(format!(
            "Raw size: {} bytes, {} lines, ~{} tokens",
            asset.byte_count, asset.line_count, asset.estimated_tokens
        ));

        // Parse search output for structured stats
        let stats = parse_grep_output(raw_output);

        summary_lines.push(format!("Total matches: {}", stats.total_matches));

        if !stats.files_matched.is_empty() {
            summary_lines.push(format!("Files matched: {}", stats.files_matched.len()));
            let file_list: Vec<String> = stats
                .files_matched
                .iter()
                .take(20)
                .map(|f| format!("  {f}"))
                .collect();
            if stats.files_matched.len() > 20 {
                summary_lines.push(format!(
                    "(showing first 20 of {} files)",
                    stats.files_matched.len()
                ));
            }
            summary_lines.extend(file_list);
        }

        // Representative matches (top N, byte-capped)
        let max_representative = 15usize;
        let mut excerpt_parts: Vec<String> = Vec::new();
        let mut total_bytes: usize = 0;
        excerpt_parts.push("Top matches:".to_string());
        for m in stats.match_lines.iter().take(max_representative) {
            let line = format!("  {}", truncate_line(m, EXCERPT_MAX_LINE_BYTES));
            if total_bytes + line.len() > EXCERPT_MAX_BYTES {
                break;
            }
            total_bytes += line.len();
            excerpt_parts.push(line);
        }
        if stats.total_matches > max_representative {
            excerpt_parts.push(format!(
                "  ... and {} more matches",
                stats.total_matches - max_representative
            ));
        }
        let excerpt = Some(excerpt_parts.join("\n"));

        // Recall guidance
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<keyword> — search within this output"
        ));
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — browse the output"
        ));

        Projection {
            type_label: "search/grep output",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure: false,
        }
    }
}

// ============================================================================
// 3. Shell/test projector
// ============================================================================

/// Projector for shell command and test runner outputs (`Bash`, `shell_exec`, etc.).
///
/// Surfaces exit status, detects failure indicators (errors, panics, assertions),
/// provides a tail excerpt, and recall guidance for deeper investigation.
pub struct ShellTestProjector;

/// Extract failure-relevant lines from raw output.
///
/// Uses substring matching against a curated set of error/failure indicators.
/// This is intentionally a best-effort over-approximation: benign substrings
/// (e.g. "error" in "mirror", "fail" in "unfailing") may produce false
/// positives. False negatives are preferred over missing real failures,
/// so the indicator set is lenient. Deduplication by exact line content
/// keeps the output compact.
fn find_failure_blocks(raw: &str) -> Vec<String> {
    let error_indicators = [
        "error",
        "Error",
        "ERROR",
        "fail",
        "Fail",
        "FAIL",
        "panic",
        "PANIC",
        "assertion",
        "Assertion",
        "FAILED",
        "FATAL",
        "fatal",
        "abort",
        "SIGSEGV",
        "SIGABRT",
        "stack backtrace",
        "traceback",
        "Traceback",
        "unreachable",
        "unimplemented",
        "timed out",
        "Killed",
    ];

    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut total_bytes: usize = 0;

    for line in raw.lines() {
        // Stop collecting when either limit is hit
        if blocks.len() >= FAILURE_BLOCKS_MAX || total_bytes >= FAILURE_BLOCKS_MAX_BYTES {
            break;
        }

        let is_error = error_indicators
            .iter()
            .any(|indicator| line.contains(indicator));
        if is_error {
            let truncated = truncate_line(line, EXCERPT_MAX_LINE_BYTES);
            total_bytes += truncated.len();
            blocks.push(truncated);
            in_block = true;
        } else if in_block && line.starts_with(char::is_whitespace) {
            // Continuation of an error block (indented)
            let truncated = truncate_line(line, EXCERPT_MAX_LINE_BYTES);
            total_bytes += truncated.len();
            blocks.push(truncated);
        } else {
            in_block = false;
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    blocks.retain(|b| seen.insert(b.clone()));

    blocks
}

/// Extract the last N lines for a tail excerpt.
fn tail_excerpt(raw: &str, n: usize) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let len = lines.len();
    if len <= n {
        lines
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        let start = len - n;
        let tail: Vec<String> = lines[start..].iter().map(|l| format!("  {l}")).collect();
        format!("  ... ({} lines before)\n{}", start, tail.join("\n"))
    }
}

impl OutputProjector for ShellTestProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::ShellTest
    }

    fn type_label(&self) -> &'static str {
        "shell/test output"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        let status = if asset.success { "SUCCESS" } else { "FAILED" };
        summary_lines.push(format!("Tool: {} — exit status: {status}", asset.tool_name));
        summary_lines.push(format!(
            "Size: {} bytes, {} lines, ~{} tokens",
            asset.byte_count, asset.line_count, asset.estimated_tokens
        ));

        let failure_blocks = if !asset.success {
            find_failure_blocks(raw_output)
        } else {
            Vec::new()
        };

        let is_failure = !asset.success;
        if is_failure {
            if !failure_blocks.is_empty() {
                summary_lines.push(format!(
                    "Detected {} failure indicator(s)",
                    failure_blocks.len()
                ));
            } else {
                summary_lines.push("No specific failure indicators detected".to_string());
            }
        }

        // Excerpt: failure blocks (if any), then tail — byte-capped
        let mut excerpt_parts: Vec<String> = Vec::new();
        let mut excerpt_bytes: usize = 0;
        if !failure_blocks.is_empty() {
            excerpt_parts.push("Failure indicators:".to_string());
            for fb in &failure_blocks {
                let line = format!("  {}", truncate_line(fb, EXCERPT_MAX_LINE_BYTES));
                if excerpt_bytes + line.len() > EXCERPT_MAX_BYTES {
                    break;
                }
                excerpt_bytes += line.len();
                excerpt_parts.push(line);
            }
            excerpt_parts.push(String::new());
        }

        // Tail excerpt (remaining budget)
        let tail_lines = 15usize;
        let tail_header = format!("Last {tail_lines} lines of output:");
        excerpt_bytes += tail_header.len();
        excerpt_parts.push(tail_header);
        let tail_text = tail_excerpt(raw_output, tail_lines);
        // Truncate each tail line and cap total
        let mut tail_bytes: usize = 0;
        for tline in tail_text.lines() {
            let truncated = truncate_line(tline, EXCERPT_MAX_LINE_BYTES);
            if excerpt_bytes + tail_bytes + truncated.len() + 1 > EXCERPT_MAX_BYTES {
                excerpt_parts.push("  ... (truncated)".to_string());
                break;
            }
            tail_bytes += truncated.len() + 1;
            excerpt_parts.push(truncated);
        }

        let excerpt = if !raw_output.is_empty() {
            Some(excerpt_parts.join("\n"))
        } else {
            None
        };

        // Recall guidance
        recall_guidance.push(format!(
            "output_tail handle={handle} lines=50 — view more of the end of output"
        ));
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<error> — search for error patterns"
        ));
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — read arbitrary ranges"
        ));

        Projection {
            type_label: "shell/test output",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure,
        }
    }
}

// ============================================================================
// 4. Directory/tree projector
// ============================================================================

/// Projector for directory/tree listing outputs (`Glob`, `ls`, `list_dir`).
///
/// Summarises entry counts, shows representative entries, and provides
/// paging and search recall guidance for large listings.
pub struct DirectoryTreeProjector;

impl OutputProjector for DirectoryTreeProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::DirectoryTree
    }

    fn type_label(&self) -> &'static str {
        "directory listing"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        summary_lines.push(format!("Tool: {} (directory listing)", asset.tool_name));
        summary_lines.push(format!(
            "Size: {} bytes, {} lines",
            asset.byte_count, asset.line_count
        ));

        // Count entries and identify types
        let all_lines: Vec<&str> = raw_output.lines().collect();
        let non_empty: Vec<&&str> = all_lines.iter().filter(|l| !l.trim().is_empty()).collect();
        let total_entries = non_empty.len();

        summary_lines.push(format!("Total entries: {total_entries}"));

        // Representative entries (first 25, byte-capped)
        let max_representative = 25usize;
        let mut excerpt_parts: Vec<String> = Vec::new();
        let mut total_bytes: usize = 0;
        excerpt_parts.push(format!(
            "Representative entries (first {max_representative}):"
        ));
        for entry in non_empty.iter().take(max_representative) {
            let line = format!("  {}", truncate_line(entry, EXCERPT_MAX_LINE_BYTES));
            if total_bytes + line.len() > EXCERPT_MAX_BYTES {
                break;
            }
            total_bytes += line.len();
            excerpt_parts.push(line);
        }
        if total_entries > max_representative {
            excerpt_parts.push(format!(
                "... and {} more entries",
                total_entries - max_representative
            ));
        }

        let excerpt = if total_entries > 0 {
            Some(excerpt_parts.join("\n"))
        } else {
            None
        };

        // Recall guidance
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — browse entries by line range"
        ));
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<filename> — find specific entries"
        ));

        Projection {
            type_label: "directory listing",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure: false,
        }
    }
}

// ============================================================================
// 5. JSON/default (structured data) projector
// ============================================================================

/// Projector for JSON or structured data outputs (`mcp__*`, browser tools).
///
/// If valid JSON, analyses top-level shape (object/array), key names,
/// and array counts. Falls back to structure hints and a bounded excerpt
/// for non-JSON text.
pub struct JsonDefaultProjector;

/// Result of analyzing JSON/text structure shape.
#[derive(Default)]
struct JsonShape {
    is_object: bool,
    is_array: bool,
    top_keys: Vec<String>,
    /// Largest array length found among top-level values.
    array_count: usize,
    key_count: usize,
    is_valid_json: bool,
}

/// Analyze JSON/structure shape from raw output.
fn analyze_json_shape(raw: &str) -> JsonShape {
    let trimmed = raw.trim();
    let mut shape = JsonShape::default();

    // Try serde_json parse for full analysis
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        shape.is_valid_json = true;
        match value {
            serde_json::Value::Object(map) => {
                shape.is_object = true;
                shape.key_count = map.len();
                shape.top_keys = map.keys().take(20).cloned().collect();
                for (_key, val) in &map {
                    if let serde_json::Value::Array(arr) = val {
                        shape.is_array = true;
                        shape.array_count = shape.array_count.max(arr.len());
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                shape.is_array = true;
                shape.array_count = arr.len();
            }
            _ => {}
        }
    } else {
        // Not valid JSON — still provide structure hints.
        // These heuristics are intentionally rough: brace counting includes nested
        // objects and is only meant as a rough upper bound for the summary.
        if trimmed.starts_with('{') {
            shape.is_object = true;
            // Upper bound: every brace pair roughly corresponds to one key
            shape.key_count = trimmed.bytes().filter(|&b| b == b'{' || b == b'}').count() / 2;
        } else if trimmed.starts_with('[') {
            shape.is_array = true;
        }
    }

    shape
}

impl OutputProjector for JsonDefaultProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::JsonDefault
    }

    fn type_label(&self) -> &'static str {
        "JSON/structured output"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        summary_lines.push(format!("Tool: {} (structured output)", asset.tool_name));
        summary_lines.push(format!(
            "Size: {} bytes, {} lines, ~{} tokens",
            asset.byte_count, asset.line_count, asset.estimated_tokens
        ));

        // Analyze shape
        let shape = analyze_json_shape(raw_output);

        if shape.is_valid_json {
            if shape.is_object {
                summary_lines.push("Structure: JSON object".to_string());
            } else if shape.is_array {
                summary_lines.push("Structure: JSON array".to_string());
            }
        } else {
            summary_lines.push("Structure: text (not valid JSON)".to_string());
        }

        if shape.key_count > 0 {
            summary_lines.push(format!("Top-level keys: {}", shape.key_count));
            if !shape.top_keys.is_empty() {
                let keys_str = shape
                    .top_keys
                    .iter()
                    .map(|k| format!("  \"{k}\""))
                    .collect::<Vec<_>>()
                    .join("\n");
                summary_lines.push(format!("Key names:\n{keys_str}"));
            }
        }

        if shape.array_count > 0 {
            summary_lines.push(format!("Array items: {}", shape.array_count));
        }

        // Excerpt: trim to a bounded range (byte-capped)
        let all_lines: Vec<&str> = raw_output.lines().collect();
        let total_lines = all_lines.len();
        let excerpt = if !all_lines.is_empty() {
            let mut result: Vec<String> = Vec::new();
            let mut total_bytes: usize = 0;
            for l in all_lines.iter().take(30) {
                let formatted = format!("  {}", truncate_line(l, EXCERPT_MAX_LINE_BYTES));
                if total_bytes + formatted.len() > EXCERPT_MAX_BYTES {
                    break;
                }
                total_bytes += formatted.len();
                result.push(formatted);
            }
            let mut content = result.join("\n");
            if total_lines > 30 || result.len() < all_lines.len().min(30) {
                let omitted = total_lines - result.len();
                content.push_str(&format!("\n  ... ({} more lines)", omitted));
            }
            Some(content)
        } else {
            None
        };

        // Recall guidance
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — read the full output"
        ));
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<key> — search for specific keys/values"
        ));

        Projection {
            type_label: "JSON/structured output",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure: false,
        }
    }
}

// ============================================================================
// 6. Generic text projector (fallback for unknown large output)
// ============================================================================

/// Fallback projector for unknown or generic text outputs.
///
/// Produces a deterministic head/tail summary (10 lines each) with full
/// recall guidance. Used when no specialised projector matches the tool kind.
pub struct GenericTextProjector;

impl OutputProjector for GenericTextProjector {
    fn kind(&self) -> ProjectorKind {
        ProjectorKind::GenericText
    }

    fn type_label(&self) -> &'static str {
        "text output"
    }

    fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        let handle = asset.handle.as_str().to_string();
        let mut summary_lines = Vec::new();
        let mut recall_guidance = Vec::new();

        summary_lines.push(format!("Tool: {} (text output)", asset.tool_name));
        summary_lines.push(format!(
            "Size: {} bytes, {} lines, ~{} tokens",
            asset.byte_count, asset.line_count, asset.estimated_tokens
        ));

        // Deterministic head/tail summary (byte-capped)
        let lines: Vec<&str> = raw_output.lines().collect();
        let head_lines = 10usize;
        let tail_lines = 10usize;

        let mut excerpt_parts: Vec<String> = Vec::new();
        let mut total_bytes: usize = 0;

        if !lines.is_empty() {
            // Head
            excerpt_parts.push(format!("Head (first {head_lines} lines):"));
            for l in lines.iter().take(head_lines) {
                let line = format!("  {}", truncate_line(l, EXCERPT_MAX_LINE_BYTES));
                if total_bytes + line.len() > EXCERPT_MAX_BYTES {
                    break;
                }
                total_bytes += line.len();
                excerpt_parts.push(line);
            }

            // Tail (only if enough lines to justify separate tail and budget remains)
            if lines.len() > head_lines + tail_lines && total_bytes < EXCERPT_MAX_BYTES {
                excerpt_parts.push(format!(
                    "\n  ... ({} lines omitted)\n",
                    lines.len() - head_lines - tail_lines
                ));
                excerpt_parts.push(format!("Tail (last {tail_lines} lines):"));
                for l in lines.iter().skip(lines.len() - tail_lines) {
                    let line = format!("  {}", truncate_line(l, EXCERPT_MAX_LINE_BYTES));
                    if total_bytes + line.len() > EXCERPT_MAX_BYTES {
                        break;
                    }
                    total_bytes += line.len();
                    excerpt_parts.push(line);
                }
            } else if lines.len() > head_lines && total_bytes < EXCERPT_MAX_BYTES {
                for l in lines.iter().skip(head_lines) {
                    let line = format!("  {}", truncate_line(l, EXCERPT_MAX_LINE_BYTES));
                    if total_bytes + line.len() > EXCERPT_MAX_BYTES {
                        break;
                    }
                    total_bytes += line.len();
                    excerpt_parts.push(line);
                }
            }
        }

        let excerpt = if !excerpt_parts.is_empty() {
            Some(excerpt_parts.join("\n"))
        } else {
            None
        };

        // Recall guidance
        recall_guidance.push(format!(
            "output_read handle={handle} start_line=1 end_line=500 — read arbitrary line ranges"
        ));
        recall_guidance.push(format!(
            "output_search handle={handle} pattern=<keyword> — search within this output"
        ));
        recall_guidance.push(format!(
            "output_tail handle={handle} lines=50 — view the end of this output"
        ));

        Projection {
            type_label: "text output",
            summary_lines,
            excerpt,
            handle,
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance,
            is_failure: false,
        }
    }
}

// ============================================================================
// === Projector registry ===
// ============================================================================

/// Registry mapping [`ProjectorKind`] to its projector implementation.
pub struct ProjectorRegistry {
    projectors: Vec<Box<dyn OutputProjector>>,
}

impl ProjectorRegistry {
    /// Create a registry with all built-in projectors.
    pub fn new() -> Self {
        let projectors: Vec<Box<dyn OutputProjector>> = vec![
            Box::new(ReadFileProjector),
            Box::new(SearchProjector),
            Box::new(ShellTestProjector),
            Box::new(DirectoryTreeProjector),
            Box::new(JsonDefaultProjector),
            Box::new(GenericTextProjector),
        ];
        Self { projectors }
    }

    /// Get the projector for a given [`ProjectorKind`].
    ///
    /// Returns the matching projector, falling back to [`GenericTextProjector`]
    /// when no specific projector is registered (including for
    /// [`ProjectorKind::BrowserSnapshot`], which has no dedicated projector yet).
    /// A `tracing::warn!` is emitted on fallback so missing projectors are
    /// visible during development.
    pub fn get(&self, kind: ProjectorKind) -> &dyn OutputProjector {
        if let Some(p) = self.projectors.iter().find(|p| p.kind() == kind) {
            return p.as_ref();
        }
        tracing::warn!(
            ?kind,
            "No dedicated projector registered; falling back to GenericText"
        );
        // Generic text is the last entry (guaranteed by `new()`)
        self.projectors.last().unwrap().as_ref()
    }

    /// Produce a projection for an asset with the given raw output.
    pub fn project(&self, asset: &ToolOutputAsset, raw_output: &str) -> Projection {
        self.get(asset.projector_kind).project(asset, raw_output)
    }

    /// Produce a metadata-only projection (no raw content loaded).
    pub fn project_metadata_only(&self, asset: &ToolOutputAsset) -> Projection {
        self.get(asset.projector_kind).project_metadata_only(asset)
    }
}

impl Default for ProjectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global shared projector registry, lazily initialized once.
///
/// Avoids allocating 6 boxed projectors on every `output_summary` call.
pub static PROJECTOR_REGISTRY: std::sync::LazyLock<ProjectorRegistry> =
    std::sync::LazyLock::new(ProjectorRegistry::new);

// ============================================================================
// === Snapshot helpers ===
// ============================================================================

/// Format bytes as a human-readable string with stable formatting.
/// Always uses IEC-style units (KiB, MiB) with exactly 2 decimal places.
/// This function is deterministic: same input always produces the same output.
pub fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

// ============================================================================
// === Tests ===
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_output_store::{
        AssetLifecycle, OutputSizeClass, ProjectorKind, ToolOutputAsset, ToolOutputHandle,
    };

    /// Build a minimal test asset with the given projector kind and content.
    fn make_test_asset(
        kind: ProjectorKind,
        tool_name: &str,
        output: &str,
    ) -> (ToolOutputAsset, String) {
        let (byte_count, line_count) = crate::tool_output_store::count_bytes_and_lines(output);
        let content_hash = crate::tool_output_store::compute_content_hash(output);
        let estimated_tokens = crate::tool_output_store::estimate_tokens(byte_count);
        let size_class = OutputSizeClass::classify(
            byte_count,
            line_count,
            estimated_tokens,
            &crate::tool_output_store::ProjectionSizeConfig::default(),
        );

        let asset = ToolOutputAsset {
            handle: ToolOutputHandle::new("test_session"),
            session_id: "test_session".into(),
            turn_id: "turn_1".into(),
            tool_call_id: "call_1".into(),
            tool_name: tool_name.into(),
            arguments_digest: "abc123".into(),
            success: true,
            lifecycle: AssetLifecycle::Active,
            projector_kind: kind,
            byte_count,
            line_count,
            estimated_tokens,
            size_class,
            content_hash,
            blob_path: "/tmp/test.blob".into(),
            line_index_path: None,
            chunk_index_path: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            last_accessed_at: "2026-01-01T00:00:00Z".into(),
            expired_at: None,
        };

        (asset, output.to_string())
    }

    // =========================================================================
    // Format tests
    // =========================================================================

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(2048), "2.00 KiB");
        assert_eq!(format_bytes(1536), "1.50 KiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(2_097_152), "2.00 MiB");
    }

    #[test]
    fn format_bytes_gib() {
        assert_eq!(format_bytes(2_147_483_648), "2.00 GiB");
    }

    #[test]
    fn format_bytes_deterministic() {
        let a = format_bytes(12345);
        let b = format_bytes(12345);
        assert_eq!(a, b);
    }

    #[test]
    fn projection_format_is_deterministic() {
        let proj = Projection {
            type_label: "test",
            summary_lines: vec!["line 1".into(), "line 2".into()],
            excerpt: Some("excerpt content".into()),
            handle: "out_abc_123".into(),
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance: vec!["output_read handle=out_abc_123".into()],
            is_failure: false,
        };

        let fmt1 = proj.format();
        let fmt2 = proj.format();
        assert_eq!(fmt1, fmt2);
        assert!(!fmt1.contains("blob_path"));
        assert!(!fmt1.contains("timestamp"));
    }

    // =========================================================================
    // ReadFile projector snapshot tests
    // =========================================================================

    #[test]
    fn read_file_projector_basic() {
        let output = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let (asset, raw) = make_test_asset(ProjectorKind::ReadFile, "Read", output);
        let projector = ReadFileProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("file read output"));
        assert!(formatted.contains(&asset.handle.as_str().to_string()));
        assert!(formatted.contains("Read"));
        assert!(formatted.contains("line 1"));
        assert!(formatted.contains("output_read"));
        assert!(formatted.contains("output_search"));
        assert!(
            !formatted.contains("/tmp/test.blob"),
            "must not leak blob path"
        );
        assert!(
            !formatted.contains("2026-01-01"),
            "must not leak timestamps"
        );
    }

    #[test]
    fn read_file_projector_large_output_truncated_excerpt() {
        // Generate 100 lines — excerpt should be capped
        let mut output = String::new();
        for i in 0..100 {
            output.push_str(&format!("line_{:04}: some content here\n", i));
        }
        let (asset, raw) = make_test_asset(ProjectorKind::ReadFile, "read_file", &output);
        let projector = ReadFileProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("line_0000"));
        assert!(formatted.contains("more lines"));
        // Should have recall guidance for reading large output
        assert!(formatted.contains("output_read"));
    }

    // =========================================================================
    // Search projector snapshot tests
    // =========================================================================

    #[test]
    fn search_projector_grep_output() {
        let output = "\
src/main.rs:10:fn main() {
src/main.rs:42:    println!(\"hello\");
tests/test.rs:5:fn test_main() {
tests/test.rs:15:    assert_eq!(1 + 1, 2);
";
        let (asset, raw) = make_test_asset(ProjectorKind::Search, "Grep", output);
        let projector = SearchProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("search/grep output"));
        assert!(formatted.contains("Total matches: 4"));
        assert!(formatted.contains("output_search"));
        assert!(!formatted.contains("/tmp/test.blob"));
    }

    #[test]
    fn search_projector_empty_output() {
        let (asset, raw) = make_test_asset(ProjectorKind::Search, "rg", "");
        let projector = SearchProjector;
        let proj = projector.project(&asset, &raw);

        assert_eq!(
            proj.summary_lines
                .iter()
                .find(|l| l.contains("Total"))
                .unwrap(),
            "Total matches: 0"
        );
    }

    // =========================================================================
    // Shell/test projector snapshot tests
    // =========================================================================

    #[test]
    fn shell_test_projector_success() {
        let output = "Building...\nCompiling crate...\nFinished in 5.2s\n";
        let (mut asset, raw) = make_test_asset(ProjectorKind::ShellTest, "Bash", output);
        asset.success = true;

        let projector = ShellTestProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("shell/test output"));
        assert!(formatted.contains("SUCCESS"));
        assert!(!proj.is_failure);
        assert!(formatted.contains("output_tail"));
    }

    #[test]
    fn shell_test_projector_failure_with_errors() {
        let output = "\
   Compiling my_crate v0.1.0
error[E0308]: mismatched types
  --> src/main.rs:10:5
   |
10 |     let x: u32 = String::new();
   |            ---   ^^^^^^^^^^^^^ expected `u32`, found `String`
   |
error: could not compile `my_crate` due to 1 previous error
";
        let (mut asset, raw) = make_test_asset(ProjectorKind::ShellTest, "Bash", output);
        asset.success = false;

        let projector = ShellTestProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("FAILED"));
        assert!(proj.is_failure);
        assert!(formatted.contains("error"));
        assert!(formatted.contains("Failure indicators"));
        assert!(formatted.contains("output_search"));
    }

    #[test]
    fn shell_test_projector_panic_output() {
        let output = "\
running 10 tests
test test_a ... ok
test test_b ... FAILED
test test_c ... ok

failures:

---- test_b stdout ----
thread 'test_b' panicked at src/lib.rs:42:9:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
";
        let (mut asset, raw) = make_test_asset(ProjectorKind::ShellTest, "Bash", output);
        asset.success = false;

        let projector = ShellTestProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("FAILED"));
        assert!(proj.is_failure);
        assert!(formatted.contains("Failure indicators"));
        assert!(formatted.contains("FAILED") || formatted.contains("panicked"));
    }

    // =========================================================================
    // Directory tree projector snapshot tests
    // =========================================================================

    #[test]
    fn directory_tree_projector() {
        let output = "\
src/
src/main.rs
src/lib.rs
src/utils.rs
tests/
tests/test_main.rs
tests/test_utils.rs
README.md
Cargo.toml
Cargo.lock
.gitignore
";
        let (asset, raw) = make_test_asset(ProjectorKind::DirectoryTree, "Glob", output);
        let projector = DirectoryTreeProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("directory listing"));
        assert!(formatted.contains("Total entries:"));
        assert!(formatted.contains("src/main.rs"));
        assert!(formatted.contains("output_read"));
        assert!(formatted.contains("output_search"));
    }

    // =========================================================================
    // JSON/default projector snapshot tests
    // =========================================================================

    #[test]
    fn json_projector_object_output() {
        let output = r#"{"name": "test", "version": "1.0", "dependencies": {"serde": "1.0", "tokio": "1.0"}}"#;
        let (asset, raw) = make_test_asset(ProjectorKind::JsonDefault, "mcp__tool", output);
        let projector = JsonDefaultProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("JSON/structured output"));
        assert!(formatted.contains("Structure: JSON object"));
        assert!(formatted.contains("Key names"));
    }

    #[test]
    fn json_projector_array_output() {
        let output = r#"[{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}, {"id": 3, "name": "Charlie"}]"#;
        let (asset, raw) = make_test_asset(ProjectorKind::JsonDefault, "mcp__tool", output);
        let projector = JsonDefaultProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("JSON/structured output"));
        assert!(formatted.contains("Array items: 3"));
    }

    #[test]
    fn json_projector_non_json_fallback() {
        let output =
            "Just some regular text output\nwithout any JSON structure\nthat goes on for a while\n";
        let (asset, raw) = make_test_asset(ProjectorKind::JsonDefault, "mcp__tool", output);
        let projector = JsonDefaultProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("not valid JSON"));
        assert!(formatted.contains("output_read"));
    }

    // =========================================================================
    // Generic text projector snapshot tests
    // =========================================================================

    #[test]
    fn generic_text_projector_head_tail() {
        let mut output = String::new();
        for i in 0..50 {
            output.push_str(&format!("line_{:04}: content\n", i));
        }
        let (asset, raw) = make_test_asset(ProjectorKind::GenericText, "UnknownTool", &output);
        let projector = GenericTextProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("text output"));
        assert!(formatted.contains("Head"));
        assert!(formatted.contains("Tail"));
        assert!(formatted.contains("line_0000"));
        assert!(formatted.contains("line_0049"));
        assert!(formatted.contains("lines omitted"));
        assert!(formatted.contains("output_read"));
        assert!(formatted.contains("output_search"));
        assert!(formatted.contains("output_tail"));
    }

    #[test]
    fn generic_text_projector_short_output_no_tail_separate() {
        let output = "short\noutput\nhere\n";
        let (asset, raw) = make_test_asset(ProjectorKind::GenericText, "Tool", output);
        let projector = GenericTextProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("Head"));
        // Short output should NOT have a separate Tail section
        assert!(!formatted.contains("Tail (last"));
    }

    #[test]
    fn generic_text_projector_empty_output() {
        let (asset, raw) = make_test_asset(ProjectorKind::GenericText, "Tool", "");
        let projector = GenericTextProjector;
        let proj = projector.project(&asset, &raw);
        let formatted = proj.format();

        assert!(formatted.contains("text output"));
        assert!(!formatted.contains("Head"));
        assert!(formatted.contains("output_read"));
    }

    // =========================================================================
    // Projection no volatile data tests
    // =========================================================================

    #[test]
    fn all_projectors_exclude_blob_paths() {
        let output = "test output\n";
        let kinds = vec![
            ProjectorKind::ReadFile,
            ProjectorKind::Search,
            ProjectorKind::ShellTest,
            ProjectorKind::DirectoryTree,
            ProjectorKind::JsonDefault,
            ProjectorKind::GenericText,
        ];
        let tool_names = vec!["Read", "Grep", "Bash", "Glob", "mcp__tool", "Tool"];

        let registry = ProjectorRegistry::new();

        for (&kind, tool_name) in kinds.iter().zip(tool_names.iter()) {
            let (asset, raw) = make_test_asset(kind, tool_name, output);
            let proj = registry.project(&asset, &raw);
            let formatted = proj.format();

            assert!(
                !formatted.contains("/tmp/"),
                "{kind:?} projector leaked filesystem path"
            );
            assert!(
                !formatted.contains("2026-01-01"),
                "{kind:?} projector leaked timestamp"
            );
        }
    }

    #[test]
    fn all_projectors_exclude_volatile_timestamps() {
        let output = "test output\n";
        let kinds = [
            ProjectorKind::ReadFile,
            ProjectorKind::Search,
            ProjectorKind::ShellTest,
            ProjectorKind::DirectoryTree,
            ProjectorKind::JsonDefault,
            ProjectorKind::GenericText,
        ];
        let tool_names = ["Read", "Grep", "Bash", "Glob", "mcp__tool", "Tool"];

        let registry = ProjectorRegistry::new();

        for (&kind, tool_name) in kinds.iter().zip(tool_names.iter()) {
            let (asset, raw) = make_test_asset(kind, tool_name, output);
            let proj = registry.project(&asset, &raw);
            let formatted = proj.format();

            // Verify no raw timestamp strings appear
            assert!(
                !formatted.contains("created_at"),
                "{kind:?} leaked created_at"
            );
            assert!(
                !formatted.contains("last_accessed_at"),
                "{kind:?} leaked last_accessed_at"
            );
            assert!(
                !formatted.contains("T00:00:00"),
                "{kind:?} leaked ISO timestamp"
            );
        }
    }

    // =========================================================================
    // ProjectorRegistry tests
    // =========================================================================

    #[test]
    fn registry_returns_correct_projector_for_each_kind() {
        let registry = ProjectorRegistry::new();

        assert_eq!(
            registry.get(ProjectorKind::ReadFile).kind(),
            ProjectorKind::ReadFile
        );
        assert_eq!(
            registry.get(ProjectorKind::Search).kind(),
            ProjectorKind::Search
        );
        assert_eq!(
            registry.get(ProjectorKind::ShellTest).kind(),
            ProjectorKind::ShellTest
        );
        assert_eq!(
            registry.get(ProjectorKind::DirectoryTree).kind(),
            ProjectorKind::DirectoryTree
        );
        assert_eq!(
            registry.get(ProjectorKind::JsonDefault).kind(),
            ProjectorKind::JsonDefault
        );
        assert_eq!(
            registry.get(ProjectorKind::GenericText).kind(),
            ProjectorKind::GenericText
        );
        // BrowserSnapshot falls back to GenericText
        assert_eq!(
            registry.get(ProjectorKind::BrowserSnapshot).kind(),
            ProjectorKind::GenericText
        );
    }

    #[test]
    fn metadata_only_projection_still_has_handle_and_guidance() {
        let (asset, _) = make_test_asset(ProjectorKind::GenericText, "Tool", "large output");
        let registry = ProjectorRegistry::new();
        let proj = registry.project_metadata_only(&asset);
        let formatted = proj.format();

        assert!(formatted.contains(&asset.handle.as_str().to_string()));
        assert!(formatted.contains("output_read"));
    }

    #[test]
    fn projection_estimated_tokens_is_stable() {
        let proj = Projection {
            type_label: "test",
            summary_lines: vec!["a".into()],
            excerpt: Some("b".into()),
            handle: "out_abc_123".into(),
            provenance: ProjectionProvenance::AssetManifest,
            recall_guidance: vec!["c".into()],
            is_failure: false,
        };
        let t1 = proj.estimated_tokens();
        let t2 = proj.estimated_tokens();
        assert_eq!(t1, t2);
    }

    #[test]
    fn truncate_line_cjk_and_emoji_safe() {
        fn repeat_char(ch: char, count: usize) -> String {
            std::iter::repeat(ch).take(count).collect()
        }

        let cjk_line = repeat_char('\u{4f60}', 250);
        let result = truncate_line(&cjk_line, 500);
        assert!(result.len() <= 500);
        assert!(result.ends_with("..."));
        assert!(!result.contains('\u{fffd}'));

        let emoji_line = repeat_char('\u{1f389}', 200);
        let result2 = truncate_line(&emoji_line, 500);
        assert!(result2.len() <= 500);
        assert!(result2.ends_with("..."));
    }
}
