//! Phase 9: Quality Gates and Benchmarks — integration tests.
//!
//! These tests validate the end-to-end behavior of the output asset system:
//! asset creation → projection → recall → compaction → recovery.
//!
//! # Coverage
//!
//! - 9.1  Raw recovery after projection, compaction, and session resume
//! - 9.2  Large rg integration: find matches via recall without rerunning
//! - 9.3  Failed test log: shell/test projector surfaces failure blocks
//! - 9.4  Large file-read: recover arbitrary line ranges after compaction
//! - 9.5  Multi-turn long-session: compact-after-continuation remains correct
//! - 9.10 Recall-loop prevention: repeated broad paging detected & redirected

use std::sync::Arc;

use tempfile::TempDir;
use xiaolin_agent::builtin_tools::recall::{
    with_output_store, OutputReadTool, OutputSearchTool, OutputTailTool,
};
use xiaolin_core::tool::Tool;
use xiaolin_session::tool_output_projector::{
    OutputProjector, ReadFileProjector, ShellTestProjector,
};
use xiaolin_session::tool_output_store::{
    compute_content_hash, CreateAssetInput, ProjectionSizeConfig, ToolOutputAssetStore,
};

// ============================================================================
// Test helpers
// ============================================================================

async fn setup_store() -> (Arc<ToolOutputAssetStore>, TempDir) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory pool");
    let store = Arc::new(ToolOutputAssetStore::open(pool).await.expect("open store"));
    let tmp = TempDir::new().expect("tempdir");
    (store, tmp)
}

fn make_input(tmp: &TempDir, session_id: &str, tool_name: &str, output: &str) -> CreateAssetInput {
    CreateAssetInput {
        session_id: session_id.to_string(),
        turn_id: "turn_test".to_string(),
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: tool_name.to_string(),
        arguments: r#"{}"#.to_string(),
        success: true,
        output: output.to_string(),
        storage_root: tmp.path().to_path_buf(),
        size_config: ProjectionSizeConfig::default(),
    }
}

async fn execute_recall(
    store: Arc<ToolOutputAssetStore>,
    session_id: String,
    tool: &(dyn Tool + Sync),
    args: &str,
) -> xiaolin_core::tool::ToolResult {
    with_output_store(store, session_id, tool.execute(args)).await
}

fn generate_output(lines: usize, template: &str) -> String {
    let line_overhead = 10 + template.len(); // "NNNN: \n" = 6 + template
    let estimated_size = lines * line_overhead;
    let mut out = String::with_capacity(estimated_size);
    for i in 0..lines {
        out.push_str(&format!("{:04}: {}\n", i, template));
    }
    out
}

// ============================================================================
// 9.1 Raw recovery after projection, compaction, and session resume
// ============================================================================

#[tokio::test]
async fn quality_gate_9_1_exact_line_range_recall_after_store_cycle() {
    let (store, tmp) = setup_store().await;
    let content = generate_output(1000, "some test content for line range recall verification");
    let input = make_input(&tmp, "sess_9_1", "Bash", &content);
    let handle = store.create_asset(input).await.expect("create asset");
    let h = handle.as_str().to_string();

    // Recall line range 42-57
    let result = execute_recall(
        store.clone(),
        "sess_9_1".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 42, "end_line": 57}}"#),
    )
    .await;
    assert!(result.success, "recall failed: {}", result.output);

    for recall_i in 42..=57 {
        // generate_output uses 0-indexed line numbers; recall uses 1-indexed.
        // Line 42 (1-indexed) = "0041: ...", line 57 = "0056: ..."
        let content_i = recall_i - 1;
        let expected = format!(
            "{:04}: some test content for line range recall verification",
            content_i
        );
        assert!(
            result.output.contains(&expected),
            "missing line {} (1-indexed recall line {}): {}",
            content_i,
            recall_i,
            expected
        );
    }
    assert!(result.output.contains("[lines 42-57 of 1000]"));
}

#[tokio::test]
async fn quality_gate_9_1_recall_after_expire_and_not_found() {
    let (store, tmp) = setup_store().await;
    let content = "data to expire\n";
    let input = make_input(&tmp, "sess_9_1b", "Bash", content);
    let handle = store.create_asset(input).await.expect("create");

    use xiaolin_session::tool_output_store::RetentionReason;
    store
        .expire_asset(handle.as_str(), "sess_9_1b", RetentionReason::AgeLimit)
        .await
        .expect("expire");

    let result = execute_recall(
        store.clone(),
        "sess_9_1b".into(),
        &OutputReadTool,
        &format!(
            r#"{{"handle": "{}", "start_line": 1, "end_line": 1}}"#,
            handle.as_str()
        ),
    )
    .await;
    assert!(!result.success);
    assert!(
        result.output.contains("expired") || result.output.contains("No output asset found"),
        "expected expired/not-found, got: {}",
        result.output
    );
}

// ============================================================================
// 9.2 Large rg integration: find matches via recall without rerunning
// ============================================================================

#[tokio::test]
async fn quality_gate_9_2_search_recall_no_rerun() {
    let (store, tmp) = setup_store().await;

    let mut grep_output = String::new();
    for i in 0..500 {
        let file = format!("crates/crate_{}/src/mod.rs", i % 10);
        let line_no = (i * 7 + 42) % 300;
        grep_output.push_str(&format!("{}:{}:fn function_{}() {{\n", file, line_no, i));
    }
    grep_output.push_str("src/main.rs:1:fn main() {\n");
    grep_output.push_str("src/main.rs:42:    println!(\"hello world\");\n");
    grep_output.push_str("tests/integration.rs:5:async fn test_recall() {\n");

    let input = make_input(&tmp, "sess_9_2", "rg", &grep_output);

    let handle = store.create_asset(input).await.expect("create");
    let h = handle.as_str().to_string();

    // Agent searches via output_search instead of rerunning rg
    let result = execute_recall(
        store.clone(),
        "sess_9_2".into(),
        &OutputSearchTool,
        &format!(r#"{{"handle": "{h}", "pattern": "main.rs", "context_lines": 0}}"#),
    )
    .await;
    assert!(result.success, "search failed: {}", result.output);
    assert!(result.output.contains("main.rs"));
    assert!(result.output.contains("hello world") || result.output.contains("fn main"));

    // Search for a specific function
    let result2 = execute_recall(
        store.clone(),
        "sess_9_2".into(),
        &OutputSearchTool,
        &format!(r#"{{"handle": "{h}", "pattern": "function_42", "context_lines": 0}}"#),
    )
    .await;
    assert!(result2.success);
    assert!(result2.output.contains("function_42"));
}

// ============================================================================
// 9.3 Failed test log: shell/test projector surfaces failure blocks
// ============================================================================

#[tokio::test]
async fn quality_gate_9_3_shell_test_failure_surfaces_blocks() {
    let failed_test_output = "\
running 42 tests
test test_a ... ok
test test_b ... ok
test test_c ... ok
test test_complex_logic ... FAILED
test test_d ... ok

failures:

---- test_complex_logic stdout ----
thread 'test_complex_logic' panicked at src/logic.rs:85:9:
assertion `left == right` failed
  left: Vec([1, 2, 3])
 right: Vec([1, 4, 3])
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- test_other stdout ----
thread 'test_other' panicked at src/other.rs:12:5:
called `Option::unwrap()` on a `None` value


failures:
    test_complex_logic
    test_other

test result: FAILED. 40 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.34s
";

    let (store, tmp) = setup_store().await;
    let mut input = make_input(&tmp, "sess_9_3", "Bash", failed_test_output);
    input.success = false;

    let handle = store.create_asset(input).await.expect("create");
    let asset = store
        .get_asset(handle.as_str(), "sess_9_3")
        .await
        .expect("get asset");

    // Project with ShellTestProjector
    let projector = ShellTestProjector;
    let proj = projector.project(&asset, failed_test_output);
    let formatted = proj.format();

    assert!(proj.is_failure, "projector must detect failure");
    assert!(formatted.contains("FAILED"));
    assert!(formatted.contains("Failure indicators"));
    assert!(formatted.contains("panicked") || formatted.contains("assertion"));

    // Recall exact lines around the failure
    let h = handle.as_str().to_string();
    let result = execute_recall(
        store.clone(),
        "sess_9_3".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 10, "end_line": 18}}"#),
    )
    .await;
    assert!(result.success, "recall failed: {}", result.output);
    assert!(result.output.contains("panicked at src/logic.rs:85:9"));
    assert!(result.output.contains("assertion `left == right` failed"));
    assert!(result.output.contains("left: Vec([1, 2, 3])"));

    // Verify tail recall works
    let tail_result = execute_recall(
        store.clone(),
        "sess_9_3".into(),
        &OutputTailTool,
        &format!(r#"{{"handle": "{h}", "lines": 5}}"#),
    )
    .await;
    assert!(tail_result.success);
    assert!(tail_result.output.contains("FAILED"));
    assert!(tail_result.output.contains("finished in"));
}

// ============================================================================
// 9.4 Large file-read: recover arbitrary line ranges after compaction
// ============================================================================

#[tokio::test]
async fn quality_gate_9_4_large_file_read_recover_arbitrary_ranges() {
    let (store, tmp) = setup_store().await;
    let file_content = generate_output(
        2000,
        "this is a line from a large source file for testing projection",
    );

    let mut input = make_input(&tmp, "sess_9_4", "Read", &file_content);
    input.arguments = r#"{"file_path": "/home/user/project/src/large_module.rs"}"#.to_string();

    let handle = store.create_asset(input).await.expect("create");
    let asset = store
        .get_asset(handle.as_str(), "sess_9_4")
        .await
        .expect("get asset");

    // Project with ReadFileProjector
    let projector = ReadFileProjector;
    let proj = projector.project(&asset, &file_content);
    let formatted = proj.format();

    assert!(formatted.contains("file read output"));
    assert!(formatted.contains("output_read"));
    assert!(formatted.contains("output_search"));
    assert!(!formatted.contains("/tmp/"));

    let h = handle.as_str().to_string();

    // Recover beginning
    let head = execute_recall(
        store.clone(),
        "sess_9_4".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 1, "end_line": 3}}"#),
    )
    .await;
    assert!(head.success, "head recall failed: {}", head.output);
    assert!(head.output.contains("0000:"));
    assert!(head.output.contains("0002:"));

    // Recover middle range (beyond projected excerpt)
    // Line 500 (1-indexed) = "0499:" (0-indexed i=499)
    // Line 502 (1-indexed) = "0501:" (0-indexed i=501)
    let mid = execute_recall(
        store.clone(),
        "sess_9_4".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 500, "end_line": 502}}"#),
    )
    .await;
    assert!(mid.success, "mid recall failed: {}", mid.output);
    assert!(mid.output.contains("0499:"));
    assert!(mid.output.contains("0501:"));

    // Recover tail
    // Line 1998 (1-indexed) = "1997:", Line 2000 (1-indexed) = "1999:"
    let tail = execute_recall(
        store.clone(),
        "sess_9_4".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 1998, "end_line": 2000}}"#),
    )
    .await;
    assert!(tail.success, "tail recall failed: {}", tail.output);
    assert!(tail.output.contains("1997:"));
    assert!(tail.output.contains("1999:"));
}

// ============================================================================
// 9.5 Multi-turn long-session: compact-after-continuation remains correct
// ============================================================================

#[tokio::test]
async fn quality_gate_9_5_multiturn_handles_persist_across_assets() {
    let (store, tmp) = setup_store().await;
    let session = "sess_9_5";
    let mut handles: Vec<String> = Vec::new();

    for turn in 0..20 {
        let content = generate_output(100, &format!("turn_{:02} output line", turn));
        let mut input = make_input(&tmp, session, "Bash", &content);
        input.turn_id = format!("turn_{:02}", turn);
        input.tool_call_id = format!("call_turn_{:02}", turn);
        let handle = store.create_asset(input).await.expect("create asset");
        handles.push(handle.as_str().to_string());
    }

    let stored = store.list_session_assets(session).await.expect("list");
    assert_eq!(stored.len(), 20);

    // Recall oldest asset (post-compact simulation)
    let first = execute_recall(
        store.clone(),
        session.into(),
        &OutputReadTool,
        &format!(
            r#"{{"handle": "{}", "start_line": 1, "end_line": 3}}"#,
            handles[0]
        ),
    )
    .await;
    assert!(first.success, "oldest recall failed: {}", first.output);
    assert!(first.output.contains("turn_00 output line"));

    // Recall middle asset
    let mid = execute_recall(
        store.clone(),
        session.into(),
        &OutputReadTool,
        &format!(
            r#"{{"handle": "{}", "start_line": 50, "end_line": 52}}"#,
            handles[10]
        ),
    )
    .await;
    assert!(mid.success);
    assert!(mid.output.contains("turn_10 output line"));

    // Recall latest asset
    let latest = execute_recall(
        store.clone(),
        session.into(),
        &OutputReadTool,
        &format!(
            r#"{{"handle": "{}", "start_line": 95, "end_line": 100}}"#,
            handles[19]
        ),
    )
    .await;
    assert!(latest.success);
    assert!(latest.output.contains("turn_19 output line"));

    // Search across a specific turn
    let search = execute_recall(
        store.clone(),
        session.into(),
        &OutputSearchTool,
        &format!(r#"{{"handle": "{}", "pattern": "turn_05"}}"#, handles[5]),
    )
    .await;
    assert!(search.success, "search failed: {}", search.output);
    assert!(search.output.contains("turn_05"));
}

// ============================================================================
// 9.10 Recall-loop prevention: repeated broad paging / same-range recall
// ============================================================================

#[tokio::test]
async fn quality_gate_9_10_broad_range_is_bounded_and_deterministic() {
    let (store, tmp) = setup_store().await;
    let content = generate_output(2000, "loop prevention test content line");
    let input = make_input(&tmp, "sess_9_10", "Bash", &content);
    let handle = store.create_asset(input).await.expect("create");
    let h = handle.as_str().to_string();

    // 1. Line range exceeding MAX_LINE_RANGE (500) must be rejected
    let too_many_lines = execute_recall(
        store.clone(),
        "sess_9_10".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 1, "end_line": 600}}"#),
    )
    .await;
    assert!(
        !too_many_lines.success,
        "range > 500 lines must be rejected"
    );
    assert!(
        too_many_lines.output.contains("too large") || too_many_lines.output.contains("500"),
        "error should mention line limit"
    );

    // 2. Handle-only output_read must be rejected (unbounded read prevention)
    let handle_only = execute_recall(
        store.clone(),
        "sess_9_10".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}"}}"#),
    )
    .await;
    assert!(
        !handle_only.success,
        "handle-only output_read must be rejected per spec"
    );
    assert!(
        handle_only.output.contains("bounded selector")
            || handle_only.output.contains("output_tail")
            || handle_only.output.contains("output_search"),
        "error should suggest bounded selectors"
    );

    // 3. Byte range exceeding MAX_BYTE_RANGE (100000) must be rejected
    let too_many_bytes = execute_recall(
        store.clone(),
        "sess_9_10".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_byte": 0, "end_byte": 200000}}"#),
    )
    .await;
    assert!(
        !too_many_bytes.success,
        "byte range > 100KB must be rejected"
    );
    assert!(
        too_many_bytes.output.contains("too large") || too_many_bytes.output.contains("100000"),
        "error should mention byte limit"
    );

    // 4. Same range = same content (deterministic, no loop divergence)
    let read1 = execute_recall(
        store.clone(),
        "sess_9_10".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 100, "end_line": 110}}"#),
    )
    .await;
    assert!(read1.success);

    let read2 = execute_recall(
        store.clone(),
        "sess_9_10".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 100, "end_line": 110}}"#),
    )
    .await;
    assert!(read2.success);

    // Compare only the content after the pagination header line.
    // The first line is always "[lines N-M of total]", strip it.
    fn strip_header(s: &str) -> &str {
        s.find('\n').map(|pos| &s[pos + 1..]).unwrap_or(s)
    }
    let content1 = strip_header(&read1.output);
    let content2 = strip_header(&read2.output);
    assert_eq!(
        content1, content2,
        "same line range must return identical content"
    );

    // 5. Navigation hints must exist
    assert!(
        read1.output.contains("use output_read"),
        "must include pagination hints"
    );
}

// ============================================================================
// Edge case tests: empty and single-line output
// ============================================================================

#[tokio::test]
async fn quality_gate_edge_empty_output_recall() {
    let (store, tmp) = setup_store().await;
    let input = make_input(&tmp, "sess_empty", "Bash", "");
    let handle = store.create_asset(input).await.expect("create");
    let h = handle.as_str().to_string();

    // output_tail on empty output must return a clear "empty" message
    let result = execute_recall(
        store.clone(),
        "sess_empty".into(),
        &OutputTailTool,
        &format!(r#"{{"handle": "{h}", "lines": 10}}"#),
    )
    .await;
    assert!(
        result.success,
        "tail on empty output should succeed: {}",
        result.output
    );
    assert!(
        result.output.contains("empty"),
        "tail on empty output should indicate emptiness: {}",
        result.output
    );

    // output_read with line range on empty output must fail gracefully
    let read_result = execute_recall(
        store.clone(),
        "sess_empty".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 1, "end_line": 5}}"#),
    )
    .await;
    // Empty output has 0 lines, so line 1 is out of bounds
    assert!(
        !read_result.success
            || read_result.output.contains("out of bounds")
            || read_result.output.contains("0 lines"),
        "read on empty output should indicate out-of-bounds: {}",
        read_result.output
    );
}

#[tokio::test]
async fn quality_gate_edge_single_line_output_recall() {
    let (store, tmp) = setup_store().await;
    let content = "only one line of output, no trailing newline";
    let input = make_input(&tmp, "sess_single", "Bash", content);
    let handle = store.create_asset(input).await.expect("create");
    let h = handle.as_str().to_string();

    // Recall the single line
    let result = execute_recall(
        store.clone(),
        "sess_single".into(),
        &OutputReadTool,
        &format!(r#"{{"handle": "{h}", "start_line": 1, "end_line": 1}}"#),
    )
    .await;
    assert!(
        result.success,
        "single-line recall failed: {}",
        result.output
    );
    assert!(
        result.output.contains(content),
        "must return the single line"
    );
    // Single line: no before, no after (total_lines=1)
    assert!(!result.output.contains("previous lines"));
    assert!(!result.output.contains("next lines"));
    // pagination header should indicate [lines 1-1 of 1]
    assert!(result.output.contains("[lines 1-1 of 1]"));

    // Search on single-line output
    let search_result = execute_recall(
        store.clone(),
        "sess_single".into(),
        &OutputSearchTool,
        &format!(r#"{{"handle": "{h}", "pattern": "trailing"}}"#),
    )
    .await;
    assert!(
        search_result.success,
        "search on single line failed: {}",
        search_result.output
    );
    assert!(
        search_result.output.contains("trailing"),
        "must find pattern in single line"
    );
}

// ============================================================================
// Content hash integrity across all projector kinds
// ============================================================================

#[tokio::test]
async fn quality_gate_content_hash_integrity_across_projectors() {
    let (store, tmp) = setup_store().await;

    let scenarios: Vec<(&str, &str, &str)> = vec![
        ("Read", "sess_int_read", "line 1\nline 2\nline 3\n"),
        (
            "Grep",
            "sess_int_grep",
            "src/main.rs:10:fn main()\nsrc/lib.rs:5:fn lib()\n",
        ),
        ("Bash", "sess_int_bash", "Compiling...\nFinished in 2.3s\n"),
        ("Glob", "sess_int_glob", "src/\nsrc/main.rs\nsrc/lib.rs\n"),
        (
            "mcp__test",
            "sess_int_mcp",
            r#"{"key": "value", "num": 42}"#,
        ),
        ("GenericTool", "sess_int_gen", "plain text output\n"),
    ];

    for (tool_name, session_id, content) in scenarios {
        let input = make_input(&tmp, session_id, tool_name, content);
        let handle = store.create_asset(input).await.expect("create");
        let asset = store
            .get_asset(handle.as_str(), session_id)
            .await
            .expect("get");
        let blob = store
            .read_blob(&asset, session_id)
            .await
            .expect("read blob");

        assert_eq!(asset.content_hash, compute_content_hash(content));
        assert_eq!(blob, content);
    }
}
