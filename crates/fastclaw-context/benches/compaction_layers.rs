use criterion::{black_box, criterion_group, criterion_main, Criterion};
use fastclaw_context::budget::TokenBudgetTracker;
use fastclaw_context::compressor::{CompactionStrategy, ContextCompactor};
use fastclaw_context::engine::ContextEngine;
use fastclaw_context::reactive::{ReactiveCompactor, ReactiveCompactorConfig};
use fastclaw_context::snip::{SnipCompactor, SnipCompactorConfig};
use fastclaw_core::types::{ChatMessage, Role};

fn make_msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(serde_json::Value::String(text.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn build_conversation(turns: usize) -> Vec<ChatMessage> {
    let mut msgs = vec![make_msg(
        Role::System,
        "You are a senior engineer providing code review.",
    )];
    for i in 1..=turns {
        msgs.push(make_msg(
            Role::User,
            &format!(
                "Turn {i}: Analyze the performance of the distributed routing component \
                 with latency percentiles, throughput, and resource utilization."
            ),
        ));
        msgs.push(make_msg(
            Role::Assistant,
            &format!(
                "Turn {i}: The routing component uses consistent hashing with p50 latency \
                 of 12ms and throughput of 15k req/s. At p99, latency rises to 45ms. \
                 Resource utilization is 60% CPU, 4GB memory per node at peak."
            ),
        ));
    }
    msgs
}

fn bench_snip_compact(c: &mut Criterion) {
    let msgs_50 = build_conversation(50);
    let msgs_200 = build_conversation(200);

    let snip = SnipCompactor::new(SnipCompactorConfig {
        max_tokens: 4096,
        min_rounds_to_keep: 3,
    });

    c.bench_function("snip_compact_50_turns", |b| {
        b.iter(|| snip.compact(black_box(&msgs_50)))
    });

    c.bench_function("snip_compact_200_turns", |b| {
        b.iter(|| snip.compact(black_box(&msgs_200)))
    });
}

fn bench_reactive_compact(c: &mut Criterion) {
    let msgs_50 = build_conversation(50);
    let msgs_200 = build_conversation(200);

    let reactive = ReactiveCompactor::new(ReactiveCompactorConfig {
        target_tokens: 4096,
        snip_min_rounds: 3,
        hard_truncate_keep: 6,
    });

    c.bench_function("reactive_compact_50_turns", |b| {
        b.iter(|| reactive.compact(black_box(&msgs_50)))
    });

    c.bench_function("reactive_compact_200_turns", |b| {
        b.iter(|| reactive.compact(black_box(&msgs_200)))
    });
}

fn bench_importance_compact(c: &mut Criterion) {
    let msgs_50 = build_conversation(50);
    let msgs_200 = build_conversation(200);

    let compactor = ContextCompactor::new(CompactionStrategy::ImportanceBased {
        max_messages: 30,
        recent_window: 10,
    });

    c.bench_function("importance_compact_50_turns", |b| {
        b.iter(|| compactor.compact(black_box(&msgs_50)))
    });

    c.bench_function("importance_compact_200_turns", |b| {
        b.iter(|| compactor.compact(black_box(&msgs_200)))
    });
}

fn bench_token_budget_compact(c: &mut Criterion) {
    let msgs_200 = build_conversation(200);

    let compactor = ContextCompactor::new(CompactionStrategy::TokenBudget { max_tokens: 6144 });

    c.bench_function("token_budget_compact_200_turns", |b| {
        b.iter(|| compactor.compact(black_box(&msgs_200)))
    });
}

fn bench_fit_to_context_window(c: &mut Criterion) {
    let msgs_200 = build_conversation(200);

    c.bench_function("fit_to_context_window_200_turns", |b| {
        b.iter(|| {
            let mut msgs = msgs_200.clone();
            ContextEngine::fit_to_context_window(black_box(&mut msgs), 8192, None)
        })
    });
}

fn bench_budget_tracker_check(c: &mut Criterion) {
    c.bench_function("budget_tracker_record_100_turns", |b| {
        b.iter(|| {
            let mut tracker = TokenBudgetTracker::new(128_000);
            for _ in 0..100 {
                black_box(tracker.record(500));
            }
        })
    });

    c.bench_function("budget_tracker_record_single", |b| {
        let mut tracker = TokenBudgetTracker::new(128_000);
        b.iter(|| black_box(tracker.record(500)))
    });
}

criterion_group!(
    benches,
    bench_snip_compact,
    bench_reactive_compact,
    bench_importance_compact,
    bench_token_budget_compact,
    bench_fit_to_context_window,
    bench_budget_tracker_check,
);
criterion_main!(benches);
