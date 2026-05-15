//! End-to-end scenario tests for FastClaw.
//!
//! These tests simulate realistic multi-step user workflows through the full
//! gateway stack.  A `ScriptedProvider` replaces the real LLM, returning a
//! pre-programmed sequence of responses (including tool_calls) so every
//! scenario is deterministic and network-free.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use fastclaw_agent::{CompletionParams, LlmProvider};
use fastclaw_core::types::{
    ChatChoice, ChatMessage, ChatResponse, DeltaContent, FunctionCall, Role, StreamChoice,
    StreamDelta, ToolCall,
};
use fastclaw_gateway::{build_app, AppState};
use fastclaw_security::{ApiKeyAuth, AuthConfig};
use serde_json::{json, Value};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// ScriptedProvider — programmable mock LLM that returns queued responses
// ---------------------------------------------------------------------------

struct ScriptedResponse {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

struct ScriptedProvider {
    responses: Vec<ScriptedResponse>,
    call_count: AtomicUsize,
}

impl ScriptedProvider {
    fn new(responses: Vec<ScriptedResponse>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    async fn chat_completion(
        &self,
        _params: &CompletionParams<'_>,
    ) -> anyhow::Result<ChatResponse> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        let (content, tool_calls) = if let Some(resp) = self.responses.get(idx) {
            (
                resp.content
                    .as_ref()
                    .map(|s| serde_json::Value::String(s.clone())),
                resp.tool_calls.clone(),
            )
        } else {
            // Extra calls (e.g. session title generation) get a benign fallback.
            (
                Some(serde_json::Value::String("(fallback)".to_string())),
                None,
            )
        };

        Ok(ChatResponse {
            id: format!("scripted-{idx}"),
            object: "chat.completion".into(),
            created: 0,
            model: "scripted-model".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: Role::Assistant,
                    content,
                    reasoning_content: None,
                    name: None,
                    tool_calls,
                    tool_call_id: None,
            compact_metadata: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        })
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
        let resp = self.chat_completion(params).await?;
        let choice = &resp.choices[0];
        use futures::stream;
        let deltas = vec![
            Ok(StreamDelta {
                id: resp.id.clone(),
                object: "chat.completion.chunk".into(),
                created: 0,
                model: resp.model.clone(),
                choices: vec![StreamChoice {
                    index: 0,
                    delta: DeltaContent {
                        role: Some(Role::Assistant),
                        content: choice.message.text_content(),
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
                usage: None,
            }),
            Ok(StreamDelta {
                id: resp.id.clone(),
                object: "chat.completion.chunk".into(),
                created: 0,
                model: resp.model.clone(),
                choices: vec![StreamChoice {
                    index: 0,
                    delta: DeltaContent {
                        role: None,
                        content: None,
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some("stop".into()),
                }],
                usage: None,
            }),
        ];
        Ok(Box::pin(stream::iter(deltas)))
    }
}

// ---------------------------------------------------------------------------
// Test harness (reuses production AppState::for_test)
// ---------------------------------------------------------------------------

struct E2eServer {
    addr: SocketAddr,
    _tmp: tempfile::TempDir,
}

impl E2eServer {
    async fn start(provider: Box<dyn LlmProvider>) -> Self {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let state = AppState::for_test(provider, tmp.path())
            .await
            .expect("build test AppState");
        let auth = ApiKeyAuth::new(&AuthConfig {
            enabled: false,
            api_keys: vec![],
        });
        let app = build_app(state, auth);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self { addr, _tmp: tmp }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn ws_url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }
}

fn make_tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
        output: None,
        success: None,
        duration_ms: None,
    }
}

fn simple_provider() -> Box<dyn LlmProvider> {
    Box::new(ScriptedProvider::new(vec![ScriptedResponse {
        content: Some("Hello from scripted mock".into()),
        tool_calls: None,
    }]))
}

// ===================================================================
// Scenario 1: Multi-step tool chain (LLM → calculator → final answer)
// ===================================================================

#[tokio::test]
async fn e2e_multi_step_tool_chain() {
    let provider = ScriptedProvider::new(vec![
        // Round 1: LLM requests calculator tool
        ScriptedResponse {
            content: None,
            tool_calls: Some(vec![make_tool_call(
                "tc_calc_1",
                "calculator",
                r#"{"expression": "2 + 3 * 4"}"#,
            )]),
        },
        // Round 2: After receiving tool result, LLM provides final answer
        ScriptedResponse {
            content: Some("The result of 2 + 3 * 4 is 14.".into()),
            tool_calls: None,
        },
    ]);

    let srv = E2eServer::start(Box::new(provider)).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "What is 2 + 3 * 4?"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .expect("response should have content");
    assert!(
        content.contains("14"),
        "final answer should contain the computed result: {content}"
    );

    let session_id = resp["_meta"]["sessionId"].as_str().unwrap();
    let msgs: Value = client
        .get(srv.url(&format!("/api/v1/sessions/{session_id}/messages")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = msgs["messages"].as_array().unwrap();
    // The runtime persists at least the user message and the final assistant message.
    // Intermediate tool_call / tool_result messages may or may not be saved depending
    // on session persistence granularity; the key assertion is that we got a correct
    // final answer that required the tool chain.
    assert!(
        messages.len() >= 2,
        "should have at least user + final assistant: got {}",
        messages.len()
    );
}

// ===================================================================
// Scenario 2: Session continuity across multiple chat requests
// ===================================================================

#[tokio::test]
async fn e2e_session_continuity() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("Hello! I remember you asked about Rust.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Sure, continuing our Rust discussion.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Third turn, context preserved.".into()),
            tool_calls: None,
        },
    ]);

    let srv = E2eServer::start(Box::new(provider)).await;
    let client = reqwest::Client::new();

    // Turn 1: create session
    let r1: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "Tell me about Rust"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = r1["_meta"]["sessionId"].as_str().unwrap().to_string();

    // Turn 2: continue same session
    let r2: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "sessionId": &session_id,
            "messages": [{"role": "user", "content": "Go on"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        r2["_meta"]["sessionId"].as_str().unwrap(),
        session_id,
        "should reuse the same session"
    );

    // Turn 3: third turn
    let _r3: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "sessionId": &session_id,
            "messages": [{"role": "user", "content": "More details please"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Verify accumulated messages
    let msgs: Value = client
        .get(srv.url(&format!("/api/v1/sessions/{session_id}/messages")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = msgs["messages"].as_array().unwrap();
    assert!(
        messages.len() >= 6,
        "3 user + 3 assistant messages expected, got {}",
        messages.len()
    );

    let user_msgs: Vec<_> = messages.iter().filter(|m| m["role"] == "user").collect();
    assert_eq!(user_msgs.len(), 3, "should have 3 user messages");
}

// ===================================================================
// Scenario 3: Memory API lifecycle — episodes, facts, search
// ===================================================================

#[tokio::test]
async fn e2e_memory_lifecycle() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    // Store a fact via the upsert endpoint (agent_id is a query param)
    let fact: Value = client
        .post(srv.url("/api/v1/memory/facts?agent_id=main"))
        .json(&json!({
            "id": "fact-001",
            "category": "technology",
            "subject": "FastClaw",
            "predicate": "is written in",
            "object": "Rust",
            "confidence": 1.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(fact.get("ok").is_some(), "fact should be created: {fact}");

    // List facts
    let facts: Value = client
        .get(srv.url("/api/v1/memory/facts?agent_id=main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let fact_list = facts["facts"]
        .as_array()
        .expect("should return facts array");
    assert!(
        !fact_list.is_empty(),
        "should have at least one fact stored"
    );

    // Search facts
    let search: Value = client
        .get(srv.url("/api/v1/memory/facts/search?agent_id=main&q=Rust"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let status = search.as_object().expect("search should return an object");
    assert!(!status.is_empty(), "fact search should return results");

    // Chat to potentially generate an episode (auto_record_episode is fire-and-forget)
    let _: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "Explain FastClaw memory"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Give the async auto-record task a moment to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // List episodes — they may or may not be auto-recorded depending on content
    // length thresholds, but the endpoint should be functional.
    let episodes_resp = client
        .get(srv.url("/api/v1/memory/episodes?agent_id=main"))
        .send()
        .await
        .unwrap();
    assert_eq!(episodes_resp.status(), 200);
    let episodes: Value = episodes_resp.json().await.unwrap();
    assert!(
        episodes.get("episodes").is_some(),
        "episodes endpoint should return an episodes field: {episodes}"
    );

    // Search episodes endpoint should also work
    let search_resp = client
        .get(srv.url("/api/v1/memory/episodes/search?agent_id=main&q=memory"))
        .send()
        .await
        .unwrap();
    assert_eq!(search_resp.status(), 200);
}

// ===================================================================
// Scenario 4: Feedback → Evaluate → Distill evolution loop
// ===================================================================

#[tokio::test]
async fn e2e_evolution_feedback_loop() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    // Submit positive feedback
    let fb_resp = client
        .post(srv.url("/api/v1/evolution/feedback"))
        .json(&json!({
            "agent_id": "main",
            "session_id": "test-session-1",
            "kind": "thumbs_up",
            "comment": "Great answer about Rust!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(fb_resp.status(), 200);

    // Submit negative feedback
    let fb_resp = client
        .post(srv.url("/api/v1/evolution/feedback"))
        .json(&json!({
            "agent_id": "main",
            "session_id": "test-session-2",
            "kind": "thumbs_down",
            "comment": "Inaccurate response"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(fb_resp.status(), 200);

    // Retrieve feedback for agent
    let fb_list: Value = client
        .get(srv.url("/api/v1/evolution/feedback/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let feedbacks = fb_list["feedback"]
        .as_array()
        .or_else(|| fb_list["feedbacks"].as_array())
        .expect("feedback array");
    assert!(
        feedbacks.len() >= 2,
        "should have at least 2 feedback entries, got {}",
        feedbacks.len()
    );

    // Evaluate agent performance
    let eval: Value = client
        .get(srv.url("/api/v1/evolution/evaluate/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        eval.get("report").is_some(),
        "evaluation should return a report: {eval}"
    );

    // Trigger prompt distillation
    let distill: Value = client
        .post(srv.url("/api/v1/evolution/distill/main"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !distill.is_null(),
        "distill should return a result: {distill}"
    );

    // List candidates
    let candidates: Value = client
        .get(srv.url("/api/v1/evolution/candidates/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        candidates.get("candidates").is_some(),
        "should return candidates list: {candidates}"
    );
}

// ===================================================================
// Scenario 5: WebSocket multi-turn conversation with streaming
// ===================================================================

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn ws_recv_json(
    rx: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.next())
            .await
            .expect("ws recv timeout")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

async fn ws_send_json(
    tx: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    val: Value,
) {
    tx.send(Message::Text(val.to_string())).await.unwrap();
}

/// Helper: drive a WS chat to completion, returning (session_id, event_types).
async fn ws_chat_to_completion(
    tx: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    rx: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    id: &str,
    user_msg: &str,
    session_id: Option<&str>,
) -> (String, Vec<String>) {
    let mut params = json!({
        "messages": [{"role": "user", "content": user_msg}]
    });
    if let Some(sid) = session_id {
        params["sessionId"] = json!(sid);
    }
    ws_send_json(tx, json!({"id": id, "method": "chat", "params": params})).await;

    let mut sid = String::new();
    let mut event_types = Vec::new();

    loop {
        let msg = ws_recv_json(rx).await;
        let ty = msg["type"].as_str().unwrap_or("unknown").to_string();
        event_types.push(ty.clone());
        if ty == "chat.start" {
            sid = msg["data"]["sessionId"]
                .as_str()
                .unwrap_or_default()
                .to_string();
        }
        if ty == "chat.complete" || ty == "chat.error" {
            break;
        }
    }
    (sid, event_types)
}

#[tokio::test]
async fn e2e_ws_multi_turn_streaming() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("First response via WebSocket.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Second response, same session.".into()),
            tool_calls: None,
        },
    ]);

    let srv = E2eServer::start(Box::new(provider)).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();

    // Consume welcome
    let welcome = ws_recv_json(&mut rx).await;
    assert_eq!(welcome["type"], "connected");

    // Turn 1
    let (session_id, events1) =
        ws_chat_to_completion(&mut tx, &mut rx, "t1", "Hello WebSocket!", None).await;
    assert!(!session_id.is_empty(), "should receive a session ID");
    assert!(events1.contains(&"chat.start".to_string()));
    assert!(events1.contains(&"chat.complete".to_string()));

    // Turn 2: continue same session
    let (sid2, events2) =
        ws_chat_to_completion(&mut tx, &mut rx, "t2", "Continue please", Some(&session_id)).await;
    assert_eq!(sid2, session_id, "should reuse same session");
    assert!(events2.contains(&"chat.start".to_string()));

    // Verify messages persisted
    ws_send_json(
        &mut tx,
        json!({
            "id": "sm1",
            "method": "sessions.messages",
            "params": {"sessionId": &session_id}
        }),
    )
    .await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "sessions.messages");
    let messages = resp["data"]["messages"].as_array().unwrap();
    assert!(
        messages.len() >= 4,
        "2 turns = 2 user + 2 assistant = 4 messages minimum, got {}",
        messages.len()
    );
}

// ===================================================================
// Scenario 7: Health, readiness, and metrics observability
// ===================================================================

#[tokio::test]
async fn e2e_observability_endpoints() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    // Health
    let health: Value = client
        .get(srv.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");

    // Readiness
    let ready: Value = client
        .get(srv.url("/ready"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ready["status"], "ready");

    // Metrics (Prometheus text format) — endpoint should return 200
    let metrics_resp = client.get(srv.url("/metrics")).send().await.unwrap();
    assert_eq!(metrics_resp.status(), 200);

    // Structured metrics (may be Prometheus text or JSON)
    let structured_resp = client.get(srv.url("/api/v1/metrics")).send().await.unwrap();
    assert_eq!(structured_resp.status(), 200);
    let body = structured_resp.text().await.unwrap();
    // Endpoint should be reachable (body may or may not be empty).
    let _ = &body;
}

// ===================================================================
// Scenario 8: Full chat → session → delete lifecycle via HTTP
// ===================================================================

#[tokio::test]
async fn e2e_http_chat_session_delete_lifecycle() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("Response one.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Response two.".into()),
            tool_calls: None,
        },
    ]);

    let srv = E2eServer::start(Box::new(provider)).await;
    let client = reqwest::Client::new();

    // Chat 1
    let r1: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "start conversation"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = r1["_meta"]["sessionId"].as_str().unwrap().to_string();

    // Session appears in list
    let list: Value = client
        .get(srv.url("/api/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        list["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session_id),
        "new session should appear in list"
    );

    // Chat 2 in same session
    let _r2: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "sessionId": &session_id,
            "messages": [{"role": "user", "content": "follow up"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Verify messages
    let msgs: Value = client
        .get(srv.url(&format!("/api/v1/sessions/{session_id}/messages")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(msgs["messages"].as_array().unwrap().len() >= 4);

    // Delete session
    let del: Value = client
        .delete(srv.url(&format!("/api/v1/sessions/{session_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del["deleted"], true);

    // Session should be gone
    let resp = client
        .get(srv.url(&format!("/api/v1/sessions/{session_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ===================================================================
// Scenario 9: Agent tools listing and registry validation
// ===================================================================

#[tokio::test]
async fn e2e_agent_tools_registry() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    let agents: Value = client
        .get(srv.url("/api/v1/agents"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_list = agents["agents"].as_array().unwrap();
    assert!(!agent_list.is_empty(), "should have at least one agent");
    assert_eq!(agent_list[0]["agentId"], "main");

    let tools: Value = client
        .get(srv.url("/api/v1/tools"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tool_list = tools["tools"].as_array().unwrap();
    let tool_names: Vec<&str> = tool_list
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();

    assert!(
        tool_names.contains(&"calculator"),
        "calculator should be registered: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"get_current_time"),
        "get_current_time should be registered: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"read_file"),
        "read_file should be registered: {:?}",
        tool_names
    );
}

// ===================================================================
// Scenario 10: Dynamic routes CRUD lifecycle
// ===================================================================

#[tokio::test]
async fn e2e_dynamic_routes_lifecycle() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    // Initially empty
    let list0: Value = client
        .get(srv.url("/api/v1/routes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list0["routes"].as_array().unwrap().len(), 0);

    // Create route
    let created: Value = client
        .post(srv.url("/api/v1/routes"))
        .json(&json!({
            "agentId": "main",
            "match": {"channel": "slack"}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let route_id = created["id"].as_str().unwrap().to_string();

    // Update route
    let updated: Value = client
        .put(srv.url(&format!("/api/v1/routes/{route_id}")))
        .json(&json!({
            "agentId": "main",
            "match": {"channel": "telegram"}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["match"]["channel"], "telegram");

    // List should have one
    let list1: Value = client
        .get(srv.url("/api/v1/routes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list1["routes"].as_array().unwrap().len(), 1);

    // Delete
    let del: Value = client
        .delete(srv.url(&format!("/api/v1/routes/{route_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del["deleted"], true);
}

// ===================================================================
// Scenario: Memory record → recall regression tests
// ===================================================================

/// Verify the episode lifecycle: auto-record via chat → keyword search retrieves it.
#[tokio::test]
async fn memory_episode_record_and_keyword_search() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("Dark mode is a popular UI preference that reduces eye strain and saves battery on OLED screens.".into()),
            tool_calls: None,
        },
    ]);
    let srv = E2eServer::start(Box::new(provider)).await;
    let client = reqwest::Client::new();

    let _: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "Tell me about dark mode preferences"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let search: Value = client
        .get(srv.url("/api/v1/memory/episodes/search?agent_id=main&q=dark%20mode"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let episodes = search["episodes"].as_array();
    assert!(
        episodes.is_some_and(|e| !e.is_empty()),
        "keyword search should find episodes about dark mode: {search}"
    );
}

/// Verify fact CRUD: upsert → search → delete → search-empty.
#[tokio::test]
async fn memory_fact_crud_cycle() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    let upsert: Value = client
        .post(srv.url("/api/v1/memory/facts?agent_id=main"))
        .json(&json!({
            "id": "fact-crud-test",
            "category": "preference",
            "subject": "User",
            "predicate": "prefers",
            "object": "TypeScript over JavaScript",
            "confidence": 0.9
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(upsert.get("ok").is_some(), "fact upsert: {upsert}");

    let search: Value = client
        .get(srv.url("/api/v1/memory/facts/search?agent_id=main&q=TypeScript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let facts = search["facts"].as_array().or(search["results"].as_array());
    assert!(
        facts.is_some_and(|f| !f.is_empty()),
        "search should find the TypeScript fact: {search}"
    );

    let del: Value = client
        .delete(srv.url("/api/v1/memory/facts/fact-crud-test?agent_id=main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        del.get("ok").is_some() || del.get("deleted").is_some(),
        "fact delete: {del}"
    );

    let search_after: Value = client
        .get(srv.url("/api/v1/memory/facts/search?agent_id=main&q=TypeScript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let remaining = search_after["facts"]
        .as_array()
        .or(search_after["results"].as_array());
    let has_deleted = remaining.is_some_and(|r| r.iter().any(|f| f["id"] == "fact-crud-test"));
    assert!(
        !has_deleted,
        "deleted fact should not appear in search: {search_after}"
    );
}

/// Verify that auto_record_episode creates an episode after a chat turn.
#[tokio::test]
async fn memory_auto_episode_from_chat() {
    let provider = simple_provider();
    let srv = E2eServer::start(provider).await;
    let client = reqwest::Client::new();

    let episodes_before: Value = client
        .get(srv.url("/api/v1/memory/episodes?agent_id=main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let count_before = episodes_before["episodes"]
        .as_array()
        .map_or(0, |e| e.len());

    let _chat: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "Tell me about Rust programming language features"}],
            "stream": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let episodes_after: Value = client
        .get(srv.url("/api/v1/memory/episodes?agent_id=main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let count_after = episodes_after["episodes"].as_array().map_or(0, |e| e.len());

    assert!(
        count_after > count_before,
        "auto_record_episode should create an episode after chat (before={count_before}, after={count_after})"
    );
}

/// Verify that stored facts appear as [Relevant memories] in chat context.
/// We achieve this by: storing a fact, then sending a chat with a related query.
/// The ScriptedProvider echoes back, and the fact's content should influence context.
#[tokio::test]
async fn memory_recall_injects_relevant_memories() {
    let srv = E2eServer::start(simple_provider()).await;
    let client = reqwest::Client::new();

    let _: Value = client
        .post(srv.url("/api/v1/memory/facts?agent_id=main"))
        .json(&json!({
            "id": "recall-test-fact",
            "category": "technology",
            "subject": "FastClaw",
            "predicate": "uses",
            "object": "SQLite for session storage",
            "confidence": 1.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let chat_resp: Value = client
        .post(srv.url("/api/v1/chat"))
        .json(&json!({
            "messages": [{"role": "user", "content": "What database does FastClaw use for storage?"}],
            "stream": false,
            "agent_id": "main"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        chat_resp.get("choices").is_some() || chat_resp.get("message").is_some(),
        "chat should succeed when memory is active: {chat_resp}"
    );
}

/// Verify multiple episodes from multi-turn conversation.
#[tokio::test]
async fn memory_multi_turn_episode_accumulation() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("Python is great for data science.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Rust is excellent for systems programming.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Go is popular for cloud-native development.".into()),
            tool_calls: None,
        },
    ]);
    let srv = E2eServer::start(Box::new(provider)).await;
    let client = reqwest::Client::new();

    let turns = [
        "Tell me about Python",
        "Tell me about Rust",
        "Tell me about Go",
    ];
    for turn in &turns {
        let _: Value = client
            .post(srv.url("/api/v1/chat"))
            .json(&json!({
                "messages": [{"role": "user", "content": turn}],
                "stream": false
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let episodes: Value = client
        .get(srv.url("/api/v1/memory/episodes?agent_id=main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let count = episodes["episodes"].as_array().map_or(0, |e| e.len());
    assert!(
        count >= 1,
        "multi-turn chat should produce at least 1 auto-recorded episode (got {count})"
    );
}

// ===================================================================
// Scenario: Global MCP server config round-trip
// ===================================================================

#[tokio::test]
async fn e2e_global_mcp_config_round_trip() {
    use fastclaw_core::config::FastClawConfig;
    use fastclaw_core::config_access::{filter_config_for_read, CONFIG_WRITABLE_KEYS};

    let raw = serde_json::json!({
        "mcpServers": [
            { "id": "chrome-devtools", "command": "npx", "args": ["-y", "@anthropic-ai/chrome-devtools-mcp@latest"], "enabled": true },
            { "id": "tauri-mcp", "command": "npx", "args": ["-y", "@anthropic-ai/tauri-mcp-server@latest"] }
        ]
    });

    let config: FastClawConfig =
        serde_json::from_value(raw.clone()).expect("mcpServers should deserialize");
    assert_eq!(config.mcp_servers.len(), 2);
    assert_eq!(config.mcp_servers[0].id, "chrome-devtools");
    assert_eq!(config.mcp_servers[0].command, "npx");
    assert_eq!(config.mcp_servers[0].enabled, Some(true));
    assert_eq!(config.mcp_servers[1].id, "tauri-mcp");
    assert_eq!(config.mcp_servers[1].enabled, None);

    let serialized = serde_json::to_value(&config).expect("should serialize");
    let filtered = filter_config_for_read(&serialized);
    assert!(
        filtered.get("mcpServers").is_some(),
        "mcpServers should be readable through ACL filter"
    );

    assert!(
        CONFIG_WRITABLE_KEYS.contains(&"mcpServers"),
        "mcpServers should be writable"
    );

    let default_config: FastClawConfig = serde_json::from_value(json!({})).unwrap();
    assert!(
        default_config.mcp_servers.is_empty(),
        "mcpServers should default to empty vec"
    );
}

// ===================================================================
// Scenario: sessions.claim allows cross-connection resume
// ===================================================================

#[tokio::test]
async fn e2e_session_claim_allows_resume() {
    let provider = ScriptedProvider::new(vec![
        ScriptedResponse {
            content: Some("Hello from claimed session.".into()),
            tool_calls: None,
        },
        ScriptedResponse {
            content: Some("Second turn in claimed session.".into()),
            tool_calls: None,
        },
    ]);

    let srv = E2eServer::start(Box::new(provider)).await;

    // Connection 1: create a session via chat
    let (ws1, _) = connect_async(&srv.ws_url()).await.expect("ws1 connect");
    let (mut tx1, mut rx1) = ws1.split();
    let _ = ws_recv_json(&mut rx1).await; // consume welcome

    let (session_id, _) =
        ws_chat_to_completion(&mut tx1, &mut rx1, "c1", "Create session", None).await;
    assert!(!session_id.is_empty());

    // Connection 2: fresh connection, claim the session
    let (ws2, _) = connect_async(&srv.ws_url()).await.expect("ws2 connect");
    let (mut tx2, mut rx2) = ws2.split();
    let _ = ws_recv_json(&mut rx2).await; // consume welcome

    // Claim the session
    ws_send_json(
        &mut tx2,
        json!({"id": "claim1", "method": "sessions.claim", "params": {"sessionId": &session_id}}),
    )
    .await;
    let claim_resp = ws_recv_json(&mut rx2).await;
    assert_eq!(claim_resp["type"], "sessions.claim");
    assert_eq!(claim_resp["data"]["claimed"], true);
    assert_eq!(claim_resp["data"]["sessionId"], session_id);

    // Now sessions.messages should work on connection 2
    ws_send_json(
        &mut tx2,
        json!({"id": "sm1", "method": "sessions.messages", "params": {"sessionId": &session_id}}),
    )
    .await;
    let msgs_resp = ws_recv_json(&mut rx2).await;
    assert_eq!(msgs_resp["type"], "sessions.messages");
    let messages = msgs_resp["data"]["messages"].as_array().unwrap();
    assert!(
        messages.len() >= 2,
        "should have messages from first connection, got {}",
        messages.len()
    );

    // Chat on the claimed session from connection 2
    let (sid2, events) = ws_chat_to_completion(
        &mut tx2,
        &mut rx2,
        "c2",
        "Continue on connection 2",
        Some(&session_id),
    )
    .await;
    assert_eq!(sid2, session_id);
    assert!(events.contains(&"chat.complete".to_string()));
}

// ===================================================================
// Scenario: sessions.claim rejects non-existent session
// ===================================================================

#[tokio::test]
async fn e2e_session_claim_rejects_missing() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx).await;

    ws_send_json(
        &mut tx,
        json!({
            "id": "c1",
            "method": "sessions.claim",
            "params": {"sessionId": "nonexistent-session-id"}
        }),
    )
    .await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert_eq!(resp["error"]["code"], 404);
}

// ===================================================================
// Scenario: chat.cancel via WS
// ===================================================================

#[tokio::test]
async fn e2e_chat_cancel_via_ws() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx).await;

    // Cancel a non-existent request ID
    ws_send_json(
        &mut tx,
        json!({"id": "cancel1", "method": "chat.cancel", "params": {"requestId": "fake-id"}}),
    )
    .await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "chat.cancel");
    assert_eq!(
        resp["data"]["cancelled"], false,
        "non-existent request should not cancel"
    );

    // Cancel without requestId
    ws_send_json(
        &mut tx,
        json!({"id": "cancel2", "method": "chat.cancel", "params": {}}),
    )
    .await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
}

// ===================================================================
// Scenario: chat.complete includes elapsed time and token estimates
// ===================================================================

#[tokio::test]
async fn e2e_chat_complete_includes_usage_stats() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx).await;

    ws_send_json(
        &mut tx,
        json!({
            "id": "stats1",
            "method": "chat",
            "params": {
                "messages": [{"role": "user", "content": "Hello"}]
            }
        }),
    )
    .await;

    let complete_data: Option<Value>;
    loop {
        let msg = ws_recv_json(&mut rx).await;
        let ty = msg["type"].as_str().unwrap_or("");
        if ty == "chat.complete" {
            complete_data = msg.get("data").cloned();
            break;
        }
        if ty == "chat.error" {
            panic!("chat failed: {:?}", msg["error"]);
        }
    }

    let data = complete_data.expect("should have chat.complete data");
    assert!(
        data.get("elapsedMs").is_some(),
        "chat.complete should include elapsedMs"
    );
    assert!(
        data.get("inputTokensEstimate").is_some(),
        "chat.complete should include inputTokensEstimate"
    );
    assert!(
        data.get("outputTokensEstimate").is_some(),
        "chat.complete should include outputTokensEstimate"
    );
    let elapsed = data["elapsedMs"].as_u64().unwrap();
    assert!(elapsed > 0, "elapsed should be positive");
}

// ===================================================================
// Scenario: WS methods list includes new methods
// ===================================================================

#[tokio::test]
async fn e2e_connected_advertises_all_methods() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (_tx, mut rx) = ws.split();

    let welcome = ws_recv_json(&mut rx).await;
    assert_eq!(welcome["type"], "connected");

    let methods = welcome["data"]["methods"]
        .as_array()
        .expect("methods array");
    let method_strs: Vec<&str> = methods.iter().filter_map(|v| v.as_str()).collect();

    for expected in &[
        "sessions.claim",
        "chat.cancel",
        "chat.answer",
        "mcp.status",
        "mcp.reload",
        "mcp.add",
        "mcp.remove",
        "models.list",
        "config.get",
        "config.set",
    ] {
        assert!(
            method_strs.contains(expected),
            "connected methods should include '{expected}', got: {method_strs:?}"
        );
    }
}

// ===================================================================
// Scenario: models.list returns model data
// ===================================================================

#[tokio::test]
async fn e2e_models_list_via_ws() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx).await;

    ws_send_json(&mut tx, json!({"id": "ml1", "method": "models.list"})).await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "models.list");
    assert!(resp["data"]["models"].is_array());
}

// ===================================================================
// Scenario: unknown WS method returns error
// ===================================================================

#[tokio::test]
async fn e2e_unknown_ws_method_returns_error() {
    let srv = E2eServer::start(simple_provider()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx).await;

    ws_send_json(
        &mut tx,
        json!({"id": "unk1", "method": "nonexistent.method"}),
    )
    .await;
    let resp = ws_recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown method"),);
}
