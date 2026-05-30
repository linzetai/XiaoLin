use fastclaw_core::agent_config::SubAgentPolicy;
use fastclaw_core::types::{ChatMessage, Role};

use super::trajectory::append_text_to_chat_content;

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

#[allow(dead_code)]
pub(crate) fn append_subagent_prompt_to_system(messages: &mut [ChatMessage], block: &str) {
    if let Some(sys) = messages.first_mut().filter(|m| m.role == Role::System) {
        append_text_to_chat_content(&mut sys.content, block);
    }
}

/// Information needed to dynamically inject sub-agent guidance into the system prompt.
pub struct SubAgentPromptContext<'a> {
    pub policy: &'a SubAgentPolicy,
    pub available_agents: &'a [(String, Option<String>)],
    pub current_depth: u32,
    /// Currently active sub-agent runs for this session (for status injection).
    pub active_runs: Option<&'a [ActiveRunSummary]>,
}

/// Lightweight summary of an active sub-agent run for prompt injection.
#[derive(Debug, Clone)]
pub struct ActiveRunSummary {
    pub run_id: String,
    pub subagent_type: String,
    pub task: String,
    pub elapsed_ms: u64,
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

    // Inject current active run status if any.
    if let Some(active) = ctx.active_runs {
        if !active.is_empty() {
            block.push_str(&format!(
                "\n[Active Sub-Agents: {}]\n",
                active.len()
            ));
            for run in active {
                block.push_str(&format!(
                    "- {} ({}): \"{}\" [{:.1}s elapsed]\n",
                    run.run_id,
                    run.subagent_type,
                    run.task,
                    run.elapsed_ms as f64 / 1000.0,
                ));
            }
        }
    }

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
