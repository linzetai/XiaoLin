//! Context Projection Pipeline — the single owner of model-visible output
//! projection under token budget.
//!
//! # Design
//!
//! The pipeline centralizes decisions about how tool outputs are represented
//! in LLM context. It replaces the scattered logic that previously lived in
//! post-tool processing, ContentFilterHook, budget allocation, and hard
//! context fitting.
//!
//! ## Policy (adaptive projection)
//!
//! | Size class | Relevance | Strategy |
//! |-----------|-----------|----------|
//! | Small     | any       | Keep inline (no projection needed) |
//! | Medium    | high      | Keep key content (excerpt) |
//! | Medium    | low       | Typed summary + handle |
//! | Large     | high      | Typed summary + excerpt + handle |
//! | Large     | low       | Handle-only manifest (last resort) |
//!
//! ## Provenance tracking
//!
//! Every projection decision records provenance so downstream layers
//! (ContentFilterHook, hard fit) can recognize already-projected content
//! and avoid re-truncation.

use std::collections::HashSet;

use xiaolin_core::types::{ChatMessage, Role};
use xiaolin_session::tool_output_projector::Projection;
use xiaolin_session::tool_output_store::{OutputSizeClass, ProjectionSizeConfig};

// ============================================================================
// Projection budget accounting
// ============================================================================

/// Accumulated projection budget metrics for a single context assembly pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectionBudget {
    /// Total raw bytes of tool outputs before projection.
    pub raw_bytes: usize,
    /// Estimated raw tokens (bytes / 4) before projection.
    pub raw_tokens_estimate: usize,
    /// Total bytes of projected output after projection.
    pub projected_bytes: usize,
    /// Estimated projected tokens after projection.
    pub projected_tokens_estimate: usize,
    /// Estimated tokens saved by projection.
    pub tokens_saved: usize,
    /// Number of tool outputs that were projected.
    pub projected_count: usize,
    /// Number of tool outputs left inline (small enough).
    pub inline_count: usize,
}

impl ProjectionBudget {
    /// Record a projection decision.
    pub fn record_projection(&mut self, raw_bytes: usize, projected: &Projection) {
        self.raw_bytes += raw_bytes;
        self.raw_tokens_estimate += raw_bytes / 4;
        let proj_bytes = projected.format().len();
        self.projected_bytes += proj_bytes;
        self.projected_tokens_estimate += projected.estimated_tokens();
        self.tokens_saved += (raw_bytes / 4).saturating_sub(projected.estimated_tokens());
        self.projected_count += 1;
    }

    /// Record an inline decision (small output kept as-is).
    pub fn record_inline(&mut self, raw_bytes: usize) {
        self.raw_bytes += raw_bytes;
        self.raw_tokens_estimate += raw_bytes / 4;
        self.inline_count += 1;
    }
}

// ============================================================================
// Relevance classification
// ============================================================================

/// Relevance classification for a tool output.
///
/// "High" means the output is likely needed for the current task (recent,
/// referenced by assistant, contains errors, etc.). "Low" means it can be
/// summarized aggressively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputRelevance {
    High,
    Low,
}

// ============================================================================
// Pipeline
// ============================================================================

/// Context Projection Pipeline — the single owner of model-visible output
/// projection under token budget.
///
/// Called from the unified pre-query compact pipeline after microcompact
/// and budget allocation, before ContentFilterHook and hard context fitting.
/// This ensures tool outputs are projected exactly once, and downstream
/// layers recognize already-projected content.
pub(crate) struct ContextProjectionPipeline {
    /// Size classification thresholds.
    size_config: ProjectionSizeConfig,
    /// Accumulated budget metrics from the last projection pass.
    budget: ProjectionBudget,
    /// Handles that were projected in the current pass (for dedup).
    projected_handles: HashSet<String>,
    /// Maximum token budget for projected tool outputs in the current pass.
    /// When exceeded, remaining outputs get handle-only manifests.
    projection_token_budget: Option<usize>,
    /// Tokens consumed by projections so far in the current pass.
    projection_tokens_used: usize,
}

impl ContextProjectionPipeline {
    /// Create a new pipeline with default size configuration.
    pub fn new() -> Self {
        Self {
            size_config: ProjectionSizeConfig::default(),
            budget: ProjectionBudget::default(),
            projected_handles: HashSet::new(),
            projection_token_budget: None,
            projection_tokens_used: 0,
        }
    }

    /// Set the maximum token budget for all projections in this pass.
    /// When the budget is exhausted, remaining outputs get handle-only manifests.
    pub fn set_projection_budget(&mut self, max_tokens: usize) {
        self.projection_token_budget = Some(max_tokens);
    }

    /// Return the accumulated budget metrics from the last `project_messages` call.
    #[allow(dead_code)]
    pub fn budget(&self) -> &ProjectionBudget {
        &self.budget
    }

    /// Clear accumulated state for a new pass.
    pub fn reset(&mut self) {
        self.budget = ProjectionBudget::default();
        self.projected_handles.clear();
        self.projection_tokens_used = 0;
    }

    /// Project all tool output messages in `messages` that have asset handles.
    ///
    /// This iterates through messages, identifies tool outputs that are large
    /// enough to project (Medium or Large), and replaces them with appropriate
    /// projections based on size class, relevance, and budget.
    pub fn project_messages(&mut self, messages: &mut [ChatMessage]) {
        // Collect assistant text content for relevance scoring before the
        // mutable iteration below. This avoids borrowing `messages` immutably
        // and mutably at the same time.
        let assistant_texts: Vec<String> = messages
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .filter_map(|m| m.text_content().map(|c| c.into_owned()))
            .collect();
        let total_msgs = messages.len();

        for (idx, msg) in messages.iter_mut().enumerate() {
            if !matches!(msg.role, Role::Tool) {
                continue;
            }

            let text = match msg.text_content() {
                Some(t) => t,
                None => continue,
            };

            // Skip messages that are already projections or compacted.
            if is_already_projected(&text) {
                continue;
            }

            // Try to extract handle from the new projection format or legacy format.
            let handle = extract_handle_from_message(&text);

            if let Some(ref handle) = handle {
                if self.projected_handles.contains(handle) {
                    continue;
                }
                self.projected_handles.insert(handle.clone());
            }

            let raw_bytes = text.len();
            let raw_lines = text.lines().count();
            let raw_tokens = raw_bytes / 4;

            let size_class =
                OutputSizeClass::classify(raw_bytes, raw_lines, raw_tokens, &self.size_config);

            match size_class {
                OutputSizeClass::Small => {
                    // Small outputs stay inline.
                    self.budget.record_inline(raw_bytes);
                    continue;
                }
                OutputSizeClass::Medium => {
                    let relevance = classify_relevance(
                        idx,
                        total_msgs,
                        msg.name.as_deref().unwrap_or("unknown"),
                        &text,
                        &assistant_texts,
                    );

                    if relevance == OutputRelevance::High {
                        // Medium + high relevance: keep inline.
                        self.budget.record_inline(raw_bytes);
                        continue;
                    }

                    // Medium + low relevance: typed summary + handle.
                    let projection = build_metadata_projection(
                        msg.name.as_deref().unwrap_or("unknown"),
                        handle.as_deref().unwrap_or("unknown"),
                        &text,
                        raw_bytes,
                        raw_lines,
                    );
                    let proj_tokens = projection.estimated_tokens();
                    if self.try_consume_budget(proj_tokens) {
                        self.budget.record_projection(raw_bytes, &projection);
                        msg.content = Some(serde_json::Value::String(projection.format()));
                    } else {
                        let h = handle.as_deref().unwrap_or("unknown");
                        let content =
                            build_handle_only(msg.name.as_deref().unwrap_or("unknown"), h);
                        msg.content = Some(serde_json::Value::String(content));
                        self.budget.record_inline(raw_bytes);
                    }
                }
                OutputSizeClass::Large => {
                    let relevance = classify_relevance(
                        idx,
                        total_msgs,
                        msg.name.as_deref().unwrap_or("unknown"),
                        &text,
                        &assistant_texts,
                    );

                    let projection = if relevance == OutputRelevance::High {
                        // Large + high: typed summary with excerpt.
                        build_metadata_projection(
                            msg.name.as_deref().unwrap_or("unknown"),
                            handle.as_deref().unwrap_or("unknown"),
                            &text,
                            raw_bytes,
                            raw_lines,
                        )
                    } else {
                        // Large + low: minimal projection (no excerpt).
                        build_minimal_projection(
                            msg.name.as_deref().unwrap_or("unknown"),
                            handle.as_deref().unwrap_or("unknown"),
                            raw_bytes,
                            raw_lines,
                        )
                    };

                    let proj_tokens = projection.estimated_tokens();
                    if self.try_consume_budget(proj_tokens) {
                        self.budget.record_projection(raw_bytes, &projection);
                        msg.content = Some(serde_json::Value::String(projection.format()));
                    } else {
                        let h = handle.as_deref().unwrap_or("unknown");
                        let content =
                            build_handle_only(msg.name.as_deref().unwrap_or("unknown"), h);
                        msg.content = Some(serde_json::Value::String(content));
                        self.budget.record_inline(raw_bytes);
                    }
                }
            }
        }
    }

    /// Try to consume `tokens` from the projection budget.
    /// Returns true if there's enough budget remaining.
    fn try_consume_budget(&mut self, tokens: usize) -> bool {
        if let Some(max) = self.projection_token_budget {
            if self.projection_tokens_used + tokens > max {
                return false;
            }
        }
        self.projection_tokens_used += tokens;
        true
    }
}

impl Default for ContextProjectionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Public helpers (used by xiaolin-context ContentFilterHook)
// ============================================================================

/// Check if a message content is already a projection or compaction result.
/// These should NOT be re-truncated by downstream layers like ContentFilterHook.
///
/// This function is intentionally duplicated in `xiaolin-context/src/engine.rs`
/// as `ContentFilterHook::is_already_compressed` for historical reasons.
/// New callers should use this centralized version.
pub fn is_already_projected(text: &str) -> bool {
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

    // New projection format markers: "[<type> — handle: out_...]\n"
    // All projector type labels from tool_output_projector.rs
    if text.starts_with("[shell/test output — handle:")
        || text.starts_with("[file read output — handle:")
        || text.starts_with("[search/grep output — handle:")
        || text.starts_with("[directory listing — handle:")
        || text.starts_with("[browser snapshot — handle:")
        || text.starts_with("[JSON/structured output — handle:")
        || text.starts_with("[text output — handle:")
        || text.starts_with("[output_summary:")
        || text.starts_with("[output stored — handle:")
    {
        return true;
    }

    false
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Extract an output handle from message text.
///
/// Handles three formats:
/// 1. Legacy XML: `<output-handle>out_abc_def</output-handle>`
/// 2. New projection: `[… — handle: out_abc_def]\n`
/// 3. Handle-only fallback: `[output stored — handle: out_abc_def]\n`
fn extract_handle_from_message(text: &str) -> Option<String> {
    // Format 1: Legacy XML
    let start_tag = "<output-handle>";
    let end_tag = "</output-handle>";
    if let Some(start) = text.find(start_tag) {
        let content_start = start + start_tag.len();
        if let Some(end) = text[content_start..].find(end_tag) {
            return Some(text[content_start..content_start + end].to_string());
        }
    }

    // Format 2: New projection format "[… — handle: out_abc]\n"
    let prefix = " — handle: ";
    if let Some(handle_start) = text.find(prefix) {
        let after_prefix = &text[handle_start + prefix.len()..];
        if let Some(handle_end) = after_prefix.find(']') {
            let handle = &after_prefix[..handle_end];
            if handle.starts_with("out_") {
                return Some(handle.to_string());
            }
        }
    }

    None
}

/// Classify the relevance of a tool output based on position, content, and
/// whether it's referenced by assistant messages.
fn classify_relevance(
    msg_index: usize,
    total_messages: usize,
    tool_name: &str,
    content: &str,
    assistant_texts: &[String],
) -> OutputRelevance {
    // Recent messages (last 20% of conversation) are always relevant.
    if total_messages > 0 {
        let position_ratio = msg_index as f32 / total_messages as f32;
        if position_ratio > 0.80 {
            return OutputRelevance::High;
        }
    }

    // Error/failure outputs are always relevant.
    if content.contains("error")
        || content.contains("Error")
        || content.contains("FAILED")
        || content.contains("panic")
        || content.contains("assertion failed")
        || content.contains("exit code")
    {
        return OutputRelevance::High;
    }

    // File reads and searches that were referenced by assistant are relevant.
    let is_referenced = assistant_texts.iter().any(|asst_text| {
        content
            .lines()
            .take(5)
            .filter(|l| l.len() > 15)
            .take(3)
            .any(|line| {
                let check_end = line.floor_char_boundary(std::cmp::min(line.len(), 40));
                asst_text.contains(&line[..check_end])
            })
    });

    if is_referenced {
        return OutputRelevance::High;
    }

    // File reads and shell commands for key tools are relevant by default.
    match tool_name {
        "read_file" | "Read" | "Bash" | "shell" | "shell_exec" | "Glob" | "Grep" => {
            OutputRelevance::High
        }
        _ => OutputRelevance::Low,
    }
}

/// Build a metadata projection with excerpt for a tool output.
fn build_metadata_projection(
    tool_name: &str,
    handle: &str,
    content: &str,
    raw_bytes: usize,
    raw_lines: usize,
) -> Projection {
    let type_label = classify_type_label(tool_name);
    let mut summary_lines = vec![
        format!("Tool: {tool_name}"),
        format!(
            "Size: {raw_bytes} bytes, {raw_lines} lines, ~{} tokens",
            raw_bytes / 4
        ),
    ];

    // Check for failure indicators
    let is_failure = content.contains("error")
        || content.contains("Error")
        || content.contains("FAILED")
        || content.contains("exit code: 1")
        || content.contains("exit code: 2");

    if is_failure {
        summary_lines.push("Status: FAILED".to_string());
    }

    // Build a small tail excerpt for context
    let tail_lines: Vec<&str> = content.lines().rev().take(5).collect();
    let excerpt = if !tail_lines.is_empty() {
        let tail_text: String = tail_lines
            .into_iter()
            .rev()
            .map(|l| {
                if l.len() > 500 {
                    format!("{}...", &l[..497])
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Some(tail_text)
    } else {
        None
    };

    Projection {
        type_label,
        summary_lines,
        excerpt,
        handle: handle.to_string(),
        recall_guidance: vec![
            format!("output_read handle={handle} start_line=1 end_line=500"),
            format!("output_search handle={handle} pattern=<keyword>"),
            format!("output_tail handle={handle} lines=50"),
        ],
        is_failure,
    }
}

/// Build a minimal projection (no excerpt, just size + handle).
fn build_minimal_projection(
    tool_name: &str,
    handle: &str,
    raw_bytes: usize,
    raw_lines: usize,
) -> Projection {
    let type_label = classify_type_label(tool_name);
    Projection {
        type_label,
        summary_lines: vec![
            format!("Tool: {tool_name}"),
            format!(
                "Size: {raw_bytes} bytes, {raw_lines} lines, ~{} tokens",
                raw_bytes / 4
            ),
        ],
        excerpt: None,
        handle: handle.to_string(),
        recall_guidance: vec![
            format!("output_read handle={handle} start_line=1 end_line=500"),
            format!("output_summary handle={handle} — get typed summary"),
        ],
        is_failure: false,
    }
}

/// Build a handle-only fallback message (last resort when budget exhausted).
/// Includes the handle so the model can recover content via recall tools.
fn build_handle_only(tool_name: &str, handle: &str) -> String {
    let label = classify_type_label(tool_name);
    format!(
        "[output stored — handle: {handle}]\n\
         Type: {label}\n\
         Use output_read, output_search, or output_summary to recall content.\n"
    )
}

/// Map a tool name to a human-readable type label.
fn classify_type_label(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "shell" | "shell_exec" | "run_command" | "exec" => "shell/test output",
        "Read" | "read_file" => "file read output",
        "Grep" | "search" | "rg" => "search/grep output",
        "Glob" | "ls" | "list_dir" | "list_directory" => "directory listing",
        "mcp__browser" | "browser" | "TakeSnapshot" => "browser snapshot",
        _ if tool_name.starts_with("mcp__") => "JSON/structured output",
        _ => "text output",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_msg(name: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Tool,
            content: Some(serde_json::Value::String(text.to_string())),
            name: Some(name.to_string()),
            tool_call_id: Some(format!("call_{name}")),
            ..Default::default()
        }
    }

    fn asst_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn test_is_already_projected_detects_compaction_markers() {
        assert!(is_already_projected("[faded] some content"));
        assert!(is_already_projected("[time-compacted] old"));
        assert!(is_already_projected("[summarized] summary here"));
        assert!(is_already_projected("[oneliner] brief"));
        assert!(is_already_projected("[recall-available] xyz"));
        assert!(is_already_projected("[superseded"));
        assert!(is_already_projected("[Old tool result content cleared]"));
    }

    #[test]
    fn test_is_already_projected_detects_projection_format() {
        assert!(is_already_projected(
            "[shell/test output — handle: out_abc123]\n- Tool: Bash\n"
        ));
        assert!(is_already_projected(
            "[file read output — handle: out_xyz]\n- Tool: Read\n"
        ));
        assert!(is_already_projected(
            "[text output — handle: out_foo]\n- Tool: unknown\n"
        ));
        assert!(is_already_projected("[output stored — handle: out_bar]"));
        assert!(is_already_projected("[output_summary: out_baz]"));
    }

    #[test]
    fn test_is_already_projected_permits_raw_output() {
        assert!(!is_already_projected("regular output content"));
        assert!(!is_already_projected("error: something went wrong"));
    }

    #[test]
    fn test_extract_handle_from_legacy_format() {
        let text = "Result: <output-handle>out_abc_def</output-handle>\nPreview: ...";
        assert_eq!(
            extract_handle_from_message(text),
            Some("out_abc_def".to_string())
        );
    }

    #[test]
    fn test_extract_handle_from_projection_format() {
        let text = "[shell/test output — handle: out_sess123_456]\n- Tool: Bash\n- Size: 5 bytes\n";
        assert_eq!(
            extract_handle_from_message(text),
            Some("out_sess123_456".to_string())
        );
    }

    #[test]
    fn test_extract_handle_from_handle_only_format() {
        let text = "[output stored — handle: out_handle_only_999]\nUse output_read...\n";
        assert_eq!(
            extract_handle_from_message(text),
            Some("out_handle_only_999".to_string())
        );
    }

    #[test]
    fn test_extract_handle_no_handle_returns_none() {
        let text = "plain text output with no handle";
        assert_eq!(extract_handle_from_message(text), None);
    }

    #[test]
    fn test_classify_relevance_errors_are_high() {
        assert_eq!(
            classify_relevance(0, 10, "Bash", "error: compilation failed", &[]),
            OutputRelevance::High
        );
        assert_eq!(
            classify_relevance(0, 10, "unknown", "FAILED: test_foo", &[]),
            OutputRelevance::High
        );
    }

    #[test]
    fn test_classify_relevance_recent_messages_high() {
        // index 9/10 = 0.9 > 0.80 threshold
        assert_eq!(
            classify_relevance(9, 10, "unknown", "some output", &[]),
            OutputRelevance::High
        );
    }

    #[test]
    fn test_classify_relevance_old_unknown_low() {
        assert_eq!(
            classify_relevance(0, 10, "web_search", "some results", &[]),
            OutputRelevance::Low
        );
    }

    #[test]
    fn test_small_output_stays_inline() {
        let mut pipeline = ContextProjectionPipeline::new();
        let small = "short output".to_string();
        let mut msgs = vec![tool_msg("Bash", &small)];

        pipeline.project_messages(&mut msgs);

        // Content should be unchanged (inline).
        assert_eq!(msgs[0].text_content().unwrap(), small);
        assert_eq!(pipeline.budget().inline_count, 1);
        assert_eq!(pipeline.budget().projected_count, 0);
    }

    #[test]
    fn test_medium_low_relevance_gets_projected() {
        let mut pipeline = ContextProjectionPipeline::new();
        // 300 lines → Medium (> 200, ≤ 1000)
        let medium = "line\n".repeat(300); // 300 lines
        let mut msgs = vec![tool_msg("web_search", &medium)];

        // 'web_search' is Low relevance, Medium size → should be projected to summary
        // Note: no handle in content → handle will be "unknown"
        pipeline.project_messages(&mut msgs);

        let result = msgs[0].text_content().unwrap();
        assert!(
            result.contains("text output — handle:") || result.contains("[output stored —"),
            "expected projection, got: {result}"
        );
    }

    #[test]
    fn test_budget_exhaustion_falls_back_to_handle_only() {
        let mut pipeline = ContextProjectionPipeline::new();
        pipeline.set_projection_budget(10); // Very tight budget
        let large = "x".repeat(100000); // Large
        let mut msgs = vec![tool_msg("web_search", &large)];

        pipeline.project_messages(&mut msgs);

        let result = msgs[0].text_content().unwrap();
        assert!(
            result.contains("[output stored — handle:"),
            "expected handle-only fallback, got: {result}"
        );
    }

    #[test]
    fn test_already_projected_skipped() {
        let mut pipeline = ContextProjectionPipeline::new();
        let already = "[faded] already compacted".to_string();
        let mut msgs = vec![tool_msg("Bash", &already)];

        pipeline.project_messages(&mut msgs);

        assert_eq!(msgs[0].text_content().unwrap(), already);
        assert_eq!(pipeline.budget().inline_count, 0); // Not counted (skipped)
    }

    #[test]
    fn test_recent_high_relevance_kept_inline() {
        let mut pipeline = ContextProjectionPipeline::new();
        // Medium size, but recent (high relevance) → kept inline
        let medium = "line\n".repeat(500);
        let mut msgs = vec![
            asst_msg("ok"),
            tool_msg("Bash", &medium), // 'Bash' is High relevance
        ];

        pipeline.project_messages(&mut msgs);

        // 'Bash' is High relevance → even Medium stays inline
        let idx = msgs.iter().position(|m| m.role == Role::Tool).unwrap();
        let result = msgs[idx].text_content().unwrap();
        // Should be unchanged since 'Bash' is High relevance
        assert_eq!(result, medium);
    }

    #[test]
    fn test_nested_truncation_prevention() {
        // A message goes through projection, then is_already_projected must
        // return true so ContentFilterHook skips it (preventing nested
        // truncation markers).
        let mut pipeline = ContextProjectionPipeline::new();
        let large = "x".repeat(100000); // Large
        let mut msgs = vec![tool_msg("web_search", &large)];

        pipeline.project_messages(&mut msgs);

        let after_projection = msgs[0].text_content().unwrap();
        assert!(
            is_already_projected(&after_projection),
            "projected content should be recognized: {after_projection}"
        );
    }

    #[test]
    fn test_budget_exhaustion_output_recognized_as_projected() {
        // When the budget is exhausted and handle-only fallback is used,
        // is_already_projected must still return true to prevent
        // ContentFilterHook from re-truncating it.
        let mut pipeline = ContextProjectionPipeline::new();
        pipeline.set_projection_budget(10); // Very tight budget
        let large = "x".repeat(100000);
        let mut msgs = vec![tool_msg("web_search", &large)];

        pipeline.project_messages(&mut msgs);

        let result = msgs[0].text_content().unwrap();
        assert!(
            result.contains("[output stored — handle:"),
            "expected handle-only fallback, got: {result}"
        );
        assert!(
            is_already_projected(&result),
            "handle-only fallback must be recognized as already-projected: {result}"
        );
    }

    #[test]
    fn test_projection_budget_accounting() {
        let mut budget = ProjectionBudget::default();
        let proj = build_metadata_projection("Bash", "out_test", "some error output\n", 1000, 20);
        budget.record_projection(1000, &proj);
        budget.record_inline(500);

        assert_eq!(budget.raw_bytes, 1500);
        assert_eq!(budget.projected_count, 1);
        assert_eq!(budget.inline_count, 1);
    }

    /// Asserts that every projection format marker recognized by
    /// `context_projection::is_already_projected` is also recognized by
    /// `tool_executor::is_already_compacted` — preventing post-tool
    /// microcompact from destroying projected content (Phase 5.3).
    #[test]
    fn test_projection_markers_block_post_tool_compaction() {
        // Projection markers from the context projection pipeline.
        let projection_examples = [
            "[shell/test output — handle: out_test123]\n- Tool: Bash\n- Size: 5000 bytes",
            "[file read output — handle: out_test456]\n- Tool: Read\n- Size: 2000 bytes",
            "[search/grep output — handle: out_test789]\n- matches: 42",
            "[directory listing — handle: out_test000]\n- entries: 15",
            "[browser snapshot — handle: out_test111]\n- url: https://example.com",
            "[JSON/structured output — handle: out_test222]\n- shape: object, 3 keys",
            "[text output — handle: out_test333]\n- Tool: unknown\n- Size: 10000 bytes",
            "[output_summary: out_test444]\nType: shell/test output",
            "[output stored — handle: out_test555]\nType: shell/test output\nUse output_read...",
        ];

        for example in projection_examples {
            assert!(
                super::is_already_projected(example),
                "context_projection::is_already_projected must recognize: {example}"
            );
            assert!(
                crate::runtime::tool_executor::is_already_compacted(example),
                "tool_executor::is_already_compacted must recognize to prevent post-tool microcompact: {example}"
            );
        }
    }
}
