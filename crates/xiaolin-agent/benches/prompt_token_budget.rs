//! Prompt token budget benchmark — validates that system prompt sizes stay
//! within budget across different configurations.
//!
//! Run with: `cargo bench -p xiaolin-agent --bench prompt_token_budget`
//!
//! This benchmark uses the 4-bytes-per-token heuristic (same as BYTES_PER_TOKEN).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use xiaolin_agent::prompt_sections::dynamic::{
    environment_section, frc_section, language_section, mcp_instructions_section, memory_section,
    session_guidance_section, token_budget_section,
};
use xiaolin_agent::prompt_sections::{
    actions_section, doing_tasks_section, intro_section, output_efficiency_section, system_section,
    tone_and_style_section, using_tools_section,
};
use xiaolin_agent::{McpServerInfo, PromptContext, PromptEngine};
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::types::ExecutionMode;

const BYTES_PER_TOKEN: usize = 4;

fn estimate_tokens(parts: &[String]) -> usize {
    let total_bytes: usize = parts.iter().map(|s| s.len()).sum();
    total_bytes / BYTES_PER_TOKEN
}

fn default_engine() -> PromptEngine {
    PromptEngine::new(
        vec![
            intro_section(),
            system_section(),
            doing_tasks_section(),
            actions_section(),
            using_tools_section(),
            tone_and_style_section(),
            output_efficiency_section(),
        ],
        vec![
            session_guidance_section(),
            environment_section(),
            memory_section(),
            language_section(),
            mcp_instructions_section(),
            token_budget_section(),
            frc_section(),
        ],
    )
}

const FULL_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "shell_exec",
    "search_in_files",
    "glob",
    "list_directory",
    "tool_search",
    "todo_write",
    "ask_question",
    "memory_store",
    "memory_search",
    "sessions_spawn",
    "task_create",
    "web_search",
    "web_fetch",
];

const MINIMAL_TOOLS: &[&str] = &["read_file", "search_in_files", "glob", "tool_search"];

fn make_ctx(mode: ExecutionMode, tools: &[&str], deferred: usize) -> PromptContext {
    PromptContext {
        agent_config: Arc::new(AgentConfig {
            agent_id: "main".into(),
            name: None,
            description: None,
            model: Default::default(),
            system_prompt: None,
            tools: vec![],
            behavior: Default::default(),
            mcp_servers: vec![],
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: Default::default(),
        }),
        enabled_tools: tools.iter().map(|s| s.to_string()).collect(),
        deferred_tool_count: deferred,
        model_id: "anthropic/claude-4-sonnet".into(),
        cwd: PathBuf::from("/home/user/project"),
        is_git: true,
        platform: "linux x86_64".into(),
        shell: "bash".into(),
        execution_mode: mode,
        mcp_servers: vec![],
        language_preference: None,
        token_budget: None,
        memory_prompt: None,
        session_start_date: "2026-04-29".into(),
        pending_todo_summary: None,
        plan_file_path: None,
        plan_file_exists: false,
    }
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         XiaoLin Prompt Token Budget Benchmark              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Heuristic: 1 token ≈ 4 bytes                              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let engine = default_engine();
    let mut all_pass = true;

    // --- 1. Static zone token count ---
    let static_engine = PromptEngine::new(
        vec![
            intro_section(),
            system_section(),
            doing_tasks_section(),
            actions_section(),
            using_tools_section(),
            tone_and_style_section(),
            output_efficiency_section(),
        ],
        vec![],
    );
    let ctx_static = make_ctx(ExecutionMode::Agent, FULL_TOOLS, 0);
    let static_parts = static_engine.build_system_prompt(&ctx_static);
    let static_tokens = estimate_tokens(&static_parts);

    let static_pass = static_tokens < 8000;
    println!(
        "  [{}] Static zone:           {:>5} tokens (limit: 8000)",
        if static_pass { "PASS" } else { "FAIL" },
        static_tokens
    );
    if !static_pass {
        all_pass = false;
    }

    // --- 2. Dynamic zone variation range ---
    let ctx_full = make_ctx(ExecutionMode::Agent, FULL_TOOLS, 10);
    let full_prompt = engine.build_system_prompt(&ctx_full);
    let full_tokens = estimate_tokens(&full_prompt);
    let dynamic_tokens = full_tokens.saturating_sub(static_tokens);

    // Dynamic zone is intentionally compact; guard against bloat beyond 5000
    let dynamic_pass = dynamic_tokens > 0 && dynamic_tokens <= 5000;
    println!(
        "  [{}] Dynamic zone:          {:>5} tokens (range: >0, <=5000)",
        if dynamic_pass { "PASS" } else { "FAIL" },
        dynamic_tokens
    );
    if !dynamic_pass {
        all_pass = false;
    }

    // --- 3. Full eager tools total ---
    let eager_pass = full_tokens < 15000;
    println!(
        "  [{}] Full eager total:      {:>5} tokens (limit: 15000)",
        if eager_pass { "PASS" } else { "FAIL" },
        full_tokens
    );
    if !eager_pass {
        all_pass = false;
    }

    // --- 4. Deferred mode savings ---
    engine.clear_cache();
    let ctx_deferred = make_ctx(ExecutionMode::Agent, MINIMAL_TOOLS, 12);
    let deferred_prompt = engine.build_system_prompt(&ctx_deferred);
    let deferred_tokens = estimate_tokens(&deferred_prompt);
    let savings_pct = if full_tokens > 0 {
        ((full_tokens - deferred_tokens) as f64 / full_tokens as f64) * 100.0
    } else {
        0.0
    };

    // Deferred must be smaller than eager, and total under budget
    let deferred_pass = deferred_tokens < 8000 && deferred_tokens < full_tokens;
    println!(
        "  [{}] Deferred mode:         {:>5} tokens (limit: 8000, savings: {:.1}%)",
        if deferred_pass { "PASS" } else { "FAIL" },
        deferred_tokens,
        savings_pct
    );
    if !deferred_pass {
        all_pass = false;
    }

    // --- 5. With MCP servers ---
    engine.clear_cache();
    let mut ctx_mcp = make_ctx(ExecutionMode::Agent, FULL_TOOLS, 0);
    ctx_mcp.mcp_servers = vec![
        McpServerInfo {
            id: "db-server".into(),
            instructions: Some("Query the database using SQL".into()),
        },
        McpServerInfo {
            id: "search-server".into(),
            instructions: Some("Full-text search API".into()),
        },
    ];
    let mcp_prompt = engine.build_system_prompt(&ctx_mcp);
    let mcp_tokens = estimate_tokens(&mcp_prompt);
    let mcp_pass = mcp_tokens < 15000;
    println!(
        "  [{}] With 2 MCP servers:    {:>5} tokens (limit: 15000)",
        if mcp_pass { "PASS" } else { "FAIL" },
        mcp_tokens
    );
    if !mcp_pass {
        all_pass = false;
    }

    // --- 6. Chinese language preference ---
    engine.clear_cache();
    let mut ctx_zh = make_ctx(ExecutionMode::Agent, FULL_TOOLS, 0);
    ctx_zh.language_preference = Some("zh-CN".into());
    let zh_prompt = engine.build_system_prompt(&ctx_zh);
    let zh_tokens = estimate_tokens(&zh_prompt);
    let zh_pass = zh_tokens < 15000;
    println!(
        "  [{}] Chinese language:      {:>5} tokens (limit: 15000)",
        if zh_pass { "PASS" } else { "FAIL" },
        zh_tokens
    );
    if !zh_pass {
        all_pass = false;
    }

    // --- Performance (assembly speed) ---
    println!();
    println!("  Performance:");
    engine.clear_cache();
    let ctx_perf = make_ctx(ExecutionMode::Agent, FULL_TOOLS, 10);
    let start = Instant::now();
    let iterations = 1000;
    for _ in 0..iterations {
        engine.clear_cache();
        let _ = engine.build_system_prompt(&ctx_perf);
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / iterations;
    println!(
        "    Cold assembly: {:?}/call ({} iterations)",
        per_call, iterations
    );

    let start = Instant::now();
    for _ in 0..iterations {
        let _ = engine.build_system_prompt(&ctx_perf);
    }
    let elapsed = start.elapsed();
    let per_call_hot = elapsed / iterations;
    println!(
        "    Hot  assembly: {:?}/call ({} iterations, cached)",
        per_call_hot, iterations
    );

    println!();
    if all_pass {
        println!("  ✓ All token budget checks passed.");
    } else {
        println!("  ✗ Some token budget checks FAILED!");
        std::process::exit(1);
    }
}
