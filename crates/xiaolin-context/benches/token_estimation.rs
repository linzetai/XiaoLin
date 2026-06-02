use criterion::{black_box, criterion_group, criterion_main, Criterion};
use xiaolin_context::estimate_messages_tokens;
use xiaolin_core::types::{ChatMessage, Role};

fn make_messages(count: usize, content_len: usize) -> Vec<ChatMessage> {
    let text: String = "a".repeat(content_len);
    (0..count)
        .map(|i| ChatMessage {
            role: if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            content: Some(serde_json::Value::String(text.clone())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        })
        .collect()
}

fn bench_token_estimation(c: &mut Criterion) {
    let short_conv = make_messages(10, 200);
    let long_conv = make_messages(50, 2000);
    let single_msg = make_messages(1, 50_000);

    c.bench_function("estimate_tokens_10msg_200chars", |b| {
        b.iter(|| estimate_messages_tokens(black_box(&short_conv)))
    });

    c.bench_function("estimate_tokens_50msg_2000chars", |b| {
        b.iter(|| estimate_messages_tokens(black_box(&long_conv)))
    });

    c.bench_function("estimate_tokens_1msg_50k_chars", |b| {
        b.iter(|| estimate_messages_tokens(black_box(&single_msg)))
    });
}

criterion_group!(benches, bench_token_estimation);
criterion_main!(benches);
