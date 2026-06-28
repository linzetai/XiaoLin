//! Dynamic prompt sections that depend on per-session runtime state.
//!
//! These sections read `PromptContext` fields like `execution_mode`,
//! `enabled_tools`, `cwd`, etc. to produce context-aware prompt fragments.

use xiaolin_core::types::ExecutionMode;

use super::super::prompt_engine::{PromptContext, PromptSection};

/// Session-specific guidance based on enabled tools and execution mode.
///
/// Conditionally includes guidance blocks for sub-agents, plan mode,
/// ask-question tool, etc. depending on what is actually available.
///
/// Corresponds to Claude Code's `getSessionSpecificGuidanceSection()`.
///
/// `cache_break: true`: this is per-session content (mode / plan state / todo),
/// so it is recomputed deterministically every turn. This keeps the output
/// byte-stable when the context is unchanged (so the provider prompt cache
/// still hits) while avoiding cross-session pollution of the global section
/// cache and auto-reflecting plan-mode/mode switches without explicit
/// invalidation. See `docs/` cache design notes.
pub fn session_guidance_section() -> PromptSection {
    PromptSection {
        name: "session_guidance",
        compute: Box::new(|ctx| {
            let lang = ctx.language_preference.as_deref();
            Some(match lang {
                Some("zh" | "zh-CN" | "zh-TW") => session_guidance_zh(ctx),
                _ => session_guidance_en(ctx),
            })
        }),
        cache_break: true,
    }
}

fn has(ctx: &PromptContext, name: &str) -> bool {
    ctx.enabled_tools.contains(name)
}

fn session_guidance_en(ctx: &PromptContext) -> String {
    let mut parts = Vec::new();

    parts.push("<session_guidance>".to_string());

    if ctx.execution_mode == ExecutionMode::Agent {
        if let Some(ref path) = ctx.plan_file_path {
            if ctx.plan_file_exists {
                parts.push(format!(
                    "\
## Active Plan Reference

A plan file exists from a previous planning phase at `{path}`. \
If this plan is relevant to the current work and not already complete, \
continue working on it. You can read this file to review the plan details."
                ));
            }
        }
    }

    if has(ctx, "sessions_spawn") || has(ctx, "task_create") {
        parts.push(
            "\
## Sub-Agent / Task Delegation

You have access to sub-agent tools for parallel task execution:
- `spawn_subagent` — fire-and-forget: spawns a child agent and returns a `run_id` immediately
- `subagent_get` — check the status/result of a previously spawned sub-agent by `run_id`
- `subagent_list` — list all sub-agent runs and their statuses

**Parallel execution pattern**: You can call `spawn_subagent` multiple times in a single response \
to launch several sub-agents concurrently. Continue with your own work while they run. \
Use `subagent_get` later to retrieve their results when needed.

Use sub-agents when:
- The task is complex and benefits from decomposition
- You need to run independent work streams in parallel
- A sub-task requires a different context or specialized focus

Do NOT delegate when:
- The task is simple enough to handle directly
- The overhead of delegation outweighs the benefit"
                .to_string(),
        );
    }

    if has(ctx, "ask_question") {
        parts.push(
            "\
## Asking the User

Use the ask_question tool when you need clarification, not as a crutch. \
Exhaust code analysis and context clues first. Ask specific questions \
with concrete options rather than open-ended ones."
                .to_string(),
        );
    }

    if has(ctx, "todo_write") {
        if let Some(ref summary) = ctx.pending_todo_summary {
            parts.push(format!(
                "\
## Implementation Checklist (from Plan Mode)

You have pending tasks from a previous planning phase. Follow this checklist:

{summary}

Work through these items systematically:
1. Mark the current task as `in_progress` using `todo_write`
2. Implement it fully
3. Mark it `completed` and move to the next `pending` task
4. Continue until all tasks are done"
            ));
        } else {
            parts.push(
                "\
## Task Management

Use todo_write for complex multi-step tasks (3+ steps). \
Skip it for simple tasks that need only 1-2 actions. \
Keep exactly one task in_progress at a time. \
Mark tasks complete immediately upon finishing."
                    .to_string(),
            );
        }
    }

    if has(ctx, "memory_store") || has(ctx, "memory_search") {
        parts.push(
            "\
## Memory

You have access to persistent memory. Search memory at the start of complex tasks \
to check for relevant context from previous sessions. Store important decisions, \
patterns, and user preferences for future reference.

IMPORTANT: When the user explicitly asks you to \"remember\", \"memorize\", \"note down\", \
\"store for later\", or similar phrases indicating they want information persisted across \
sessions, you MUST call memory_store immediately — do NOT just verbally acknowledge. \
The user expects durable persistence, not a one-turn acknowledgment."
                .to_string(),
        );
    }

    parts.push("</session_guidance>".to_string());

    parts.join("\n\n")
}

fn session_guidance_zh(ctx: &PromptContext) -> String {
    let mut parts = Vec::new();

    parts.push("<session_guidance>".to_string());

    if ctx.execution_mode == ExecutionMode::Agent {
        if let Some(ref path) = ctx.plan_file_path {
            if ctx.plan_file_exists {
                parts.push(format!(
                    "\
## 活跃计划引用

上次计划阶段产生的计划文件位于 `{path}`。\
如果该计划与当前工作相关且尚未完成，请继续执行。\
你可以读取此文件查看计划详情。"
                ));
            }
        }
    }

    if has(ctx, "sessions_spawn") || has(ctx, "task_create") {
        parts.push(
            "\
## 子代理 / 任务委派

你可以使用子代理工具进行并行任务执行：
- `spawn_subagent` — 即发即忘：启动子代理后立即返回 `run_id`
- `subagent_get` — 通过 `run_id` 查询子代理的状态和结果
- `subagent_list` — 列出所有子代理运行及其状态

**并行执行模式**：你可以在一次回复中多次调用 `spawn_subagent` 来同时启动多个子代理。\
在它们运行期间继续做你自己的工作，需要结果时再用 `subagent_get` 获取。

适用场景：
- 任务复杂，适合分解
- 需要并行运行独立的工作流
- 子任务需要不同的上下文或专业聚焦

不应委派的情况：
- 任务简单，可以直接处理
- 委派的开销大于收益"
                .to_string(),
        );
    }

    if has(ctx, "ask_question") {
        parts.push(
            "\
## 向用户提问

需要澄清时才使用 ask_question 工具，而不是作为依赖。\
先充分利用代码分析和上下文线索。提出具体的问题并给出明确选项，\
而非开放式提问。"
                .to_string(),
        );
    }

    if has(ctx, "todo_write") {
        if let Some(ref summary) = ctx.pending_todo_summary {
            parts.push(format!(
                "\
## 实施清单（来自计划模式）

你有来自之前计划阶段的待办任务，请按照此清单执行：

{summary}

系统化地完成这些任务：
1. 使用 `todo_write` 将当前任务标记为 `in_progress`
2. 完整实施该任务
3. 标记为 `completed`，然后处理下一个 `pending` 任务
4. 持续直到所有任务完成"
            ));
        } else {
            parts.push(
                "\
## 任务管理

复杂的多步骤任务（3+ 步）使用 todo_write。\
简单的 1-2 步任务跳过它。\
同时只保持一个任务为 in_progress。\
完成后立即标记为 complete。"
                    .to_string(),
            );
        }
    }

    if has(ctx, "memory_store") || has(ctx, "memory_search") {
        parts.push(
            "\
## 记忆

你可以使用持久化记忆。在复杂任务开始时搜索记忆，\
检查之前会话中的相关上下文。存储重要决策、模式和用户偏好以供将来参考。

重要：当用户明确说「记住」「记下」「帮我存一下」「以后别忘了」等表示要跨会话持久化信息的指令时，\
你必须立即调用 memory_store 工具 - 不能只是口头确认。用户期望的是持久化存储，而非一轮对话内的临时记忆。"
                .to_string(),
        );
    }

    parts.push("</session_guidance>".to_string());

    parts.join("\n\n")
}

/// Environment section: runtime context about the working environment.
///
/// Outputs: cwd, platform, shell, model, git status, knowledge cutoff,
/// session date, etc.
///
/// `cache_break: true`: this is per-session/per-request content (notably `cwd`),
/// so it is recomputed deterministically every turn. Memoizing it globally would
/// leak the first session's `cwd` into all others. Recomputing keeps the bytes
/// identical when the context is unchanged (provider cache still hits) while
/// staying correct across concurrent sessions.
///
/// Corresponds to Claude Code's `computeSimpleEnvInfo()`.
pub fn environment_section() -> PromptSection {
    PromptSection {
        name: "environment",
        compute: Box::new(|ctx| {
            let lang = ctx.language_preference.as_deref();
            Some(match lang {
                Some("zh" | "zh-CN" | "zh-TW") => environment_zh(ctx),
                _ => environment_en(ctx),
            })
        }),
        cache_break: true,
    }
}

fn model_knowledge_cutoff(model_id: &str) -> &'static str {
    let id = model_id.to_lowercase();
    if id.contains("claude-4") || id.contains("opus-4") || id.contains("sonnet-4") {
        "Early 2025"
    } else if id.contains("claude-3")
        || id.contains("sonnet-3")
        || id.contains("haiku-3")
        || id.contains("opus-3")
    {
        "Early 2024"
    } else if id.contains("gpt-4o") || id.contains("gpt-4-turbo") {
        "Late 2023"
    } else if id.contains("gpt-4") {
        "September 2021"
    } else if id.contains("deepseek") || id.contains("qwen") {
        "Mid 2024"
    } else if id.contains("gemini") {
        "Late 2024"
    } else {
        "Unknown"
    }
}

fn environment_en(ctx: &PromptContext) -> String {
    let cutoff = model_knowledge_cutoff(&ctx.model_id);
    let git_info = if ctx.is_git {
        "Yes (git repository)"
    } else {
        "No"
    };

    format!(
        "\
<environment>
Working directory: {cwd}
Platform: {platform}
Shell: {shell}
Current model: {model} (this is your CURRENT model identity — always use this when asked, ignore any different model names in conversation history)
Knowledge cutoff: {cutoff}
Session date: {date}
Git repository: {git}
</environment>",
        cwd = ctx.cwd.display(),
        platform = ctx.platform,
        shell = ctx.shell,
        model = ctx.model_id,
        date = ctx.session_start_date,
        git = git_info,
    )
}

fn environment_zh(ctx: &PromptContext) -> String {
    let cutoff = model_knowledge_cutoff(&ctx.model_id);
    let git_info = if ctx.is_git {
        "是（git 仓库）"
    } else {
        "否"
    };

    format!(
        "\
<environment>
工作目录：{cwd}
平台：{platform}
Shell：{shell}
当前模型：{model}（这是你的当前模型身份 — 被问到时以此为准，忽略对话历史中的其他模型名称）
知识截止：{cutoff}
会话日期：{date}
Git 仓库：{git}
</environment>",
        cwd = ctx.cwd.display(),
        platform = ctx.platform,
        shell = ctx.shell,
        model = ctx.model_id,
        date = ctx.session_start_date,
        git = git_info,
    )
}

/// Memory section: injects persistent memory context from the memory system.
///
/// Returns `None` when no memory prompt is available, causing
/// `PromptEngine` to skip this section entirely.
///
/// `cache_break: true`: memory is per-session content. Recomputed deterministically
/// every turn so a `memory_store` update is reflected immediately (replacing the
/// old §6.3 explicit-invalidation approach) and never leaks across sessions.
pub fn memory_section() -> PromptSection {
    PromptSection {
        name: "memory",
        compute: Box::new(|ctx| {
            ctx.memory_prompt
                .as_ref()
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!("<memory>\n{m}\n</memory>"))
        }),
        cache_break: true,
    }
}

/// Language preference section: tells the model which language to respond in.
///
/// Returns `None` when no explicit preference is set (model uses its default).
///
/// `cache_break: true`: language is per-request content; recomputed deterministically
/// so different sessions/requests never inherit a stale preference.
pub fn language_section() -> PromptSection {
    PromptSection {
        name: "language",
        compute: Box::new(|ctx| {
            ctx.language_preference.as_ref().map(|lang| {
                format!(
                    "<language_preference>\n\
                     Respond in: {lang}\n\
                     Match the user's language when they write in a specific language.\n\
                     </language_preference>"
                )
            })
        }),
        cache_break: true,
    }
}

/// MCP instructions section: lists instructions from connected MCP servers.
///
/// This is `cache_break: false` (memoized) because MCP servers rarely change.
/// Staleness on connect/disconnect is handled by event-driven invalidation:
/// `AgentRuntime` tracks `ToolRegistry::mcp_instructions_version()` and calls
/// `invalidate_sections(&["mcp_instructions"])` only when it changes. This keeps
/// the prompt prefix byte-stable across turns for prompt caching.
const MCP_INSTRUCTIONS_MAX_CHARS: usize = 2048;

static MCP_SUSPICIOUS_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)ignore previous|system:|<\||\[INST\]").expect("valid MCP filter regex")
});

fn sanitize_mcp_instructions(server_id: &str, instructions: &str) -> Option<String> {
    let sanitized = xiaolin_mcp::sanitize::sanitize_unicode(instructions.trim());
    if MCP_SUSPICIOUS_RE.is_match(&sanitized) {
        tracing::warn!(
            server_id = %server_id,
            "blocking MCP server instructions: suspicious prompt injection pattern detected"
        );
        return None;
    }
    let truncated: String = sanitized.chars().take(MCP_INSTRUCTIONS_MAX_CHARS).collect();
    if truncated.is_empty() {
        None
    } else {
        Some(truncated)
    }
}

pub fn mcp_instructions_section() -> PromptSection {
    PromptSection {
        name: "mcp_instructions",
        compute: Box::new(|ctx| {
            let servers_with_instructions: Vec<_> = ctx
                .mcp_servers
                .iter()
                .filter_map(|s| {
                    s.instructions.as_ref().and_then(|i| {
                        sanitize_mcp_instructions(&s.id, i).map(|clean| (s.id.as_str(), clean))
                    })
                })
                .collect();

            if servers_with_instructions.is_empty() {
                return None;
            }

            let mut parts = vec!["<mcp_instructions>".to_string()];
            for (id, instructions) in servers_with_instructions {
                parts.push(format!("## MCP Server: {id}\n\n{instructions}"));
            }
            parts.push("</mcp_instructions>".to_string());
            Some(parts.join("\n\n"))
        }),
        cache_break: false,
    }
}

/// Token budget section: guidance on response token budget when enabled.
///
/// Returns `None` when no budget is set, letting the model use full capacity.
pub fn token_budget_section() -> PromptSection {
    PromptSection {
        name: "token_budget",
        compute: Box::new(|ctx| {
            ctx.token_budget.map(|budget| {
                let lang = ctx.language_preference.as_deref();
                match lang {
                    Some("zh" | "zh-CN" | "zh-TW") => format!(
                        "<token_budget>\n\
                         你的回复 token 预算约为 {budget} tokens。\n\
                         优先保证完整性和正确性，但注意控制回复长度。\n\
                         如果任务需要更多空间，可以超出预算，但应尽量精简。\n\
                         </token_budget>"
                    ),
                    _ => format!(
                        "<token_budget>\n\
                         Your response token budget is approximately {budget} tokens.\n\
                         Prioritize completeness and correctness, but be mindful of length.\n\
                         You may exceed the budget if the task requires it, but aim to be concise.\n\
                         </token_budget>"
                    ),
                }
            })
        }),
        // Per-request budget: recompute deterministically each turn.
        cache_break: true,
    }
}

/// Code context section: injects structural context from recently-read files.
///
/// Recomputed every turn (`cache_break: true`) because the code graph
/// changes as the agent reads new files. Returns `None` when no files
/// have been read yet this session.
pub fn code_context_section() -> PromptSection {
    PromptSection {
        name: "code_context",
        compute: Box::new(|_ctx| {
            crate::code_graph::CodeGraphCache::global().format_for_prompt(2000)
        }),
        cache_break: true,
    }
}

/// Function result clearing section: informs the model that old tool results
/// may be automatically compacted or cleared to save context space.
/// When output handles (like "out_...") are available, the model should prefer
/// recall tools (output_read, output_search, output_tail, output_summary)
/// over re-running expensive commands.
pub fn frc_section() -> PromptSection {
    PromptSection {
        name: "frc",
        compute: Box::new(|ctx| {
            let lang = ctx.language_preference.as_deref();
            Some(match lang {
                Some("zh" | "zh-CN" | "zh-TW") => "\
<function_result_clearing>\n\
旧的工具调用结果可能被自动压缩或替换为摘要，以节省上下文空间。\n\
如果你需要之前工具调用的精确内容，当输出句柄（如 out_...）可用时，\n\
优先使用 output_read、output_search 或 output_tail 获取精确内容，\n\
而不是重新运行昂贵的命令。被清理的结果会显示为简短的摘要标记。\n\
</function_result_clearing>"
                    .to_string(),
                _ => "\
<function_result_clearing>\n\
Old tool call results may be automatically compacted or replaced with summaries \n\
to save context space. When an output handle (e.g., \"out_...\") is available, \n\
prefer using output_read, output_search, or output_tail to recover exact content \n\
instead of re-running expensive commands. Only re-run the tool if no handle is \n\
available or you need fundamentally different output. Cleared results appear as \n\
short summary markers.\n\
</function_result_clearing>"
                    .to_string(),
            })
        }),
        cache_break: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::prompt_engine::{McpServerInfo, PromptContext};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use xiaolin_core::agent_config::AgentConfig;
    use xiaolin_core::types::ExecutionMode;

    fn base_ctx(lang: Option<&str>) -> PromptContext {
        PromptContext {
            agent_config: Arc::new(AgentConfig {
                agent_id: "test".into(),
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
            enabled_tools: HashSet::new(),
            deferred_tool_count: 0,
            model_id: "claude-4-sonnet".into(),
            cwd: PathBuf::from("/home/user/project"),
            is_git: true,
            platform: "linux x86_64".into(),
            shell: "bash".into(),
            execution_mode: ExecutionMode::Agent,
            mcp_servers: vec![],
            language_preference: lang.map(String::from),
            token_budget: None,
            memory_prompt: None,
            session_start_date: "2026-04-29".into(),
            pending_todo_summary: None,
            plan_file_path: None,
            plan_file_exists: false,
            system_base_prompt: None,
        }
    }

    fn ctx_with_tools(lang: Option<&str>, tools: &[&str], mode: ExecutionMode) -> PromptContext {
        let mut ctx = base_ctx(lang);
        ctx.enabled_tools = tools.iter().map(|s| s.to_string()).collect();
        ctx.execution_mode = mode;
        ctx
    }

    #[test]
    fn session_guidance_plan_mode_no_longer_in_system_prompt() {
        let ctx = ctx_with_tools(None, &[], ExecutionMode::Plan);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(
            !text.contains("Plan Mode (Read-Only)"),
            "Plan mode instructions moved to per-turn attachments"
        );
    }

    #[test]
    fn session_guidance_plan_mode_zh_no_longer_in_system_prompt() {
        let ctx = ctx_with_tools(Some("zh"), &[], ExecutionMode::Plan);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(
            !text.contains("计划模式（只读）"),
            "Plan mode instructions moved to per-turn attachments"
        );
    }

    #[test]
    fn session_guidance_agent_mode_no_plan_block() {
        let ctx = ctx_with_tools(None, &[], ExecutionMode::Agent);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(!text.contains("Plan Mode"));
    }

    #[test]
    fn session_guidance_includes_subagent_when_available() {
        let ctx = ctx_with_tools(None, &["task_create"], ExecutionMode::Agent);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Sub-Agent"));
        assert!(text.contains("delegation"));
    }

    #[test]
    fn session_guidance_excludes_subagent_when_unavailable() {
        let ctx = ctx_with_tools(None, &["read_file"], ExecutionMode::Agent);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(!text.contains("Sub-Agent"));
    }

    #[test]
    fn session_guidance_includes_ask_question() {
        let ctx = ctx_with_tools(None, &["ask_question"], ExecutionMode::Agent);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("ask_question"));
    }

    #[test]
    fn session_guidance_includes_todo() {
        let ctx = ctx_with_tools(None, &["todo_write"], ExecutionMode::Agent);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("todo_write"));
    }

    #[test]
    fn session_guidance_includes_memory() {
        let ctx = ctx_with_tools(
            None,
            &["memory_search", "memory_store"],
            ExecutionMode::Agent,
        );
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Memory") || text.contains("memory"));
    }

    #[test]
    fn environment_en_outputs_all_fields() {
        let section = environment_section();
        let ctx = base_ctx(None);
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("/home/user/project"));
        assert!(text.contains("linux x86_64"));
        assert!(text.contains("bash"));
        assert!(text.contains("claude-4-sonnet"));
        assert!(text.contains("2026-04-29"));
        assert!(text.contains("git repository"));
    }

    #[test]
    fn environment_zh_outputs_all_fields() {
        let section = environment_section();
        let ctx = base_ctx(Some("zh-CN"));
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("/home/user/project"));
        assert!(text.contains("linux x86_64"));
        assert!(text.contains("bash"));
        assert!(text.contains("git 仓库"));
    }

    #[test]
    fn environment_includes_knowledge_cutoff() {
        let section = environment_section();
        let ctx = base_ctx(None);
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Early 2025"));
    }

    #[test]
    fn environment_cutoff_gpt4o() {
        let mut ctx = base_ctx(None);
        ctx.model_id = "gpt-4o-2024-08-06".into();
        let section = environment_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Late 2023"));
    }

    #[test]
    fn environment_cutoff_unknown() {
        let mut ctx = base_ctx(None);
        ctx.model_id = "custom-local-model".into();
        let section = environment_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Unknown"));
    }

    #[test]
    fn environment_no_git() {
        let mut ctx = base_ctx(None);
        ctx.is_git = false;
        let section = environment_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("No"));
        assert!(!text.contains("git repository"));
    }

    #[test]
    fn session_guidance_recomputes_per_turn() {
        // Per-session content (mode/plan/todo) → recomputed deterministically each
        // turn to avoid cross-session pollution and auto-reflect mode switches.
        assert!(session_guidance_section().cache_break);
    }

    #[test]
    fn environment_recomputes_per_turn() {
        // Contains per-request cwd → must not be globally memoized.
        assert!(environment_section().cache_break);
    }

    // ── memory section ──────────────────────────────────────────

    #[test]
    fn memory_returns_content_when_present() {
        let mut ctx = base_ctx(None);
        ctx.memory_prompt = Some("User prefers Rust. Last session: fixed auth bug.".into());
        let section = memory_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("prefers Rust"));
        assert!(text.contains("<memory>"));
    }

    #[test]
    fn memory_returns_none_when_absent() {
        let ctx = base_ctx(None);
        let section = memory_section();
        assert!((section.compute)(&ctx).is_none());
    }

    #[test]
    fn memory_returns_none_when_empty() {
        let mut ctx = base_ctx(None);
        ctx.memory_prompt = Some("  ".into());
        let section = memory_section();
        assert!((section.compute)(&ctx).is_none());
    }

    // ── language section ────────────────────────────────────────

    #[test]
    fn language_returns_preference_when_set() {
        let ctx = base_ctx(Some("zh-CN"));
        let section = language_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("zh-CN"));
        assert!(text.contains("language_preference"));
    }

    #[test]
    fn language_returns_none_when_unset() {
        let ctx = base_ctx(None);
        let section = language_section();
        assert!((section.compute)(&ctx).is_none());
    }

    // ── mcp_instructions section ────────────────────────────────

    #[test]
    fn mcp_instructions_lists_servers() {
        let mut ctx = base_ctx(None);
        ctx.mcp_servers = vec![
            McpServerInfo {
                id: "git-server".into(),
                instructions: Some("Use for git operations".into()),
            },
            McpServerInfo {
                id: "no-inst".into(),
                instructions: None,
            },
            McpServerInfo {
                id: "db-server".into(),
                instructions: Some("Query the database".into()),
            },
        ];
        let section = mcp_instructions_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("git-server"));
        assert!(text.contains("Use for git operations"));
        assert!(text.contains("db-server"));
        assert!(!text.contains("no-inst"));
    }

    #[test]
    fn mcp_instructions_returns_none_when_no_instructions() {
        let mut ctx = base_ctx(None);
        ctx.mcp_servers = vec![McpServerInfo {
            id: "empty".into(),
            instructions: None,
        }];
        let section = mcp_instructions_section();
        assert!((section.compute)(&ctx).is_none());
    }

    #[test]
    fn mcp_instructions_is_memoized() {
        // Memoized for prefix stability; staleness handled by event-driven
        // invalidation keyed on ToolRegistry::mcp_instructions_version().
        assert!(!mcp_instructions_section().cache_break);
    }

    // ── token_budget section ────────────────────────────────────

    #[test]
    fn token_budget_returns_when_set() {
        let mut ctx = base_ctx(None);
        ctx.token_budget = Some(4096);
        let section = token_budget_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("4096"));
        assert!(text.contains("token_budget"));
    }

    #[test]
    fn token_budget_returns_none_when_unset() {
        let ctx = base_ctx(None);
        let section = token_budget_section();
        assert!((section.compute)(&ctx).is_none());
    }

    #[test]
    fn token_budget_zh() {
        let mut ctx = base_ctx(Some("zh"));
        ctx.token_budget = Some(8192);
        let section = token_budget_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("8192"));
        assert!(text.contains("预算"));
    }

    // ── frc section ─────────────────────────────────────────────

    #[test]
    fn frc_en_mentions_clearing() {
        let section = frc_section();
        let ctx = base_ctx(None);
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("compacted"));
        assert!(text.contains("output_read"));
        assert!(text.contains("output_search"));
    }

    #[test]
    fn frc_zh_mentions_clearing() {
        let section = frc_section();
        let ctx = base_ctx(Some("zh"));
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("压缩"));
        assert!(text.contains("output_read"));
    }

    #[test]
    fn frc_not_cache_break() {
        assert!(!frc_section().cache_break);
    }
}
