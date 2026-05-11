use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;

pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;

#[allow(dead_code)]
pub const BYTES_PER_TOKEN: usize = 4;

pub const PREVIEW_SIZE_BYTES: usize = 2000;

pub const PERSISTED_OUTPUT_TAG: &str = "<persisted-output>";
pub const PERSISTED_OUTPUT_CLOSING_TAG: &str = "</persisted-output>";

pub const TOOL_RESULT_CLEARED_MESSAGE: &str = "[Old tool result content cleared]";

const TOOL_RESULTS_SUBDIR: &str = "tool-results";

/// Resolve the effective persistence threshold for a tool.
///
/// - `usize::MAX` = hard opt-out (reserved for tools where persistence
///   would cause issues; currently no built-in tool uses this).
/// - Otherwise: `min(declared, DEFAULT_MAX_RESULT_SIZE_CHARS)`.
pub fn get_persistence_threshold(declared_max_result_size_chars: usize) -> usize {
    if declared_max_result_size_chars == usize::MAX {
        return usize::MAX;
    }
    declared_max_result_size_chars.min(DEFAULT_MAX_RESULT_SIZE_CHARS)
}

pub struct PersistedToolResult {
    pub filepath: PathBuf,
    pub original_size: usize,
    pub preview: String,
    pub has_more: bool,
}

pub struct ToolResultStorage {
    session_dir: PathBuf,
}

impl ToolResultStorage {
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    fn tool_results_dir(&self) -> PathBuf {
        self.session_dir.join(TOOL_RESULTS_SUBDIR)
    }

    fn tool_result_path(&self, tool_use_id: &str) -> PathBuf {
        self.tool_results_dir().join(format!("{tool_use_id}.txt"))
    }

    /// Process a tool result: persist large results to disk, return empty-result
    /// markers, and pass through small results unchanged.
    ///
    /// Returns `Ok(None)` if the content is small enough and should be used as-is.
    /// Returns `Ok(Some(replacement))` with the persisted-output XML message.
    /// Returns `Err` only on unexpected I/O failures (caller should fallback to
    /// using original content).
    pub fn process_result(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        content: &str,
        persistence_threshold: usize,
    ) -> Result<Option<String>, String> {
        if content.trim().is_empty() {
            return Ok(Some(format!("({tool_name} completed with no output)")));
        }

        if content.starts_with(PERSISTED_OUTPUT_TAG) {
            return Ok(None);
        }

        if content.len() <= persistence_threshold {
            return Ok(None);
        }

        let result = self.persist_tool_result(content, tool_use_id)?;
        let message = build_large_tool_result_message(&result);
        Ok(Some(message))
    }

    fn persist_tool_result(
        &self,
        content: &str,
        tool_use_id: &str,
    ) -> Result<PersistedToolResult, String> {
        let dir = self.tool_results_dir();
        fs::create_dir_all(&dir).map_err(|e| {
            format!("Failed to create tool-results directory {}: {e}", dir.display())
        })?;

        let filepath = self.tool_result_path(tool_use_id);

        // O_EXCL equivalent: create_new(true) fails if file already exists.
        // This makes replays safe — we never overwrite a prior persist.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&filepath)
        {
            Ok(mut f) => {
                f.write_all(content.as_bytes()).map_err(|e| {
                    format!("Failed to write tool result to {}: {e}", filepath.display())
                })?;
                tracing::debug!(
                    path = %filepath.display(),
                    size = content.len(),
                    "Persisted tool result to disk"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Already persisted on a prior turn — fall through to preview.
            }
            Err(e) => {
                return Err(format!(
                    "Failed to create tool result file {}: {e}",
                    filepath.display()
                ));
            }
        }

        let (preview, has_more) = generate_preview(content, PREVIEW_SIZE_BYTES);

        Ok(PersistedToolResult {
            filepath,
            original_size: content.len(),
            preview,
            has_more,
        })
    }
}

/// Build the XML-wrapped message that replaces a large tool result in the
/// conversation. The model sees the preview + file path and can use `read_file`
/// to access the full content.
pub fn build_large_tool_result_message(result: &PersistedToolResult) -> String {
    let size = format_file_size(result.original_size);
    let preview_size = format_file_size(PREVIEW_SIZE_BYTES);
    let trail = if result.has_more { "\n...\n" } else { "\n" };
    format!(
        "{PERSISTED_OUTPUT_TAG}\n\
         Output too large ({size}). Full output saved to: {path}\n\n\
         Preview (first {preview_size}):\n\
         {preview}{trail}\
         {PERSISTED_OUTPUT_CLOSING_TAG}",
        path = result.filepath.display(),
        preview = result.preview,
    )
}

/// Generate a preview of content, truncating at a newline boundary when possible.
pub fn generate_preview(content: &str, max_bytes: usize) -> (String, bool) {
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }

    let truncated = &content[..max_bytes.min(content.len())];
    let last_newline = truncated.rfind('\n');

    let cut_point = match last_newline {
        Some(pos) if pos > max_bytes / 2 => pos + 1,
        _ => max_bytes.min(content.len()),
    };

    (content[..cut_point].to_string(), true)
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// --- Message-level aggregate tool result budget ---

use std::collections::{HashMap, HashSet};

/// Per-conversation state for the aggregate tool result budget.
///
/// Once a tool_use_id enters `seen_ids`, its fate is frozen:
/// - If it's in `replacements` → re-applied identically every turn (cache stability)
/// - If it's only in `seen_ids` → never replaced (model already saw the full content)
pub struct ContentReplacementState {
    pub seen_ids: HashSet<String>,
    pub replacements: HashMap<String, String>,
}

impl ContentReplacementState {
    pub fn new() -> Self {
        Self {
            seen_ids: HashSet::new(),
            replacements: HashMap::new(),
        }
    }
}

impl Default for ContentReplacementState {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable record of one content-replacement decision.
/// Written to the transcript so decisions survive session resume.
#[derive(Debug, Clone)]
pub struct ContentReplacementRecord {
    pub tool_use_id: String,
    pub replacement: String,
}

/// A candidate tool result block eligible for budget evaluation.
struct ToolResultCandidate {
    tool_use_id: String,
    tool_name: String,
    content: String,
    size: usize,
}

struct CandidatePartition {
    must_reapply: Vec<(ToolResultCandidate, String)>,
    frozen: Vec<ToolResultCandidate>,
    fresh: Vec<ToolResultCandidate>,
}

fn partition_by_prior_decision(
    candidates: Vec<ToolResultCandidate>,
    state: &ContentReplacementState,
) -> CandidatePartition {
    let mut must_reapply = Vec::new();
    let mut frozen = Vec::new();
    let mut fresh = Vec::new();

    for c in candidates {
        if let Some(replacement) = state.replacements.get(&c.tool_use_id) {
            must_reapply.push((c, replacement.clone()));
        } else if state.seen_ids.contains(&c.tool_use_id) {
            frozen.push(c);
        } else {
            fresh.push(c);
        }
    }

    CandidatePartition { must_reapply, frozen, fresh }
}

/// Pick the largest fresh results to replace until the model-visible total
/// (frozen + remaining fresh) is at or under budget, or fresh is exhausted.
fn select_fresh_to_replace(
    fresh: &[ToolResultCandidate],
    frozen_size: usize,
    limit: usize,
) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..fresh.len()).collect();
    indices.sort_by(|&a, &b| fresh[b].size.cmp(&fresh[a].size));

    let total_fresh_size: usize = fresh.iter().map(|c| c.size).sum();
    let mut remaining = frozen_size + total_fresh_size;
    let mut selected = Vec::new();

    for idx in indices {
        if remaining <= limit {
            break;
        }
        selected.push(idx);
        remaining -= fresh[idx].size;
    }

    selected
}

/// Represents a tool result message in the conversation for budget evaluation.
pub struct ToolResultEntry {
    pub tool_use_id: String,
    pub tool_name: String,
    pub content: String,
}

/// Result of enforcing the per-message budget.
pub struct BudgetEnforcementResult {
    /// Map of tool_use_id → replacement content for entries that were replaced.
    pub replacements: HashMap<String, String>,
    /// Newly created replacement records (for transcript persistence).
    pub newly_replaced: Vec<ContentReplacementRecord>,
}

impl ToolResultStorage {
    /// Enforce the per-message budget on a group of tool result entries.
    ///
    /// Entries represent tool results from a single user message (or a group
    /// of consecutive tool result messages that will be merged on the wire).
    ///
    /// State is mutated in place: `seen_ids` and `replacements` are updated.
    ///
    /// `skip_tool_names`: tools explicitly excluded from budget calculation
    /// (those whose `max_result_size_chars() == usize::MAX`).
    pub fn enforce_per_message_budget(
        &self,
        entries: Vec<ToolResultEntry>,
        state: &mut ContentReplacementState,
        skip_tool_names: &HashSet<String>,
        budget: usize,
    ) -> BudgetEnforcementResult {
        let candidates: Vec<ToolResultCandidate> = entries
            .into_iter()
            .filter(|e| !e.content.starts_with(PERSISTED_OUTPUT_TAG))
            .map(|e| {
                let size = e.content.len();
                ToolResultCandidate {
                    tool_use_id: e.tool_use_id,
                    tool_name: e.tool_name,
                    content: e.content,
                    size,
                }
            })
            .collect();

        if candidates.is_empty() {
            return BudgetEnforcementResult {
                replacements: HashMap::new(),
                newly_replaced: Vec::new(),
            };
        }

        let CandidatePartition { must_reapply, frozen, mut fresh } =
            partition_by_prior_decision(candidates, state);

        let mut replacement_map: HashMap<String, String> = HashMap::new();

        for (c, replacement) in &must_reapply {
            replacement_map.insert(c.tool_use_id.clone(), replacement.clone());
            state.seen_ids.insert(c.tool_use_id.clone());
        }

        // Separate skipped tools from eligible fresh candidates (skip by tool name, not ID)
        let (skipped, eligible): (Vec<_>, Vec<_>) =
            fresh.drain(..).partition(|c| skip_tool_names.contains(&c.tool_name));

        for c in &skipped {
            state.seen_ids.insert(c.tool_use_id.clone());
        }

        let fresh = eligible;

        if fresh.is_empty() {
            for c in &frozen {
                state.seen_ids.insert(c.tool_use_id.clone());
            }
            return BudgetEnforcementResult {
                replacements: replacement_map,
                newly_replaced: Vec::new(),
            };
        }

        let frozen_size: usize = frozen.iter().map(|c| c.size).sum();
        let fresh_size: usize = fresh.iter().map(|c| c.size).sum();

        let selected_indices = if frozen_size + fresh_size > budget {
            select_fresh_to_replace(&fresh, frozen_size, budget)
        } else {
            Vec::new()
        };

        let selected_ids: HashSet<String> = selected_indices
            .iter()
            .map(|&i| fresh[i].tool_use_id.clone())
            .collect();

        // Mark non-selected as seen (frozen)
        for c in &frozen {
            state.seen_ids.insert(c.tool_use_id.clone());
        }
        for c in &fresh {
            if !selected_ids.contains(&c.tool_use_id) {
                state.seen_ids.insert(c.tool_use_id.clone());
            }
        }

        let mut newly_replaced = Vec::new();

        for &idx in &selected_indices {
            let c = &fresh[idx];
            match self.persist_tool_result(&c.content, &c.tool_use_id) {
                Ok(result) => {
                    let message = build_large_tool_result_message(&result);
                    state.seen_ids.insert(c.tool_use_id.clone());
                    state.replacements.insert(c.tool_use_id.clone(), message.clone());
                    replacement_map.insert(c.tool_use_id.clone(), message.clone());
                    newly_replaced.push(ContentReplacementRecord {
                        tool_use_id: c.tool_use_id.clone(),
                        replacement: message,
                    });
                }
                Err(_) => {
                    // Persist failed — mark as seen (frozen) with original content.
                    state.seen_ids.insert(c.tool_use_id.clone());
                }
            }
        }

        BudgetEnforcementResult {
            replacements: replacement_map,
            newly_replaced,
        }
    }
}

/// Reconstruct ContentReplacementState from transcript records + message IDs.
///
/// All tool_use_ids in `message_tool_use_ids` are frozen (added to seen_ids).
/// Records populate the replacements map.
pub fn reconstruct_state(
    message_tool_use_ids: &[String],
    records: &[ContentReplacementRecord],
) -> ContentReplacementState {
    let mut state = ContentReplacementState::new();
    let candidate_ids: HashSet<&String> = message_tool_use_ids.iter().collect();

    for id in message_tool_use_ids {
        state.seen_ids.insert(id.clone());
    }

    for r in records {
        if candidate_ids.contains(&r.tool_use_id) {
            state.replacements.insert(r.tool_use_id.clone(), r.replacement.clone());
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_storage() -> (ToolResultStorage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = ToolResultStorage::new(tmp.path().to_path_buf());
        (storage, tmp)
    }

    #[test]
    fn empty_result_returns_marker() {
        let (storage, _tmp) = make_storage();
        let result = storage
            .process_result("shell_exec", "id1", "", 50_000)
            .unwrap();
        assert_eq!(
            result,
            Some("(shell_exec completed with no output)".to_string())
        );
    }

    #[test]
    fn whitespace_only_result_returns_marker() {
        let (storage, _tmp) = make_storage();
        let result = storage
            .process_result("shell_exec", "id2", "   \n\t  ", 50_000)
            .unwrap();
        assert_eq!(
            result,
            Some("(shell_exec completed with no output)".to_string())
        );
    }

    #[test]
    fn already_persisted_returns_none() {
        let (storage, _tmp) = make_storage();
        let content = format!("{PERSISTED_OUTPUT_TAG}\nsome preview\n{PERSISTED_OUTPUT_CLOSING_TAG}");
        let result = storage
            .process_result("read_file", "id3", &content, 50_000)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn small_result_passes_through() {
        let (storage, _tmp) = make_storage();
        let result = storage
            .process_result("shell_exec", "id4", "hello world", 50_000)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn large_result_persists_to_disk() {
        let (storage, _tmp) = make_storage();
        let content = "x".repeat(60_000);
        let result = storage
            .process_result("shell_exec", "id5", &content, 50_000)
            .unwrap();
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.starts_with(PERSISTED_OUTPUT_TAG));
        assert!(msg.contains("Output too large"));
        assert!(msg.ends_with(PERSISTED_OUTPUT_CLOSING_TAG));

        let persisted_path = storage.tool_result_path("id5");
        assert!(persisted_path.exists());
        assert_eq!(fs::read_to_string(&persisted_path).unwrap().len(), 60_000);
    }

    #[test]
    fn duplicate_persist_is_idempotent() {
        let (storage, _tmp) = make_storage();
        let content = "y".repeat(60_000);
        let r1 = storage
            .process_result("shell_exec", "id6", &content, 50_000)
            .unwrap();
        let r2 = storage
            .process_result("shell_exec", "id6", &content, 50_000)
            .unwrap();
        assert!(r1.is_some());
        assert!(r2.is_some());
    }

    #[test]
    fn get_persistence_threshold_infinity_opt_out() {
        assert_eq!(get_persistence_threshold(usize::MAX), usize::MAX);
    }

    #[test]
    fn get_persistence_threshold_below_default() {
        assert_eq!(get_persistence_threshold(30_000), 30_000);
    }

    #[test]
    fn get_persistence_threshold_above_default() {
        assert_eq!(get_persistence_threshold(100_000), 50_000);
    }

    #[test]
    fn generate_preview_short_content() {
        let (preview, has_more) = generate_preview("hello", 2000);
        assert_eq!(preview, "hello");
        assert!(!has_more);
    }

    #[test]
    fn generate_preview_truncates_at_newline() {
        let mut content = String::new();
        for i in 0..200 {
            content.push_str(&format!("line {i}\n"));
        }
        let (preview, has_more) = generate_preview(&content, 100);
        assert!(has_more);
        assert!(preview.len() <= 100);
        assert!(preview.ends_with('\n'));
    }

    #[test]
    fn build_large_tool_result_message_format() {
        let result = PersistedToolResult {
            filepath: PathBuf::from("/tmp/test.txt"),
            original_size: 60_000,
            preview: "hello world".to_string(),
            has_more: true,
        };
        let msg = build_large_tool_result_message(&result);
        assert!(msg.starts_with(PERSISTED_OUTPUT_TAG));
        assert!(msg.contains("58.6 KB"));
        assert!(msg.contains("/tmp/test.txt"));
        assert!(msg.contains("hello world"));
        assert!(msg.contains("..."));
        assert!(msg.ends_with(PERSISTED_OUTPUT_CLOSING_TAG));
    }

    #[test]
    fn format_file_size_various() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(50_000), "48.8 KB");
        assert_eq!(format_file_size(1_048_576), "1.0 MB");
    }

    // ---- ContentReplacementState tests ----

    #[test]
    fn content_replacement_state_default() {
        let state = ContentReplacementState::default();
        assert!(state.seen_ids.is_empty());
        assert!(state.replacements.is_empty());
    }

    #[test]
    fn partition_fresh_when_state_empty() {
        let state = ContentReplacementState::new();
        let candidates = vec![
            ToolResultCandidate { tool_use_id: "a".into(), tool_name: "shell".into(), content: "x".into(), size: 100 },
            ToolResultCandidate { tool_use_id: "b".into(), tool_name: "shell".into(), content: "y".into(), size: 200 },
        ];
        let p = partition_by_prior_decision(candidates, &state);
        assert_eq!(p.fresh.len(), 2);
        assert!(p.must_reapply.is_empty());
        assert!(p.frozen.is_empty());
    }

    #[test]
    fn partition_frozen_when_seen_but_not_replaced() {
        let mut state = ContentReplacementState::new();
        state.seen_ids.insert("a".into());
        let candidates = vec![
            ToolResultCandidate { tool_use_id: "a".into(), tool_name: "shell".into(), content: "x".into(), size: 100 },
            ToolResultCandidate { tool_use_id: "b".into(), tool_name: "shell".into(), content: "y".into(), size: 200 },
        ];
        let p = partition_by_prior_decision(candidates, &state);
        assert_eq!(p.frozen.len(), 1);
        assert_eq!(p.fresh.len(), 1);
        assert!(p.must_reapply.is_empty());
    }

    #[test]
    fn partition_must_reapply_when_replaced() {
        let mut state = ContentReplacementState::new();
        state.seen_ids.insert("a".into());
        state.replacements.insert("a".into(), "[persisted]".into());
        let candidates = vec![
            ToolResultCandidate { tool_use_id: "a".into(), tool_name: "shell".into(), content: "x".into(), size: 100 },
        ];
        let p = partition_by_prior_decision(candidates, &state);
        assert_eq!(p.must_reapply.len(), 1);
        assert_eq!(p.must_reapply[0].1, "[persisted]");
        assert!(p.frozen.is_empty());
        assert!(p.fresh.is_empty());
    }

    #[test]
    fn select_fresh_to_replace_picks_largest() {
        let fresh = vec![
            ToolResultCandidate { tool_use_id: "a".into(), tool_name: "s".into(), content: String::new(), size: 100 },
            ToolResultCandidate { tool_use_id: "b".into(), tool_name: "s".into(), content: String::new(), size: 500 },
            ToolResultCandidate { tool_use_id: "c".into(), tool_name: "s".into(), content: String::new(), size: 200 },
        ];
        let selected = select_fresh_to_replace(&fresh, 0, 400);
        assert!(selected.contains(&1), "largest item (index 1, size 500) should be selected");
    }

    #[test]
    fn select_fresh_to_replace_nothing_when_under_budget() {
        let fresh = vec![
            ToolResultCandidate { tool_use_id: "a".into(), tool_name: "s".into(), content: String::new(), size: 100 },
        ];
        let selected = select_fresh_to_replace(&fresh, 0, 1000);
        assert!(selected.is_empty());
    }

    #[test]
    fn enforce_budget_under_limit_no_replacement() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let entries = vec![
            ToolResultEntry { tool_use_id: "t1".into(), tool_name: "shell".into(), content: "short".into() },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 200_000,
        );
        assert!(result.newly_replaced.is_empty());
        assert!(result.replacements.is_empty());
        assert!(state.seen_ids.contains("t1"));
    }

    #[test]
    fn enforce_budget_over_limit_persists_largest() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let big = "z".repeat(150_000);
        let small = "a".repeat(30_000);
        let entries = vec![
            ToolResultEntry { tool_use_id: "big".into(), tool_name: "shell".into(), content: big },
            ToolResultEntry { tool_use_id: "small".into(), tool_name: "shell".into(), content: small },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 100_000,
        );
        assert_eq!(result.newly_replaced.len(), 1);
        assert_eq!(result.newly_replaced[0].tool_use_id, "big");
        assert!(result.replacements.contains_key("big"));
        assert!(state.replacements.contains_key("big"));
        assert!(state.seen_ids.contains("big"));
        assert!(state.seen_ids.contains("small"));
    }

    #[test]
    fn enforce_budget_reapplies_existing_replacements() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        state.seen_ids.insert("prev".into());
        state.replacements.insert("prev".into(), "[cached-preview]".into());

        let entries = vec![
            ToolResultEntry { tool_use_id: "prev".into(), tool_name: "shell".into(), content: "x".repeat(100_000) },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 200_000,
        );
        assert_eq!(result.replacements.get("prev").unwrap(), "[cached-preview]");
        assert!(result.newly_replaced.is_empty());
    }

    #[test]
    fn enforce_budget_skips_already_persisted_content() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let already_persisted = format!("{PERSISTED_OUTPUT_TAG}\npreview\n{PERSISTED_OUTPUT_CLOSING_TAG}");
        let entries = vec![
            ToolResultEntry { tool_use_id: "p1".into(), tool_name: "shell".into(), content: already_persisted },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 200_000,
        );
        assert!(result.newly_replaced.is_empty());
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn enforce_budget_skip_tool_names() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let big = "z".repeat(200_000);
        let mut skip = HashSet::new();
        skip.insert("custom_exempt_tool".into());
        let entries = vec![
            ToolResultEntry { tool_use_id: "t1".into(), tool_name: "custom_exempt_tool".into(), content: big },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &skip, 100_000,
        );
        assert!(result.newly_replaced.is_empty());
        assert!(state.seen_ids.contains("t1"));
    }

    #[test]
    fn enforce_budget_read_file_skipped_when_in_skip_set() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let big = "z".repeat(150_000);
        let small = "a".repeat(30_000);
        let mut skip = HashSet::new();
        skip.insert("read_file".into());
        let entries = vec![
            ToolResultEntry { tool_use_id: "rf1".into(), tool_name: "read_file".into(), content: big },
            ToolResultEntry { tool_use_id: "rf2".into(), tool_name: "read_file".into(), content: small },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &skip, 100_000,
        );
        assert!(result.newly_replaced.is_empty(), "read_file should be skipped");
        assert!(state.seen_ids.contains("rf1"));
        assert!(state.seen_ids.contains("rf2"));
    }

    // ---- reconstruct_state tests ----

    #[test]
    fn reconstruct_state_populates_seen_and_replacements() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let records = vec![
            ContentReplacementRecord { tool_use_id: "a".into(), replacement: "[r]".into() },
            ContentReplacementRecord { tool_use_id: "z".into(), replacement: "[orphan]".into() },
        ];
        let state = reconstruct_state(&ids, &records);
        assert_eq!(state.seen_ids.len(), 3);
        assert!(state.seen_ids.contains("a"));
        assert!(state.seen_ids.contains("b"));
        assert!(state.seen_ids.contains("c"));
        assert_eq!(state.replacements.len(), 1);
        assert_eq!(state.replacements.get("a").unwrap(), "[r]");
        assert!(!state.replacements.contains_key("z"));
    }

    #[test]
    fn reconstruct_state_empty_inputs() {
        let state = reconstruct_state(&[], &[]);
        assert!(state.seen_ids.is_empty());
        assert!(state.replacements.is_empty());
    }

    // ---- Additional coverage ----

    #[test]
    fn process_result_infinity_threshold_never_persists() {
        let (storage, _tmp) = make_storage();
        let big = "x".repeat(500_000);
        let result = storage
            .process_result("read_file", "id-inf", &big, usize::MAX)
            .unwrap();
        assert!(result.is_none(), "usize::MAX threshold must never trigger persistence");
    }

    #[test]
    fn generate_preview_no_newline_falls_back_to_byte_cut() {
        let content = "a".repeat(5000);
        let (preview, has_more) = generate_preview(&content, 100);
        assert!(has_more);
        assert_eq!(preview.len(), 100);
        assert_eq!(preview, "a".repeat(100));
    }

    #[test]
    fn generate_preview_newline_in_first_half_uses_byte_cut() {
        let mut content = String::new();
        content.push_str("short\n");
        content.push_str(&"x".repeat(5000));
        let (preview, has_more) = generate_preview(&content, 100);
        assert!(has_more);
        assert_eq!(preview.len(), 100, "newline at pos 5 (< 50) should be ignored, use byte cut");
    }

    #[test]
    fn enforce_budget_multi_group_selects_largest_across_all() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        let entries = vec![
            ToolResultEntry { tool_use_id: "g1".into(), tool_name: "shell".into(), content: "a".repeat(80_000) },
            ToolResultEntry { tool_use_id: "g2".into(), tool_name: "shell".into(), content: "b".repeat(60_000) },
            ToolResultEntry { tool_use_id: "g3".into(), tool_name: "shell".into(), content: "c".repeat(40_000) },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 100_000,
        );
        assert!(!result.newly_replaced.is_empty());
        let replaced_ids: Vec<&str> = result.newly_replaced.iter().map(|r| r.tool_use_id.as_str()).collect();
        assert!(replaced_ids.contains(&"g1"), "largest (80k) must be replaced first");
    }

    #[test]
    fn reconstruct_state_then_enforce_produces_consistent_decisions() {
        let (storage, _tmp) = make_storage();

        let mut state = ContentReplacementState::new();
        let big = "z".repeat(150_000);
        let entries = vec![
            ToolResultEntry { tool_use_id: "t1".into(), tool_name: "shell".into(), content: big.clone() },
            ToolResultEntry { tool_use_id: "t2".into(), tool_name: "shell".into(), content: "small".into() },
        ];
        let r1 = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 100_000,
        );
        assert_eq!(r1.newly_replaced.len(), 1);
        let original_replacement = r1.newly_replaced[0].replacement.clone();

        let records: Vec<ContentReplacementRecord> = r1.newly_replaced;
        let ids = vec!["t1".to_string(), "t2".to_string()];
        let reconstructed = reconstruct_state(&ids, &records);

        assert!(reconstructed.seen_ids.contains("t1"));
        assert!(reconstructed.seen_ids.contains("t2"));
        assert_eq!(
            reconstructed.replacements.get("t1").unwrap(),
            &original_replacement,
            "reconstructed replacement must be byte-identical"
        );
    }

    #[test]
    fn enforce_budget_frozen_ids_not_replaceable() {
        let (storage, _tmp) = make_storage();
        let mut state = ContentReplacementState::new();
        state.seen_ids.insert("frozen_id".into());

        let entries = vec![
            ToolResultEntry { tool_use_id: "frozen_id".into(), tool_name: "shell".into(), content: "x".repeat(200_000) },
        ];
        let result = storage.enforce_per_message_budget(
            entries, &mut state, &HashSet::new(), 50_000,
        );
        assert!(result.newly_replaced.is_empty(), "frozen (seen but not replaced) must not be replaced");
        assert!(!result.replacements.contains_key("frozen_id"));
    }

    #[test]
    fn session_based_storage_survives_recreation() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("my_session");

        let content = "x".repeat(60_000);

        // First storage instance — persists a large result
        {
            let storage = ToolResultStorage::new(session_dir.clone());
            let result = storage
                .process_result("shell_exec", "tool_abc", &content, 50_000)
                .unwrap();
            assert!(result.is_some());
        }

        // Second storage instance (simulates session resume) — file still exists
        {
            let storage = ToolResultStorage::new(session_dir.clone());
            let filepath = storage.tool_result_path("tool_abc");
            assert!(filepath.exists(), "tool result must survive storage recreation");
            let read_back = fs::read_to_string(&filepath).unwrap();
            assert_eq!(read_back.len(), 60_000);
            assert_eq!(read_back, content);
        }
    }

    #[test]
    fn tool_result_path_uses_session_dir() {
        let storage = ToolResultStorage::new(PathBuf::from("/home/user/.fastclaw/sessions/sid123"));
        let path = storage.tool_result_path("call_456");
        assert_eq!(
            path,
            PathBuf::from("/home/user/.fastclaw/sessions/sid123/tool-results/call_456.txt")
        );
    }
}
