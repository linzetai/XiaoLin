use fastclaw_context::compressor::{
    estimate_messages_tokens, CompactionStrategy, ContextCompactor,
};
use fastclaw_context::engine::ContextEngine;
use fastclaw_core::types::{ChatMessage, Role};

fn make_system_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::System,
        content: Some(serde_json::Value::String(text.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn make_user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(text.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn make_assistant_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        content: Some(serde_json::Value::String(text.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn build_long_conversation(system_prompt: &str, turns: usize) -> Vec<ChatMessage> {
    let mut msgs = vec![make_system_msg(system_prompt)];
    for i in 1..=turns {
        msgs.push(make_user_msg(&format!(
            "Turn {i}: Please help me analyze the performance characteristics of the \
             distributed system component that handles request routing, load balancing, \
             and failover. Include details about latency percentiles, throughput metrics, \
             and resource utilization patterns under various load conditions."
        )));
        msgs.push(make_assistant_msg(&format!(
            "Turn {i} response: The distributed system component you're asking about uses \
             a multi-tier architecture with consistent hashing for request routing. Under \
             normal load (p50), latency is approximately 12ms with throughput of 15k req/s. \
             At p99, latency rises to 45ms. The failover mechanism uses a gossip protocol \
             with a detection window of 3 seconds. Resource utilization typically sits at \
             60% CPU and 4GB memory per node during peak hours."
        )));
    }
    msgs
}

const CONTEXT_WINDOW: u32 = 8192;

#[test]
fn long_session_200_turns_stays_within_context_window() {
    let system_prompt = "You are a senior systems engineer. Always provide detailed, \
                         quantitative analysis with specific metrics and recommendations.";
    let mut messages = build_long_conversation(system_prompt, 200);

    let raw_tokens = estimate_messages_tokens(&messages);
    assert!(
        raw_tokens > CONTEXT_WINDOW as usize,
        "200 turns should exceed context window before compaction (raw={raw_tokens})"
    );

    let final_tokens =
        ContextEngine::fit_to_context_window(&mut messages, CONTEXT_WINDOW, None);

    let budget = CONTEXT_WINDOW - CONTEXT_WINDOW / 4;
    assert!(
        final_tokens <= budget as usize,
        "After fit_to_context_window, tokens ({final_tokens}) must be <= budget ({budget})"
    );
}

#[test]
fn long_session_compression_ratio_at_least_3x() {
    let system_prompt = "You are a helpful coding assistant.";
    let messages = build_long_conversation(system_prompt, 200);

    let raw_tokens = estimate_messages_tokens(&messages);

    let compactor = ContextCompactor::new(CompactionStrategy::TokenBudget {
        max_tokens: (CONTEXT_WINDOW as usize) * 3 / 4,
    });
    let result = compactor.compact(&messages);
    let compacted_tokens = estimate_messages_tokens(&result.messages);

    let ratio = raw_tokens as f64 / compacted_tokens.max(1) as f64;
    assert!(
        ratio >= 3.0,
        "Compression ratio should be >= 3x, got {ratio:.2}x \
         (raw={raw_tokens}, compacted={compacted_tokens})"
    );
}

#[test]
fn long_session_preserves_system_message() {
    let system_prompt = "CRITICAL: You are a security auditor. Never reveal secrets.";
    let mut messages = build_long_conversation(system_prompt, 200);

    ContextEngine::fit_to_context_window(&mut messages, CONTEXT_WINDOW, None);

    let system_msgs: Vec<_> = messages
        .iter()
        .filter(|m| matches!(m.role, Role::System))
        .collect();
    assert!(
        !system_msgs.is_empty(),
        "System message must survive compaction"
    );

    let has_original_system = system_msgs.iter().any(|m| {
        m.content
            .as_ref()
            .and_then(|c| c.as_str())
            .map_or(false, |s| s.contains("CRITICAL: You are a security auditor"))
    });
    assert!(
        has_original_system,
        "Original system prompt must be preserved verbatim"
    );
}

#[test]
fn long_session_preserves_recent_turns() {
    let system_prompt = "You are a helpful assistant.";
    let mut messages = build_long_conversation(system_prompt, 200);

    let last_user_content = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, Role::User))
        .and_then(|m| m.content.as_ref()?.as_str().map(String::from))
        .unwrap();

    ContextEngine::fit_to_context_window(&mut messages, CONTEXT_WINDOW, None);

    let has_last_user = messages.iter().any(|m| {
        matches!(m.role, Role::User)
            && m.content
                .as_ref()
                .and_then(|c| c.as_str())
                .map_or(false, |s| s == last_user_content)
    });
    assert!(
        has_last_user,
        "The most recent user message must be preserved after compaction"
    );

    let recent_5_markers: Vec<String> = (196..=200)
        .map(|i| format!("Turn {i}:"))
        .collect();
    let all_content: String = messages
        .iter()
        .filter_map(|m| m.content.as_ref()?.as_str().map(String::from))
        .collect::<Vec<_>>()
        .join("\n");
    let preserved_count = recent_5_markers
        .iter()
        .filter(|marker| all_content.contains(marker.as_str()))
        .count();
    assert!(
        preserved_count >= 3,
        "At least 3 of the last 5 turns should be preserved, got {preserved_count}"
    );
}

#[test]
fn importance_based_compaction_evicts_old_first() {
    let system_prompt = "You are a helpful assistant.";
    let messages = build_long_conversation(system_prompt, 100);

    let compactor = ContextCompactor::new(CompactionStrategy::ImportanceBased {
        max_messages: 30,
        recent_window: 10,
    });
    let result = compactor.compact(&messages);

    let upper = 30 + 3; // max_messages + system + optional summary marker
    assert!(
        result.compacted_count <= upper,
        "Should keep at most ~{upper} messages, got {}",
        result.compacted_count
    );

    let has_turn_100 = result.messages.iter().any(|m| {
        m.content
            .as_ref()
            .and_then(|c| c.as_str())
            .map_or(false, |s| s.contains("Turn 100"))
    });
    assert!(has_turn_100, "Most recent turn (100) must be preserved");

    let has_turn_1_user = result.messages.iter().any(|m| {
        matches!(m.role, Role::User)
            && m.content
                .as_ref()
                .and_then(|c| c.as_str())
                .map_or(false, |s| s.contains("Turn 1:"))
    });
    assert!(
        !has_turn_1_user,
        "Oldest user turn (1) should be evicted from non-system messages"
    );
}

#[test]
fn sliding_window_preserves_exact_recent_count() {
    let system_prompt = "System prompt.";
    let messages = build_long_conversation(system_prompt, 50);

    let keep_recent = 10;
    let compactor =
        ContextCompactor::new(CompactionStrategy::SlidingWindow { keep_recent });
    let result = compactor.compact(&messages);

    let non_system: Vec<_> = result
        .messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System))
        .collect();
    assert!(
        non_system.len() <= keep_recent,
        "SlidingWindow should keep at most {keep_recent} non-system messages, got {}",
        non_system.len()
    );

    let has_turn_50 = result.messages.iter().any(|m| {
        m.content
            .as_ref()
            .and_then(|c| c.as_str())
            .map_or(false, |s| s.contains("Turn 50"))
    });
    assert!(has_turn_50, "Latest turn must always be preserved");
}

#[test]
fn incremental_compaction_never_exceeds_budget() {
    let system_prompt = "You are a code reviewer.";
    let mut messages = vec![make_system_msg(system_prompt)];
    // Allow a small margin for per-message overhead rounding in the token estimator
    let budget = (CONTEXT_WINDOW - CONTEXT_WINDOW / 4) as usize + 16;

    for i in 1..=200 {
        messages.push(make_user_msg(&format!(
            "Turn {i}: Review this code change that modifies the authentication middleware \
             to support OAuth2 PKCE flow with refresh token rotation."
        )));
        messages.push(make_assistant_msg(&format!(
            "Turn {i} response: The OAuth2 PKCE implementation looks correct. The code \
             properly generates the code_verifier using a cryptographic random source and \
             derives the code_challenge with S256. Refresh token rotation is handled \
             atomically. Minor suggestion: add rate limiting on the token endpoint."
        )));

        if i % 10 == 0 {
            ContextEngine::fit_to_context_window(
                &mut messages,
                CONTEXT_WINDOW,
                None,
            );
            let tokens = estimate_messages_tokens(&messages);
            assert!(
                tokens <= budget,
                "After periodic compaction at turn {i}, tokens ({tokens}) > budget ({budget})"
            );
        }
    }

    let final_tokens = estimate_messages_tokens(&messages);
    assert!(
        final_tokens <= budget,
        "Final token count ({final_tokens}) exceeds budget ({budget})"
    );
}
