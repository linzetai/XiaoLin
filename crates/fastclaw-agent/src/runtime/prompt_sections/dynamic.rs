//! Dynamic prompt sections that depend on per-session runtime state.
//!
//! These sections read `PromptContext` fields like `execution_mode`,
//! `enabled_tools`, `cwd`, etc. to produce context-aware prompt fragments.

use fastclaw_core::types::ExecutionMode;

use super::super::prompt_engine::{PromptContext, PromptSection};

/// Session-specific guidance based on enabled tools and execution mode.
///
/// Conditionally includes guidance blocks for sub-agents, plan mode,
/// ask-question tool, etc. depending on what is actually available.
///
/// Corresponds to Claude Code's `getSessionSpecificGuidanceSection()`.
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
        cache_break: false,
    }
}

fn has(ctx: &PromptContext, name: &str) -> bool {
    ctx.enabled_tools.contains(name)
}

fn session_guidance_en(ctx: &PromptContext) -> String {
    let mut parts = Vec::new();

    parts.push("<session_guidance>".to_string());

    if ctx.execution_mode == ExecutionMode::Plan {
        parts.push(
            "\
## Plan Mode (Read-Only)

You are currently in **Plan Mode**. In this mode:
- You can ONLY use read-only tools (read files, search, list directories, browse)
- You CANNOT write files, edit files, execute commands, or make any changes
- Focus on understanding, analyzing, and planning
- Create a detailed plan that can be executed when switching back to Agent mode
- Use `exit_plan_mode` when planning is complete and you're ready to implement"
                .to_string(),
        );
    }

    if has(ctx, "sessions_spawn") || has(ctx, "task_create") {
        parts.push(
            "\
## Sub-Agent / Task Delegation

You have access to sub-agent or task tools. Use them when:
- The task is complex and benefits from decomposition
- You need to run independent work streams in parallel
- A sub-task requires a different context or specialized focus

Do NOT delegate when:
- The task is simple enough to handle directly
- You need the result immediately in the current turn
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

    if has(ctx, "memory_store") || has(ctx, "memory_search") {
        parts.push(
            "\
## Memory

You have access to persistent memory. Search memory at the start of complex tasks \
to check for relevant context from previous sessions. Store important decisions, \
patterns, and user preferences for future reference."
                .to_string(),
        );
    }

    parts.push("</session_guidance>".to_string());

    parts.join("\n\n")
}

fn session_guidance_zh(ctx: &PromptContext) -> String {
    let mut parts = Vec::new();

    parts.push("<session_guidance>".to_string());

    if ctx.execution_mode == ExecutionMode::Plan {
        parts.push(
            "\
## 计划模式（只读）

你当前处于**计划模式**。在此模式下：
- 只能使用只读工具（读取文件、搜索、列出目录、浏览）
- 不能写入文件、编辑文件、执行命令或做任何修改
- 专注于理解、分析和规划
- 创建详细的计划，以便切换回 Agent 模式后执行
- 规划完成且准备实施时使用 `exit_plan_mode`"
                .to_string(),
        );
    }

    if has(ctx, "sessions_spawn") || has(ctx, "task_create") {
        parts.push(
            "\
## 子代理 / 任务委派

你可以使用子代理或任务工具。适用场景：
- 任务复杂，适合分解
- 需要并行运行独立的工作流
- 子任务需要不同的上下文或专业聚焦

不应委派的情况：
- 任务简单，可以直接处理
- 当前轮次需要立即获得结果
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

    if has(ctx, "memory_store") || has(ctx, "memory_search") {
        parts.push(
            "\
## 记忆

你可以使用持久化记忆。在复杂任务开始时搜索记忆，\
检查之前会话中的相关上下文。存储重要决策、模式和用户偏好以供将来参考。"
                .to_string(),
        );
    }

    parts.push("</session_guidance>".to_string());

    parts.join("\n\n")
}

/// Environment section: runtime context about the working environment.
///
/// Outputs: cwd, platform, shell, model, git status, knowledge cutoff,
/// session date, etc. This section is cacheable (computed once per session)
/// because the environment rarely changes mid-session.
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
        cache_break: false,
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
Model: {model}
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
模型：{model}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::prompt_engine::{McpServerInfo, PromptContext};
    use fastclaw_core::agent_config::AgentConfig;
    use fastclaw_core::types::ExecutionMode;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

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
        }
    }

    fn ctx_with_tools(lang: Option<&str>, tools: &[&str], mode: ExecutionMode) -> PromptContext {
        let mut ctx = base_ctx(lang);
        ctx.enabled_tools = tools.iter().map(|s| s.to_string()).collect();
        ctx.execution_mode = mode;
        ctx
    }

    #[test]
    fn session_guidance_plan_mode_en() {
        let ctx = ctx_with_tools(None, &[], ExecutionMode::Plan);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("Plan Mode"));
        assert!(text.contains("read-only"));
        assert!(text.contains("exit_plan_mode"));
    }

    #[test]
    fn session_guidance_plan_mode_zh() {
        let ctx = ctx_with_tools(Some("zh"), &[], ExecutionMode::Plan);
        let section = session_guidance_section();
        let text = (section.compute)(&ctx).unwrap();
        assert!(text.contains("计划模式"));
        assert!(text.contains("只读"));
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
        let ctx = ctx_with_tools(None, &["memory_search", "memory_store"], ExecutionMode::Agent);
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
    fn session_guidance_not_cache_break() {
        assert!(!session_guidance_section().cache_break);
    }

    #[test]
    fn environment_not_cache_break() {
        assert!(!environment_section().cache_break);
    }
}
