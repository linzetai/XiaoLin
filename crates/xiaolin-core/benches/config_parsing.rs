use criterion::{black_box, criterion_group, criterion_main, Criterion};
use xiaolin_core::config::XiaoLinConfig;

fn realistic_config_json() -> String {
    serde_json::json!({
        "gateway": {
            "host": "0.0.0.0",
            "port": 3100
        },
        "logging": {
            "level": "info"
        },
        "session": {
            "mode": "sqlite"
        },
        "memory": {
            "enabled": true
        },
        "security": {
            "apiKeys": ["sk-test-key-1234567890"],
            "corsOrigins": ["http://localhost:3000"]
        },
        "models": {
            "openai": {
                "provider": "openai",
                "apiKey": "sk-fake",
                "models": ["gpt-4o", "gpt-4o-mini"]
            },
            "anthropic": {
                "provider": "anthropic",
                "apiKey": "sk-fake2",
                "models": ["claude-sonnet-4-20250514"]
            }
        },
        "agents": {
            "defaults": {
                "model": "gpt-4o",
                "systemPrompt": "You are a helpful assistant.",
                "tools": ["shell_exec", "web_search"]
            },
            "list": [
                {
                    "id": "default",
                    "name": "Default Agent",
                    "model": "gpt-4o",
                    "systemPrompt": "You are a helpful AI assistant."
                },
                {
                    "id": "coder",
                    "name": "Coder Agent",
                    "model": "claude-sonnet-4-20250514",
                    "systemPrompt": "You are an expert programmer.",
                    "tools": ["shell_exec", "file_read", "file_write"]
                }
            ]
        },
        "mcpServers": [
            {
                "id": "filesystem",
                "command": "npx",
                "args": ["-y", "@anthropic-ai/filesystem-mcp"],
                "enabled": true
            }
        ],
        "modelRouter": {
            "strategy": "cost_optimized"
        },
        "tracing": {
            "conversationTrace": true
        }
    })
    .to_string()
}

fn bench_config_parsing(c: &mut Criterion) {
    let json_str = realistic_config_json();

    c.bench_function("config_from_json_str", |b| {
        b.iter(|| serde_json::from_str::<XiaoLinConfig>(black_box(&json_str)).unwrap())
    });

    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    c.bench_function("config_from_value", |b| {
        b.iter(|| serde_json::from_value::<XiaoLinConfig>(black_box(value.clone())).unwrap())
    });

    c.bench_function("config_json5_parse", |b| {
        b.iter(|| json5::from_str::<serde_json::Value>(black_box(&json_str)).unwrap())
    });
}

criterion_group!(benches, bench_config_parsing);
criterion_main!(benches);
