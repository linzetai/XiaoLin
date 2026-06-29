//! Phase 9: Performance benchmarks for output asset ingestion, indexing,
//! search, and page reads.
//!
//! Run with: `cargo bench -p xiaolin-agent --bench output_asset_bench`
//!
//! # Benchmarks
//!
//! - 9.6  Multi-megabyte ingestion, indexing, search, page reads
//! - 9.7  Token budget: projection tokens bounded, raw/projected separate
//! - 9.8  Prompt-cache stability: deterministic manifest formatting
//! - 9.9  No-negative-optimization: small/medium outputs don't need extra recall

use std::time::Instant;

use xiaolin_session::tool_output_projector::ProjectorRegistry;
use xiaolin_session::tool_output_store::{
    compute_content_hash, count_bytes_and_lines, AssetLifecycle, ChunkIndex, LineIndex,
    OutputSizeClass, ProjectionSizeConfig, ProjectorKind, ToolOutputAsset, ToolOutputHandle,
};

// ============================================================================
// Benchmark helpers
// ============================================================================

fn make_test_asset(
    kind: ProjectorKind,
    tool_name: &str,
    output: &str,
) -> (ToolOutputAsset, String) {
    let (byte_count, line_count) = count_bytes_and_lines(output);
    let content_hash = compute_content_hash(output);
    let estimated_tokens = byte_count / 4;
    let size_class = OutputSizeClass::classify(
        byte_count,
        line_count,
        estimated_tokens,
        &ProjectionSizeConfig::default(),
    );

    let asset = ToolOutputAsset {
        handle: ToolOutputHandle::new("bench_session"),
        session_id: "bench_session".into(),
        turn_id: "turn_bench".into(),
        tool_call_id: "call_bench".into(),
        tool_name: tool_name.into(),
        arguments_digest: "bench_digest".into(),
        success: true,
        lifecycle: AssetLifecycle::Active,
        projector_kind: kind,
        byte_count,
        line_count,
        estimated_tokens,
        size_class,
        content_hash,
        blob_path: "/tmp/bench.blob".into(),
        line_index_path: None,
        chunk_index_path: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        last_accessed_at: "2026-01-01T00:00:00Z".into(),
        expired_at: None,
    };

    (asset, output.to_string())
}

fn generate_text_output(size_bytes: usize) -> String {
    let line = "BENCHMARK LINE: This is a line of text generated for output asset benchmark testing purposes.\n";
    let line_len = line.len();
    let lines = size_bytes / line_len;
    let mut out = String::with_capacity(size_bytes);
    for _i in 0..lines {
        out.push_str(line);
    }
    out
}

fn generate_grep_output(num_matches: usize) -> String {
    let mut out = String::new();
    for i in 0..num_matches {
        let file = format!("crates/crate_{}/src/lib.rs", i % 20);
        let line_no = (i * 13 + 7) % 500;
        out.push_str(&format!(
            "{}:{}:fn benchmark_fn_{:08}() -> Result<(), Error> {{\n",
            file, line_no, i
        ));
    }
    out
}

fn count_tokens(text: &str) -> usize {
    text.len() / 4
}

// ============================================================================
// 9.6: Multi-megabyte ingestion, indexing, search, and page reads
// ============================================================================

fn bench_9_6_ingestion_and_indexing() {
    println!("=== 9.6 Ingestion & Indexing Benchmarks ===");
    println!();

    for &(label, size_kb, max_line_idx_ms, max_chunk_idx_ms, max_hash_ms) in &[
        ("100 KB", 100, 10, 20, 10),
        ("500 KB", 500, 25, 50, 25),
        ("1 MB", 1_000, 50, 100, 50),
        ("5 MB", 5_000, 50, 100, 50),
        ("10 MB (large asset limit)", 10_000, 50, 100, 50),
    ] {
        let content = generate_text_output(size_kb * 1024);
        let (byte_count, line_count) = count_bytes_and_lines(&content);

        let start = Instant::now();
        let _line_idx = LineIndex::build(&content);
        let line_idx_time = start.elapsed();

        let start = Instant::now();
        let chunk_idx = ChunkIndex::build(&content, 4096);
        let chunk_idx_time = start.elapsed();

        let start = Instant::now();
        let hash = compute_content_hash(&content);
        let hash_time = start.elapsed();

        println!(
            "  {label}: {} bytes, {} lines, {} pages",
            byte_count, line_count, chunk_idx.total_pages
        );
        println!(
            "    LineIndex: {:?}, ChunkIndex: {:?}, Hash: {:?}",
            line_idx_time, chunk_idx_time, hash_time
        );
        println!("    Hash: {}...", &hash[..16]);

        assert!(
            line_idx_time.as_millis() <= max_line_idx_ms as u128,
            "LineIndex build for {label} ({line_idx_time:?}) exceeds {max_line_idx_ms}ms threshold"
        );
        assert!(
            chunk_idx_time.as_millis() <= max_chunk_idx_ms as u128,
            "ChunkIndex build for {label} ({chunk_idx_time:?}) exceeds {max_chunk_idx_ms}ms threshold"
        );
        assert!(
            hash_time.as_millis() <= max_hash_ms as u128,
            "Content hash for {label} ({hash_time:?}) exceeds {max_hash_ms}ms threshold"
        );
    }

    println!();
    println!("=== 9.6 Search Performance ===");
    println!();

    for &(label, num_matches, max_search_ms) in &[
        ("1K matches", 1_000, 100),
        ("10K matches", 10_000, 200),
        ("50K matches", 50_000, 500),
    ] {
        let content = generate_grep_output(num_matches);
        let pattern = "benchmark_fn_0042";

        let start = Instant::now();
        let matching: Vec<usize> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(pattern))
            .map(|(i, _)| i)
            .collect();
        let search_time = start.elapsed();

        println!(
            "  {label}: found {} matches in {:?} ({} bytes)",
            matching.len(),
            search_time,
            content.len()
        );

        assert!(
            search_time.as_millis() <= max_search_ms as u128,
            "Search for {label} ({search_time:?}) exceeds {max_search_ms}ms threshold"
        );
    }

    println!();
    println!("=== 9.6 Page Read Performance ===");
    println!();

    let large_content = generate_text_output(5 * 1024 * 1024); // 5 MB
    let chunk_idx = ChunkIndex::build(&large_content, 4096);
    let total_bytes = large_content.len();

    let start = Instant::now();
    let mut pages_read = 0usize;
    for page in 1..=chunk_idx.total_pages {
        if let Some((start, end)) = chunk_idx.page_range(page, total_bytes) {
            let _content = &large_content[start..end];
            pages_read += 1;
        }
    }
    let elapsed = start.elapsed();

    let us_per_page = elapsed.as_micros() as f64 / pages_read.max(1) as f64;
    println!(
        "  Read {} pages (5 MB): {:?} ({:.1} µs/page)",
        pages_read, elapsed, us_per_page
    );

    assert!(
        us_per_page < 1000.0,
        "Page read time {:.1} µs/page exceeds 1ms/page threshold",
        us_per_page
    );

    println!();
    println!("  ✓ Ingestion, search, and page read benchmarks passed.");
}

// ============================================================================
// 9.7: Token budget — projection tokens bounded, raw/projected separate
// ============================================================================

fn bench_9_7_token_budget_accounting() {
    println!();
    println!("=== 9.7 Token Budget Benchmarks ===");
    println!();

    let projectors = ProjectorRegistry::new();

    let small_output = "Small output: just a few lines\nof text.\n".to_string();
    let medium_output = generate_text_output(40_000);
    let large_output = generate_text_output(100_000);

    for (label, output, tool_name, kind) in [
        ("small", &small_output, "Bash", ProjectorKind::ShellTest),
        ("medium", &medium_output, "Bash", ProjectorKind::ShellTest),
        ("large", &large_output, "Read", ProjectorKind::ReadFile),
    ] {
        let (asset, raw) = make_test_asset(kind, tool_name, output);
        let raw_tokens = raw.len() / 4;

        let proj = projectors.project(&asset, &raw);
        let projected_tokens = proj.estimated_tokens();
        let formatted_tokens = count_tokens(&proj.format());
        let saved = raw_tokens.saturating_sub(projected_tokens);

        println!(
            "  {label}: raw={raw_tokens} tokens, projected_est={projected_tokens}, \
             formatted={formatted_tokens}, saved={saved}"
        );

        if label != "small" {
            assert!(
                projected_tokens < raw_tokens,
                "{label}: projected ({projected_tokens}) must be < raw ({raw_tokens})"
            );
            assert!(saved > 0, "{label}: must save tokens (saved={saved})");
        }

        assert!(
            formatted_tokens <= projected_tokens + 50,
            "formatted token estimate ({formatted_tokens}) should be close to \
             estimated_tokens ({projected_tokens})"
        );
    }

    // Even for very large output, projection token estimate stays under a cap
    let very_large = generate_text_output(500_000);
    let (asset_lg, raw_lg) = make_test_asset(ProjectorKind::ReadFile, "Read", &very_large);
    let proj_lg = projectors.project(&asset_lg, &raw_lg);
    let proj_tokens_lg = proj_lg.estimated_tokens();

    let projection_token_cap = 2000;
    assert!(
        proj_tokens_lg < projection_token_cap,
        "projection tokens ({proj_tokens_lg}) exceed cap ({projection_token_cap})"
    );

    println!(
        "  very_large (500 KB): projected_tokens={proj_tokens_lg} (cap: {projection_token_cap})"
    );
    println!();
    println!("  ✓ Token budget benchmarks passed.");
}

// ============================================================================
// 9.8: Prompt-cache stability — deterministic manifest formatting
// ============================================================================

fn bench_9_8_prompt_cache_stability() {
    println!();
    println!("=== 9.8 Prompt-Cache Stability Benchmarks ===");
    println!();

    let projectors = ProjectorRegistry::new();

    let outputs: Vec<(&str, &str, ProjectorKind)> = vec![
        (
            "Read",
            "file line 1\nfile line 2\nfile line 3\n",
            ProjectorKind::ReadFile,
        ),
        (
            "Grep",
            "src/main.rs:10:fn main() {\nsrc/lib.rs:5:pub fn lib() {\n",
            ProjectorKind::Search,
        ),
        (
            "Bash",
            "Compiling crate...\nerror: compilation failed\n",
            ProjectorKind::ShellTest,
        ),
        (
            "Glob",
            "src/\nsrc/main.rs\nsrc/lib.rs\ntests/\n",
            ProjectorKind::DirectoryTree,
        ),
        (
            "mcp__test",
            r#"{"name": "test", "version": "1.0"}"#,
            ProjectorKind::JsonDefault,
        ),
        (
            "unknown",
            "some random text output\n",
            ProjectorKind::GenericText,
        ),
    ];

    for (tool_name, output, kind) in &outputs {
        let (asset, raw) = make_test_asset(*kind, tool_name, output);

        let formatted1 = projectors.project(&asset, &raw).format();
        let formatted2 = projectors.project(&asset, &raw).format();
        let formatted3 = projectors.project(&asset, &raw).format();

        assert_eq!(
            formatted1, formatted2,
            "{tool_name}: projection must be deterministic (run 1 vs 2)"
        );
        assert_eq!(
            formatted1, formatted3,
            "{tool_name}: projection must be deterministic (run 1 vs 3)"
        );

        let forbidden = [
            "/tmp/",
            "blob_path",
            "timestamp",
            "2026-01-01",
            "created_at",
        ];
        for fb in forbidden {
            assert!(
                !formatted1.contains(fb),
                "{tool_name}: projection must not contain '{fb}'"
            );
        }

        println!("  {tool_name}: deterministic ({}) bytes", formatted1.len());
    }

    // Same handle → same manifest bytes (cache-hit simulation)
    let (asset, raw) = make_test_asset(
        ProjectorKind::ShellTest,
        "Bash",
        "cache stability test output\nwith multiple lines\nfor verification\n",
    );

    let proj1 = projectors.project(&asset, &raw).format();
    let proj2 = projectors.project(&asset, &raw).format();

    assert_eq!(proj1, proj2, "cache-hit: same asset → same manifest bytes");
    println!("  cache-hit: byte-identical ({}) bytes ✓", proj1.len());

    println!();
    println!("  ✓ Prompt-cache stability benchmarks passed.");
}

// ============================================================================
// 9.9: No-negative-optimization — small/medium don't need extra recall
// ============================================================================

fn bench_9_9_no_negative_optimization() {
    println!();
    println!("=== 9.9 No-Negative-Optimization Benchmarks ===");
    println!();

    let projectors = ProjectorRegistry::new();

    // Small output (inline): projection contains full content
    let small = "short output\nwith two lines\n";
    let (asset_s, raw_s) = make_test_asset(ProjectorKind::ShellTest, "Bash", small);
    let proj_s = projectors.project(&asset_s, &raw_s).format();
    assert!(proj_s.contains("short output"), "small: content visible");
    assert!(
        proj_s.contains("with two lines"),
        "small: all lines visible"
    );

    // Medium output: excerpt contains enough for common decisions
    let mut medium = String::new();
    for i in 0..300 {
        medium.push_str(&format!("line {:03}: content for medium output test\n", i));
    }
    let (asset_m, raw_m) = make_test_asset(ProjectorKind::ShellTest, "Bash", &medium);
    let proj_m = projectors.project(&asset_m, &raw_m).format();
    assert!(proj_m.contains("Last"), "medium: tail excerpt present");
    assert!(proj_m.contains("line 299"), "medium: last lines visible");

    // Failing medium: error visible without recall
    let mut failing = medium.clone();
    failing.push_str("error: compilation failed due to type mismatch\n");
    let (bc_f, lc_f) = count_bytes_and_lines(&failing);
    let (mut asset_f, raw_f) = make_test_asset(ProjectorKind::ShellTest, "Bash", &failing);
    asset_f.success = false;
    asset_f.byte_count = bc_f;
    asset_f.line_count = lc_f;

    let proj_fail = projectors.project(&asset_f, &raw_f).format();
    assert!(proj_fail.contains("FAILED"), "failure: status visible");
    assert!(
        proj_fail.contains("error") || proj_fail.contains("Failure"),
        "failure: error visible in projection"
    );

    // Medium projection should be well under 500 tokens
    let proj_tokens = proj_m.len() / 4;
    assert!(
        proj_tokens < 500,
        "medium projection ({proj_tokens} tokens) exceeds 500 token budget"
    );

    println!("  small inline: content fully visible — no recall needed");
    println!(
        "  medium projected: tail excerpt + error visible — no recall needed for common decisions"
    );
    println!("  medium tokens: {proj_tokens} (bound: 500)");
    println!();
    println!("  ✓ No-negative-optimization benchmarks passed.");
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║    XiaoLin Output Asset Quality Gate Benchmarks             ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Benchmarks: 9.6, 9.7, 9.8, 9.9                             ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    bench_9_6_ingestion_and_indexing();
    bench_9_7_token_budget_accounting();
    bench_9_8_prompt_cache_stability();
    bench_9_9_no_negative_optimization();

    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  All quality gate benchmarks PASSED ✓                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
}
