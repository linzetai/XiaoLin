//! Integration tests for the Tauri IPC command logic.
//!
//! These tests exercise the same AppState interactions that the `#[tauri::command]`
//! functions perform, without needing a Tauri runtime. Each test creates a real
//! `AppState::for_test` backed by temp-dir SQLite databases and a mock LLM provider.

use fastclaw_agent::{CompletionParams, LlmProvider};
use fastclaw_core::types::{
    ChatChoice, ChatMessage, ChatRequest, ChatResponse, DeltaContent, Role, StreamChoice,
    StreamDelta,
};
use fastclaw_gateway::AppState;
use serde_json::json;

// ═══════════════════════════════════════════════════════════════════
// Mock LLM provider
// ═══════════════════════════════════════════════════════════════════

struct MockProvider;

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    async fn chat_completion(
        &self,
        _params: &CompletionParams<'_>,
    ) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            id: "mock-id".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "mock-model".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: Role::Assistant,
                    content: Some("Hello from mock".into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        })
    }

    async fn chat_completion_stream(
        &self,
        _params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use futures::stream;
        let deltas = vec![
            Ok(StreamDelta {
                id: "mock-stream".into(),
                object: "chat.completion.chunk".into(),
                created: 0,
                model: "mock-model".into(),
                choices: vec![StreamChoice {
                    index: 0,
                    delta: DeltaContent {
                        role: Some(Role::Assistant),
                        content: Some("Mock streamed reply".into()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            }),
            Ok(StreamDelta {
                id: "mock-stream".into(),
                object: "chat.completion.chunk".into(),
                created: 0,
                model: "mock-model".into(),
                choices: vec![StreamChoice {
                    index: 0,
                    delta: DeltaContent {
                        role: None,
                        content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some("stop".into()),
                }],
            }),
        ];
        Ok(Box::pin(stream::iter(deltas)))
    }
}

async fn test_state() -> (AppState, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let state = AppState::for_test(Box::new(MockProvider), tmp.path())
        .await
        .expect("build test AppState");
    (state, tmp)
}

// ═══════════════════════════════════════════════════════════════════
// Agents
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn list_agents_returns_default_agent() {
    let (state, _tmp) = test_state().await;
    let agents: Vec<_> = state
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| json!({"agentId": a.agent_id, "name": a.name, "model": a.model.model}))
        .collect();
    assert!(!agents.is_empty(), "should have at least one agent");
    let default = agents.iter().find(|a| a["agentId"] == "main");
    assert!(default.is_some(), "should have 'main' agent");
}

#[tokio::test]
async fn list_agents_contains_model_field() {
    let (state, _tmp) = test_state().await;
    let guard = state.router.read().await;
    let agents = guard.list_agents();
    for agent in agents {
        assert!(!agent.model.model.is_empty(), "model field should not be empty");
    }
}

// ═══════════════════════════════════════════════════════════════════
// Sessions — CRUD lifecycle
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn create_session_and_get() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state
        .session_store
        .create_session(&sid, "main", None)
        .await
        .expect("create session");

    let session = state
        .session_store
        .get_session(&sid)
        .await
        .expect("get session")
        .expect("session should exist");
    assert_eq!(session.id, sid);
    assert_eq!(session.agent_id, "main");
    assert!(session.title.is_none(), "new session should have no title");
}

#[tokio::test]
async fn list_sessions_includes_created() {
    let (state, _tmp) = test_state().await;

    let sid1 = uuid::Uuid::new_v4().to_string();
    let sid2 = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid1, "main", None).await.unwrap();
    state.session_store.create_session(&sid2, "main", None).await.unwrap();

    let sessions = state.session_store.list_sessions(50, 0).await.unwrap();
    assert!(sessions.len() >= 2, "should list at least 2 sessions");

    let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&sid1.as_str()));
    assert!(ids.contains(&sid2.as_str()));
}

#[tokio::test]
async fn list_sessions_respects_limit() {
    let (state, _tmp) = test_state().await;
    for _ in 0..5 {
        let sid = uuid::Uuid::new_v4().to_string();
        state.session_store.create_session(&sid, "main", None).await.unwrap();
    }

    let sessions = state.session_store.list_sessions(2, 0).await.unwrap();
    assert!(sessions.len() <= 2, "limit should cap results to 2, got {}", sessions.len());
}

#[tokio::test]
async fn list_sessions_respects_offset() {
    let (state, _tmp) = test_state().await;
    for _ in 0..5 {
        let sid = uuid::Uuid::new_v4().to_string();
        state.session_store.create_session(&sid, "main", None).await.unwrap();
    }

    let all = state.session_store.list_sessions(50, 0).await.unwrap();
    let offset_2 = state.session_store.list_sessions(50, 2).await.unwrap();
    assert_eq!(offset_2.len(), all.len().saturating_sub(2));
}

#[tokio::test]
async fn update_session_title() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    state
        .session_store
        .update_title(&sid, "Test Title")
        .await
        .expect("update title");

    let session = state.session_store.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.title.as_deref(), Some("Test Title"));
}

#[tokio::test]
async fn update_session_title_overwrites() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    state.session_store.update_title(&sid, "First").await.unwrap();
    state.session_store.update_title(&sid, "Second").await.unwrap();

    let session = state.session_store.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.title.as_deref(), Some("Second"));
}

#[tokio::test]
async fn delete_session_removes_it() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    let deleted = state.session_store.delete_session(&sid).await.unwrap();
    assert!(deleted, "should return true");

    let session = state.session_store.get_session(&sid).await.unwrap();
    assert!(session.is_none(), "session should be gone after delete");
}

#[tokio::test]
async fn delete_nonexistent_session() {
    let (state, _tmp) = test_state().await;
    let result = state.session_store.delete_session("nonexistent").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn get_nonexistent_session_returns_none() {
    let (state, _tmp) = test_state().await;
    let session = state.session_store.get_session("no-such-id").await.unwrap();
    assert!(session.is_none());
}

// ═══════════════════════════════════════════════════════════════════
// Session messages
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn append_and_load_messages() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    let user_msg = ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String("Hello".into())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    state.session_store.append_message(&sid, &user_msg).await.unwrap();

    let assistant_msg = ChatMessage {
        role: Role::Assistant,
        content: Some(serde_json::Value::String("Hi there!".into())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    state.session_store.append_message(&sid, &assistant_msg).await.unwrap();

    let messages = state.session_store.load_messages(&sid).await.unwrap();
    assert_eq!(messages.len(), 2, "should have 2 messages");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
}

#[tokio::test]
async fn load_messages_empty_session() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    let messages = state.session_store.load_messages(&sid).await.unwrap();
    assert!(messages.is_empty(), "new session should have no messages");
}

#[tokio::test]
async fn load_messages_preserves_content() {
    let (state, _tmp) = test_state().await;
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    let msg = ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String("Test content with émojis 🎉".into())),
        name: Some("test-user".into()),
        tool_calls: None,
        tool_call_id: None,
    };
    state.session_store.append_message(&sid, &msg).await.unwrap();

    let loaded = state.session_store.load_messages(&sid).await.unwrap();
    assert_eq!(loaded.len(), 1);
    let content = loaded[0].content.as_deref().unwrap();
    assert!(content.contains("émojis 🎉"), "Unicode content should be preserved");
}

// ═══════════════════════════════════════════════════════════════════
// Models
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn list_models_returns_agent_models() {
    let (state, _tmp) = test_state().await;
    let models: Vec<_> = state
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| {
            json!({
                "agentId": a.agent_id,
                "model": a.model.model,
                "provider": a.model.provider,
            })
        })
        .collect();
    assert!(!models.is_empty(), "should have at least one model entry");
    let default = models.iter().find(|m| m["agentId"] == "main");
    assert!(default.is_some(), "should have model for 'main' agent");
}

#[tokio::test]
async fn list_models_has_required_fields() {
    let (state, _tmp) = test_state().await;
    let guard = state.router.read().await;
    let agents = guard.list_agents();
    for a in agents {
        assert!(!a.agent_id.is_empty());
        assert!(!a.model.model.is_empty());
        assert!(!a.model.provider.is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════
// Config
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn config_get_full() {
    let (state, _tmp) = test_state().await;
    let full = serde_json::to_value(&*state.config).unwrap();
    assert!(full.is_object());
    assert!(full.get("gateway").is_some());
}

#[tokio::test]
async fn config_get_specific_key() {
    let (state, _tmp) = test_state().await;
    let full = serde_json::to_value(&*state.config).unwrap();
    let gateway = full.get("gateway");
    assert!(gateway.is_some(), "config should have gateway section");
}

#[tokio::test]
async fn config_serialization_roundtrip() {
    let (state, _tmp) = test_state().await;
    let json_val = serde_json::to_value(&*state.config).unwrap();
    let _parsed: fastclaw_core::config::FastClawConfig =
        serde_json::from_value(json_val).expect("config should deserialize back");
}

// ═══════════════════════════════════════════════════════════════════
// Chat streaming via AppState (no Tauri runtime needed)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn stream_chat_produces_events() {
    use fastclaw_core::types::StreamEvent;

    let (state, _tmp) = test_state().await;

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String("Hello".into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        model: None,
        stream: true,
        max_tokens: None,
        temperature: None,
        agent_id: Some("main".into()),
        session_id: None,
        tools: None,
        slash_intent: None,
        work_dir: None,
    };

    let agent_config = {
        let router = state.router.read().await;
        router.resolve(&request).unwrap().clone()
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

    let runtime = state.runtime.clone();
    let tool_reg = state.tool_registry.clone();
    let cfg = agent_config;

    let task = tokio::spawn(async move {
        runtime
            .execute_stream(&cfg, &request, &tool_reg, tx, None)
            .await
    });

    let mut got_delta = false;
    let mut got_done = false;
    let mut content = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Delta(delta) => {
                got_delta = true;
                if let Some(text) = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref())
                {
                    content.push_str(text);
                }
            }
            StreamEvent::Done { .. } => {
                got_done = true;
            }
            StreamEvent::Error(e) => {
                panic!("unexpected stream error: {e}");
            }
            _ => {}
        }
    }

    task.await.unwrap().expect("stream task should succeed");
    assert!(got_delta, "should have received at least one delta");
    assert!(got_done, "should have received done event");
    assert!(!content.is_empty(), "streamed content should not be empty");
    assert!(
        content.contains("Mock streamed"),
        "content should come from MockProvider"
    );
}

#[tokio::test]
async fn stream_chat_delta_content_accumulates() {
    use fastclaw_core::types::StreamEvent;

    let (state, _tmp) = test_state().await;

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String("Test accumulation".into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        model: None,
        stream: true,
        max_tokens: None,
        temperature: None,
        agent_id: Some("main".into()),
        session_id: None,
        tools: None,
        slash_intent: None,
        work_dir: None,
    };

    let agent_config = {
        let router = state.router.read().await;
        router.resolve(&request).unwrap().clone()
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

    let runtime = state.runtime.clone();
    let tool_reg = state.tool_registry.clone();

    tokio::spawn(async move {
        runtime
            .execute_stream(&agent_config, &request, &tool_reg, tx, None)
            .await
    });

    let mut delta_count = 0;
    while let Some(event) = rx.recv().await {
        if matches!(event, StreamEvent::Delta(_)) {
            delta_count += 1;
        }
    }
    assert!(delta_count >= 1, "should receive at least 1 delta event");
}

// ═══════════════════════════════════════════════════════════════════
// Full session lifecycle: create → chat → messages → title → delete
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn full_session_lifecycle() {
    let (state, _tmp) = test_state().await;

    // 1. Create session
    let sid = uuid::Uuid::new_v4().to_string();
    state.session_store.create_session(&sid, "main", None).await.unwrap();

    // 2. Verify empty
    let msgs = state.session_store.load_messages(&sid).await.unwrap();
    assert!(msgs.is_empty());

    // 3. Add messages
    let user_msg = ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String("What is 2+2?".into())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    state.session_store.append_message(&sid, &user_msg).await.unwrap();

    let asst_msg = ChatMessage {
        role: Role::Assistant,
        content: Some(serde_json::Value::String("4".into())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    state.session_store.append_message(&sid, &asst_msg).await.unwrap();

    // 4. Verify messages
    let msgs = state.session_store.load_messages(&sid).await.unwrap();
    assert_eq!(msgs.len(), 2);

    // 5. Update title
    state.session_store.update_title(&sid, "Math Question").await.unwrap();
    let s = state.session_store.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(s.title.as_deref(), Some("Math Question"));

    // 6. Session appears in list
    let all = state.session_store.list_sessions(100, 0).await.unwrap();
    assert!(all.iter().any(|s| s.id == sid));

    // 7. Delete
    state.session_store.delete_session(&sid).await.unwrap();

    // 8. Gone
    let gone = state.session_store.get_session(&sid).await.unwrap();
    assert!(gone.is_none());
}

// ═══════════════════════════════════════════════════════════════════
// Router resolve
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn router_resolves_main_agent() {
    let (state, _tmp) = test_state().await;
    let request = ChatRequest {
        messages: vec![],
        model: None,
        stream: false,
        max_tokens: None,
        temperature: None,
        agent_id: Some("main".into()),
        session_id: None,
        tools: None,
        slash_intent: None,
        work_dir: None,
    };
    let router = state.router.read().await;
    let config = router.resolve(&request);
    assert!(config.is_ok(), "should resolve 'main' agent");
    assert_eq!(config.unwrap().agent_id, "main");
}

#[tokio::test]
async fn router_resolve_nonexistent_agent_fails() {
    let (state, _tmp) = test_state().await;
    let request = ChatRequest {
        messages: vec![],
        model: None,
        stream: false,
        max_tokens: None,
        temperature: None,
        agent_id: Some("nonexistent-agent-12345".into()),
        session_id: None,
        tools: None,
        slash_intent: None,
        work_dir: None,
    };
    let router = state.router.read().await;
    let result = router.resolve(&request);
    assert!(result.is_err(), "nonexistent agent should fail to resolve");
}

// ═══════════════════════════════════════════════════════════════════
// Broadcast channel (for Tauri events)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ws_broadcast_delivers_to_subscriber() {
    let (state, _tmp) = test_state().await;
    let mut rx = state.ws_broadcast.subscribe();

    let payload = json!({"type":"event","event":"sessions.changed","data":{"sessionId":"test-123"}}).to_string();
    state.ws_broadcast.send(payload.clone()).unwrap();

    let received = rx.recv().await.unwrap();
    assert_eq!(received, payload);
}

#[tokio::test]
async fn ws_broadcast_multiple_subscribers() {
    let (state, _tmp) = test_state().await;
    let mut rx1 = state.ws_broadcast.subscribe();
    let mut rx2 = state.ws_broadcast.subscribe();

    let payload = json!({"event":"test"}).to_string();
    state.ws_broadcast.send(payload.clone()).unwrap();

    assert_eq!(rx1.recv().await.unwrap(), payload);
    assert_eq!(rx2.recv().await.unwrap(), payload);
}
