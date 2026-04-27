//! Heuristic complexity tier estimation from chat context.

use fastclaw_core::complexity::ComplexityTier;
use fastclaw_core::types::{ChatMessage, Role};

/// Inputs for [`estimate_complexity_tier`].
#[derive(Debug, Clone)]
pub struct TierEstimateInput<'a> {
    pub messages: &'a [ChatMessage],
    /// Number of tool definitions included in the LLM request (schema overhead).
    pub tool_definition_count: usize,
}

/// Rough token count (~4 chars per token).
fn token_len(text: &str) -> u32 {
    (text.len() as u32).div_ceil(4)
}

fn last_user_text(messages: &[ChatMessage]) -> Option<String> {
    for m in messages.iter().rev() {
        if matches!(m.role, Role::User) {
            if let Some(t) = m.text_content() {
                let t = t.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

fn has_tool_traffic(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|m| {
        m.tool_calls
            .as_ref()
            .map(|t| !t.is_empty())
            .unwrap_or(false)
            || m.tool_call_id.is_some()
    })
}

fn keyword_boost(text: &str) -> u8 {
    let lower = text.to_ascii_lowercase();
    let mut score: u8 = 0;
    const HEAVY: &[&str] = &[
        "formal proof",
        "theorem",
        "cryptograph",
        "zero-knowledge",
        "malware",
        "exploit",
        "penetration test",
        "architecture",
        "distributed system",
        "consensus",
        "formal verification",
        "llvm",
        "compiler",
        "kernel",
        "microservice",
        "kubernetes",
        "terraform",
        "multi-agent",
        "research synthesis",
        "literature review",
        "meta-analysis",
        "dataset",
        "statistical model",
        "bayesian",
        "optimization problem",
        "np-complete",
        "complexity class",
    ];
    const MEDIUM: &[&str] = &[
        "refactor",
        "debug",
        "stack trace",
        "race condition",
        "sql query",
        "migration",
        "openapi",
        "graphql",
        "pytest",
        "integration test",
        "threat model",
        "rfc",
        "compare and contrast",
        "trade-off",
        "evaluate",
        "design doc",
        "prd",
        "roadmap",
    ];
    for k in HEAVY {
        if lower.contains(k) {
            score = score.saturating_add(2);
        }
    }
    for k in MEDIUM {
        if lower.contains(k) {
            score = score.saturating_add(1);
        }
    }
    score.min(8)
}

/// Estimate workload tier from the latest user turn, history depth, tools, and keywords.
pub fn estimate_complexity_tier(input: TierEstimateInput<'_>) -> ComplexityTier {
    let n_msgs = input.messages.len();
    let user_text = last_user_text(input.messages);
    let user_tokens = user_text.as_deref().map(token_len).unwrap_or(0);
    let tool_heavy = input.tool_definition_count >= 12 || has_tool_traffic(input.messages);
    let kw = user_text.as_deref().map(keyword_boost).unwrap_or(0);

    let mut rank: u8 = 0;

    if user_tokens < 24 && n_msgs <= 2 && !tool_heavy && kw == 0 {
        return ComplexityTier::Tiny;
    }

    if user_tokens > 1800 {
        rank = rank.saturating_add(3);
    } else if user_tokens > 600 {
        rank = rank.saturating_add(2);
    } else if user_tokens > 200 {
        rank = rank.saturating_add(1);
    }

    if n_msgs > 24 {
        rank = rank.saturating_add(3);
    } else if n_msgs > 12 {
        rank = rank.saturating_add(2);
    } else if n_msgs > 6 {
        rank = rank.saturating_add(1);
    }

    if tool_heavy {
        rank = rank.saturating_add(2);
    } else if input.tool_definition_count >= 6 {
        rank = rank.saturating_add(1);
    }

    rank = rank.saturating_add(kw);

    match rank {
        0 => ComplexityTier::Small,
        1 => ComplexityTier::Small,
        2 => ComplexityTier::Medium,
        3 => ComplexityTier::Medium,
        4 => ComplexityTier::Large,
        _ => ComplexityTier::Frontier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::ChatMessage;

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(content.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn greeting_is_tiny() {
        let msgs = vec![user_msg("hi")];
        let t = estimate_complexity_tier(TierEstimateInput {
            messages: &msgs,
            tool_definition_count: 0,
        });
        assert_eq!(t, ComplexityTier::Tiny);
    }

    #[test]
    fn long_single_message_large() {
        let body = "word ".repeat(500);
        let msgs = vec![user_msg(&body)];
        let t = estimate_complexity_tier(TierEstimateInput {
            messages: &msgs,
            tool_definition_count: 0,
        });
        assert!(
            t >= ComplexityTier::Medium,
            "expected at least Medium, got {:?}",
            t
        );
    }

    #[test]
    fn tool_history_bumps_tier() {
        let msgs = vec![
            user_msg("run the build"),
            ChatMessage {
                role: Role::Assistant,
                content: None,
                name: None,
                tool_calls: Some(vec![fastclaw_core::types::ToolCall {
                    id: "1".into(),
                    call_type: "function".into(),
                    function: fastclaw_core::types::FunctionCall {
                        name: "shell".into(),
                        arguments: "{}".into(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some("ok".into()),
                name: None,
                tool_calls: None,
                tool_call_id: Some("1".into()),
            },
            user_msg("fix the error"),
        ];
        let t = estimate_complexity_tier(TierEstimateInput {
            messages: &msgs,
            tool_definition_count: 4,
        });
        assert!(t >= ComplexityTier::Medium);
    }

    #[test]
    fn heavy_keywords_frontier() {
        let msgs = vec![user_msg(
            "Write a formal proof sketch for zero-knowledge soundness in this protocol, \
             including an np-complete reduction outline.",
        )];
        let t = estimate_complexity_tier(TierEstimateInput {
            messages: &msgs,
            tool_definition_count: 0,
        });
        assert_eq!(t, ComplexityTier::Frontier);
    }
}
