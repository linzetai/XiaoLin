use xiaolin_core::agent_config::SubAgentPolicy;

pub(crate) fn memory_tool_suffix(agent_id: &str) -> String {
    agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}


/// Information needed to dynamically inject sub-agent guidance into the system prompt.
///
/// NOTE: active sub-agent status is intentionally NOT part of this struct. It is
/// per-turn dynamic content (`elapsed_ms` changes every turn) and would bust the
/// cacheable system prefix. Active status is injected separately into the last
/// user message via `build_active_runs_context` + `inject_user_context`
/// (prompt-cache D3 zero-pollution).
pub struct SubAgentPromptContext<'a> {
    pub policy: &'a SubAgentPolicy,
    pub available_agents: &'a [(String, Option<String>)],
    pub current_depth: u32,
}

/// Lightweight, per-turn snapshot of an in-flight sub-agent run, used only to
/// build the active-runs context injected into the parent's last user message.
///
/// All fields are live/ephemeral: they are recomputed each turn from the
/// `SubAgentManager` run table and never persisted. `elapsed_ms` and progress
/// counters change between turns, which is why this block is kept out of the
/// cacheable system prompt (see [`build_active_runs_context`]).
#[derive(Debug, Clone)]
pub struct ActiveRunSummary {
    /// Stable identifier of the running sub-agent.
    pub run_id: String,
    /// Sub-agent type/role (e.g. `explore`, `general`).
    pub subagent_type: String,
    /// Task prompt; truncated when rendered to bound token cost.
    pub task: String,
    /// Wall-clock time since spawn, derived live from `created_at` for running
    /// workers (their `SubAgentRun::elapsed_ms` is only set at completion).
    pub elapsed_ms: u64,
    /// Number of tool calls the worker has made so far (progress signal).
    pub tool_calls_made: u32,
    /// Name of the tool the worker is currently/most recently running, if any.
    pub current_tool: Option<String>,
}

/// Build the per-turn active sub-agent status block, injected into the last user
/// message (NOT the system prompt) to keep the cacheable system prefix stable.
///
/// Returns the inner text only (no `<system_context>` wrapper); the caller passes
/// it through `inject_user_context`, which adds the wrapper.
pub fn build_active_runs_context(active: &[ActiveRunSummary]) -> Option<String> {
    if active.is_empty() {
        return None;
    }
    let mut block = String::with_capacity(64 * active.len() + 32);
    block.push_str(&format!("[Active Sub-Agents: {}]\n", active.len()));
    for run in active {
        // Truncate long task descriptions to bound per-turn token cost. Char-based
        // (UTF-8 safe; byte slicing would panic on multibyte boundaries — rule #1).
        const MAX_TASK_CHARS: usize = 120;
        let task = if run.task.chars().count() > MAX_TASK_CHARS {
            let truncated: String = run.task.chars().take(MAX_TASK_CHARS).collect();
            format!("{truncated}…")
        } else {
            run.task.clone()
        };
        block.push_str(&format!(
            "- {} ({}): \"{}\" [{:.1}s elapsed, {} tool calls",
            run.run_id,
            run.subagent_type,
            task,
            run.elapsed_ms as f64 / 1000.0,
            run.tool_calls_made,
        ));
        if let Some(ref tool) = run.current_tool {
            block.push_str(&format!(", current: {tool}"));
        }
        block.push_str("]\n");
    }
    Some(block)
}

/// Build the dynamic sub-agent guidance block appended to the system message.
/// Returns `None` if sub-agents are disabled or depth budget is exhausted.
pub fn build_subagent_prompt_block(ctx: &SubAgentPromptContext<'_>) -> Option<String> {
    if !ctx.policy.enabled {
        return None;
    }
    let remaining = ctx.policy.max_depth.saturating_sub(ctx.current_depth);
    if remaining == 0 {
        return None;
    }

    if ctx.current_depth > 0 {
        return Some(build_child_agent_block(ctx, remaining));
    }

    let mut block = String::with_capacity(2048);
    block.push_str("\n\n[Sub-Agent Delegation — PRIORITY CAPABILITY]\n");
    block.push_str(&format!(
        "You have powerful sub-agent delegation via `spawn_subagent`. \
         Depth budget: {remaining}. Max parallel: {}.\n\n",
        ctx.policy.max_parallel,
    ));

    block.push_str(
        "\
⚡ DELEGATION IS YOUR SUPERPOWER — use it aggressively for parallelism:
- Spawn multiple sub-agents in ONE response for maximum parallelism
- You do NOT need to wait manually — the system automatically notifies you when sub-agents complete
- After spawning, you can continue reasoning or produce partial output; results arrive automatically

WHEN TO DELEGATE (strongly prefer delegation):
- 2+ independent sub-problems → spawn them ALL in parallel
- Research, exploration, or information gathering → delegate immediately
- A subtask needs focused attention in a separate context
- File reading, code analysis, or search tasks → perfect for sub-agents
- Any task that would take multiple sequential tool calls → parallelize via sub-agents

WHEN NOT TO DELEGATE (only these cases):
- Trivial single-tool operations (one quick tool call)
- Tasks requiring your current conversation context that cannot be summarized
- Sequential steps where EACH step depends on the PREVIOUS result with no parallelism

EXECUTION MODEL (Supervised Reactive Loop):
- You spawn sub-agents → system automatically waits for completions
- When ANY sub-agent completes, you receive a structured notification with results
- You then decide: spawn more tasks, reason about findings, or produce final response
- Your turn does NOT end until all sub-agents complete — no need to manage this yourself

CONCURRENCY:
- explore/research agents (concurrency_safe) run in parallel (read-lock)
- code/write agents run exclusively per session (write-lock)

",
    );

    if !ctx.policy.allowed_types.is_empty() {
        block.push_str(&format!(
            "Allowed types: {}.\n",
            ctx.policy.allowed_types.join(", "),
        ));
    } else {
        block.push_str("\
Types: general (full tools), explore (read-only research), shell (commands/builds), browser (web interaction).\n");
    }

    let delegatable: Vec<_> = ctx
        .available_agents
        .iter()
        .filter(|(id, _)| {
            ctx.policy.allowed_agents.is_empty()
                || ctx.policy.allowed_agents.iter().any(|a| a == id)
        })
        .collect();

    if !delegatable.is_empty() {
        block.push_str("\nAvailable Agents:\n");
        for (id, desc) in &delegatable {
            let d = desc.as_deref().unwrap_or("(no description)");
            block.push_str(&format!("- `{id}`: {d}\n"));
        }
    }

    block.push_str(
        "\n\
TASK DESCRIPTION RULES:
- Self-contained: include all needed context (sub-agent cannot see your conversation)
- Specific outcome: state exactly what to return
- One clear objective per sub-agent
",
    );

    if let Some(budget) = ctx.policy.token_budget {
        block.push_str(&format!("\nToken budget per sub-agent: {budget}.\n"));
    }

    // NOTE: active sub-agent status is NOT injected here. It is per-turn dynamic
    // content and would bust the cacheable system prefix. See
    // `build_active_runs_context` + `inject_user_context` (prompt-cache D1/D3).

    Some(block)
}

fn build_child_agent_block(ctx: &SubAgentPromptContext<'_>, remaining: u32) -> String {
    let mut block = String::with_capacity(256);
    block.push_str("\n\n[Sub-Agent Context]\n");
    block.push_str(
        "You are running as a sub-agent. Rules:\n\
         - Focus exclusively on your assigned task\n\
         - Return a concise, actionable result\n\
         - Do not engage in pleasantries or ask follow-up questions\n\
         - If you cannot complete the task, explain why clearly\n",
    );
    if remaining > 0 {
        block.push_str(&format!(
            "You may further delegate via `spawn_subagent` (remaining depth: {remaining}).\n",
        ));
    }
    if let Some(budget) = ctx.policy.token_budget {
        block.push_str(&format!("Token budget: {budget}.\n"));
    }
    block
}

pub(crate) const SKILL_MANAGEMENT_GUIDANCE: &str = "\n\n\
[Skill Management]\n\
When you successfully complete a complex, multi-step task:\n\
1. Consider if the approach could be reused. If so, use `write_skill` to save it as a reusable skill.\n\
2. If an existing skill was helpful but could be improved, use `read_skill` + `write_skill` to refine it.\n\
3. Good skill candidates: tasks with 3+ tool calls, recurring patterns, domain-specific workflows.\n\
4. Keep skills concise: task pattern, key steps, tool sequence, and any gotchas.\n\
Do NOT create skills for trivial single-step tasks or pure conversation.\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run(elapsed_ms: u64, tool_calls_made: u32) -> ActiveRunSummary {
        ActiveRunSummary {
            run_id: "run-1".into(),
            subagent_type: "explore".into(),
            task: "find the bug".into(),
            elapsed_ms,
            tool_calls_made,
            current_tool: None,
        }
    }

    /// D1/D2: delegation guidance must not embed any active-run status; the block
    /// is byte-identical across calls regardless of elapsed time changes.
    #[test]
    fn guidance_excludes_active_runs_and_is_byte_stable() {
        let policy = SubAgentPolicy::default();
        let agents: Vec<(String, Option<String>)> =
            vec![("explore".into(), Some("read-only research".into()))];
        let ctx = SubAgentPromptContext {
            policy: &policy,
            available_agents: &agents,
            current_depth: 0,
        };

        let a = build_subagent_prompt_block(&ctx).expect("guidance present");
        let b = build_subagent_prompt_block(&ctx).expect("guidance present");

        // Byte-stable across calls for the same agent config.
        assert_eq!(a, b);
        // No active-run status leaks into the system-prompt guidance.
        assert!(!a.contains("[Active Sub-Agents"));
        assert!(!a.contains("elapsed"));
    }

    /// D1: active-run status (with elapsed/progress) is produced by a separate
    /// builder destined for the user message, not the system prompt.
    #[test]
    fn active_runs_context_includes_progress() {
        let runs = vec![sample_run(1500, 3)];
        let ctx = build_active_runs_context(&runs).expect("context present");
        assert!(ctx.contains("[Active Sub-Agents: 1]"));
        assert!(ctx.contains("1.5s elapsed"));
        assert!(ctx.contains("3 tool calls"));
    }

    #[test]
    fn active_runs_context_empty_is_none() {
        assert!(build_active_runs_context(&[]).is_none());
    }

    /// Phase 2: the in-flight tool name is surfaced in the active-runs context so
    /// the parent can perceive real per-worker progress.
    #[test]
    fn active_runs_context_shows_current_tool() {
        let mut run = sample_run(2000, 4);
        run.current_tool = Some("grep".into());
        let ctx = build_active_runs_context(&[run]).expect("context present");
        assert!(ctx.contains("current: grep"));
    }

    /// Different elapsed values change only the user-context block, never the
    /// system-prompt guidance — proving the cache-pollution fix.
    #[test]
    fn elapsed_change_does_not_touch_guidance() {
        let policy = SubAgentPolicy::default();
        let agents: Vec<(String, Option<String>)> = vec![];
        let ctx = SubAgentPromptContext {
            policy: &policy,
            available_agents: &agents,
            current_depth: 0,
        };
        let guidance = build_subagent_prompt_block(&ctx).expect("guidance present");

        let c1 = build_active_runs_context(&[sample_run(1000, 1)]).unwrap();
        let c2 = build_active_runs_context(&[sample_run(9000, 5)]).unwrap();
        assert_ne!(c1, c2); // user-context block reflects fresh elapsed/progress
        // ...but guidance is untouched (recomputing yields the same bytes).
        assert_eq!(
            guidance,
            build_subagent_prompt_block(&ctx).expect("guidance present")
        );
    }
}
