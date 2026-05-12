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

    let mut block = String::with_capacity(1024);
    block.push_str("\n\n[Sub-Agent Delegation]\n");
    block.push_str(&format!(
        "You can delegate tasks to independent sub-agents via `spawn_subagent`. \
         Depth budget: {remaining}. Max parallel: {}.\n\n",
        ctx.policy.max_parallel,
    ));

    block.push_str(
        "\
WHEN TO DELEGATE (use sub-agents):
- 2+ independent sub-problems that benefit from parallel execution
- A subtask needs a different tool set (e.g. browser + code analysis simultaneously)
- Deep research or exploration while you continue reasoning
- Task complexity warrants dedicated focus in a separate context

WHEN NOT TO DELEGATE (use tools directly):
- Simple single-tool operations (just call the tool)
- Tasks needing your current conversation context (sub-agents start fresh)
- Sequential steps where each depends on the previous result
- When only 1 tool call would suffice

",
    );
    block.push_str(
        "WORKFLOW: `list_agents` → pick agent_id → `spawn_subagent`. \
         Batch multiple spawn calls in one response for parallel execution.\n\n",
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
        block.push_str("Agents:\n");
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
