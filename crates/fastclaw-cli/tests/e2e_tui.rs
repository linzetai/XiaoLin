//! End-to-end tests for the TUI using a mock WebSocket gateway.
//!
//! These tests spin up a lightweight WS server that speaks the FastClaw gateway
//! protocol, then drive WebSocket clients against it to verify the full message flow.
//! Tests cover: normal chat flow, tool calls, sub-agent delegation, error recovery,
//! slow responses, connection drops, and concurrent clients.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

fn content_delta(content: &str) -> Value {
    json!({
        "type": "content_delta",
        "data": {
            "delta": {
                "choices": [{"delta": {"content": content}}]
            }
        }
    })
}

fn content_delta_reasoning(content: &str) -> Value {
    json!({
        "type": "content_delta",
        "data": {
            "delta": {
                "choices": [{"delta": {"reasoning_content": content}}]
            }
        }
    })
}

fn turn_start(session_id: &str, model: Option<&str>) -> Value {
    let mut data = json!({"session_id": session_id});
    if let Some(m) = model {
        data["model"] = json!(m);
    }
    json!({"type": "turn_start", "data": data})
}

fn turn_end(elapsed_ms: u64) -> Value {
    json!({
        "type": "turn_end",
        "data": {
            "summary": {
                "turn_id": "t1",
                "tool_calls_made": 0,
                "iterations": 0,
                "elapsed_ms": elapsed_ms
            }
        }
    })
}

// ── Mock Gateway ────────────────────────────────────────────────────

struct MockGateway {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Value>>>,
    _handle: tokio::task::JoinHandle<()>,
}

/// Scenario determines what the mock gateway does for "chat" requests.
#[derive(Clone)]
enum ChatScenario {
    /// Normal: turn_start -> content_delta -> turn_end
    Normal,
    /// Full flow with tool call: turn_start -> content_delta -> tool_executing -> tool_progress -> tool_result -> content_delta -> turn_end
    WithToolCall,
    /// Sub-agent: turn_start -> sub_agent_* -> content_delta -> turn_end
    WithSubAgent,
    /// Error mid-stream: turn_start -> content_delta -> error
    ErrorMidStream,
    /// Slow response: turn_start -> (3s delay) -> content_delta -> turn_end
    SlowResponse,
    /// Hang forever: turn_start only, never complete
    Hang,
    /// Context warning: turn_start -> context_warning -> context_usage_update -> content_delta -> turn_end
    WithContextWarning,
    /// Multiple deltas with suggestions: turn_start -> content_delta*5 -> suggestions -> turn_end
    WithSuggestions,
}

async fn start_mock_gateway_with_scenario(scenario: ChatScenario) -> MockGateway {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let recv_clone = received.clone();

    let handle = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let recv = recv_clone.clone();
            let scenario = scenario.clone();
            tokio::spawn(async move {
                let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut tx, mut rx) = ws_stream.split();

                let connected_msg = json!({"type": "connected", "data": {"version":"0.0.6-test","protocol":"fastclaw-ws/1"}});
                let _ = tx.send(Message::Text(connected_msg.to_string())).await;

                while let Some(Ok(msg)) = rx.next().await {
                    if let Message::Text(text) = msg {
                        let parsed: Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let method = parsed["method"].as_str().unwrap_or("");
                        let req_id = parsed["id"].as_str().unwrap_or("").to_string();

                        recv.lock().await.push(parsed.clone());

                        match method {
                            "agents" => {
                                let resp = json!({
                                    "id": req_id,
                                    "type": "agents",
                                    "data": {
                                        "agents": [
                                            {"agentId": "default", "name": "Default Agent", "model": "test-model"},
                                            {"agentId": "coder", "name": "Coder", "model": "gpt-4o"}
                                        ]
                                    }
                                });
                                let _ = tx.send(Message::Text(resp.to_string())).await;
                            }
                            "chat" => {
                                let sid = "test-session-001";
                                match &scenario {
                                    ChatScenario::Normal => {
                                        let _ = tx.send(Message::Text(turn_start(sid, Some("test-model")).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("Hello from mock!").to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(500).to_string())).await;
                                    }
                                    ChatScenario::WithToolCall => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("Let me check the file.\n").to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"tool_executing","data":{"tool_name":"file_read","call_id":"c1","args":"{\"path\":\"/src/main.rs\"}"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"tool_progress","data":{"content":"reading 1024 bytes"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"tool_result","data":{"success":true}}).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("The file contains a main function.").to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(1500).to_string())).await;
                                    }
                                    ChatScenario::WithSubAgent => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"sub_agent_start","data":{"run_id":"r1","label":"code-review"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"sub_agent_delta","data":{"run_id":"r1","content":"Reviewing code..."}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"sub_agent_tool_executing","data":{"run_id":"r1","tool":"grep","args":{"pattern":"TODO"}}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"sub_agent_tool_result","data":{"run_id":"r1","success":true}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"sub_agent_complete","data":{"run_id":"r1","elapsed_ms":5000}}).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("Sub-agent found 3 TODOs.").to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(6000).to_string())).await;
                                    }
                                    ChatScenario::ErrorMidStream => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("Starting to process...").to_string())).await;
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                        let _ = tx.send(Message::Text(json!({"type":"error","data":{"message":"upstream provider timeout"},"error":{"message":"upstream provider timeout"}}).to_string())).await;
                                    }
                                    ChatScenario::SlowResponse => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        tokio::time::sleep(Duration::from_secs(3)).await;
                                        let _ = tx.send(Message::Text(content_delta("Sorry for the delay!").to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(3200).to_string())).await;
                                    }
                                    ChatScenario::Hang => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        // Never sends turn_end — simulates hanging
                                    }
                                    ChatScenario::WithContextWarning => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"context_warning","data":{"used_percent":90.5}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"context_usage_update","data":{"used_tokens":115000,"limit_tokens":128000}}).to_string())).await;
                                        let _ = tx.send(Message::Text(content_delta("Context is almost full.").to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(800).to_string())).await;
                                    }
                                    ChatScenario::WithSuggestions => {
                                        let _ = tx.send(Message::Text(turn_start(sid, None).to_string())).await;
                                        for i in 1..=5 {
                                            let _ = tx.send(Message::Text(content_delta(&format!("chunk{i} ")).to_string())).await;
                                        }
                                        let _ = tx.send(Message::Text(json!({"type":"suggestions","data":{"items":["add tests","refactor code","fix bug"]}}).to_string())).await;
                                        let _ = tx.send(Message::Text(turn_end(2000).to_string())).await;
                                    }
                                }
                            }
                            "sessions.list" => {
                                let resp = json!({
                                    "type": "sessions.list",
                                    "data": {
                                        "sessions": [
                                            {"id": "s1", "title": "Test Session", "createdAt": "2025-01-01T00:00:00Z"}
                                        ]
                                    }
                                });
                                let _ = tx.send(Message::Text(resp.to_string())).await;
                            }
                            "cancel" => {
                                let target = parsed["params"]["requestId"].as_str().unwrap_or("");
                                let resp = json!({
                                    "type": "cancel",
                                    "data": {"requestId": target, "cancelled": true}
                                });
                                let _ = tx.send(Message::Text(resp.to_string())).await;
                            }
                            _ => {}
                        }
                    }
                }
            });
        }
    });

    MockGateway {
        addr,
        received,
        _handle: handle,
    }
}

async fn start_mock_gateway() -> MockGateway {
    start_mock_gateway_with_scenario(ChatScenario::Normal).await
}

/// Helper: connect to a mock gateway and consume the initial "connected" message.
async fn connect_and_handshake(
    addr: SocketAddr,
) -> (
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >,
    futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
) {
    let url = format!("ws://127.0.0.1:{}/ws", addr.port());
    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (tx, mut rx) = ws_stream.split();
    // Consume "connected"
    let msg = tokio::time::timeout(Duration::from_secs(2), rx.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let Message::Text(text) = msg {
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["type"], "connected");
    }
    (tx, rx)
}

/// Helper: collect N text messages within timeout.
async fn collect_messages(
    rx: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
    n: usize,
    timeout: Duration,
) -> Vec<Value> {
    let mut results = Vec::new();
    for _ in 0..n {
        match tokio::time::timeout(timeout, rx.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    results.push(v);
                }
            }
            _ => break,
        }
    }
    results
}

// ── E2E Tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_connect_to_mock_gateway() {
    let mock = start_mock_gateway().await;
    let url = format!("ws://127.0.0.1:{}/ws", mock.addr.port());

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("should connect to mock gateway");

    let (_tx, mut rx) = ws_stream.split();
    let msg = tokio::time::timeout(Duration::from_secs(2), rx.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("ws error");

    if let Message::Text(text) = msg {
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["type"], "connected");
        assert_eq!(v["data"]["protocol"], "fastclaw-ws/1");
    } else {
        panic!("expected text message");
    }
}

#[tokio::test]
async fn e2e_normal_chat_flow() {
    let mock = start_mock_gateway().await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "hello"}], "agentId": "default"}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 3, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert_eq!(types, vec!["turn_start", "content_delta", "turn_end"]);
    assert_eq!(msgs[0]["data"]["session_id"], "test-session-001");
    assert_eq!(
        msgs[1]["data"]["delta"]["choices"][0]["delta"]["content"],
        "Hello from mock!"
    );
    assert_eq!(msgs[2]["data"]["summary"]["elapsed_ms"], 500);

    let received = mock.received.lock().await;
    assert!(received.iter().any(|v| v["method"] == "chat"));
}

#[tokio::test]
async fn e2e_tool_call_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithToolCall).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "read the file"}], "agentId": "default"}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 7, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert_eq!(
        types,
        vec![
            "turn_start",
            "content_delta",
            "tool_executing",
            "tool_progress",
            "tool_result",
            "content_delta",
            "turn_end"
        ]
    );
    assert_eq!(msgs[2]["data"]["tool_name"], "file_read");
    assert!(msgs[4]["data"]["success"].as_bool().unwrap());
}

#[tokio::test]
async fn e2e_subagent_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithSubAgent).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "review code"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 8, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"sub_agent_start"));
    assert!(types.contains(&"sub_agent_delta"));
    assert!(types.contains(&"sub_agent_complete"));
    assert!(types.last() == Some(&"turn_end"));
}

#[tokio::test]
async fn e2e_error_mid_stream() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::ErrorMidStream).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "do something"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 4, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"turn_start"));
    assert!(types.contains(&"content_delta"));
    assert!(types.contains(&"error"));
    // Should NOT contain "turn_end" — error terminates the stream
    assert!(!types.contains(&"turn_end"));

    let error_msg = msgs
        .iter()
        .find(|v| v["type"] == "error")
        .unwrap();
    assert_eq!(error_msg["error"]["message"], "upstream provider timeout");
}

#[tokio::test]
async fn e2e_slow_response_eventually_completes() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::SlowResponse).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "slow please"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    // Should get start quickly, then wait ~3s for delta + complete
    let msgs = collect_messages(&mut rx, 3, Duration::from_secs(5)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert_eq!(types, vec!["turn_start", "content_delta", "turn_end"]);
}

#[tokio::test]
async fn e2e_hang_timeout_only_start() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::Hang).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "hang"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    // Should only receive turn_start, then nothing within 2s
    let msgs = collect_messages(&mut rx, 2, Duration::from_secs(2)).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["type"], "turn_start");
}

#[tokio::test]
async fn e2e_context_warning_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithContextWarning).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "context test"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 5, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"context_warning"));
    assert!(types.contains(&"context_usage_update"));
    assert!(types.contains(&"turn_end"));
}

#[tokio::test]
async fn e2e_suggestions_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithSuggestions).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "suggest"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 8, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"suggestions"));

    let suggestion_msg = msgs.iter().find(|v| v["type"] == "suggestions").unwrap();
    let items = suggestion_msg["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn e2e_request_agents_list() {
    let mock = start_mock_gateway().await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "req-agents", "method": "agents"});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 1, Duration::from_secs(2)).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["type"], "agents");
    let agents = msgs[0]["data"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2);
}

#[tokio::test]
async fn e2e_cancel_chat() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::Hang).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    // Start a hanging chat
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "hang"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    // Consume turn_start
    let _ = collect_messages(&mut rx, 1, Duration::from_secs(1)).await;

    // Send cancel
    let cancel = json!({"id": "cancel-1", "method": "cancel", "params": {"requestId": "r1"}});
    tx.send(Message::Text(cancel.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 1, Duration::from_secs(2)).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["type"], "cancel");
    assert!(msgs[0]["data"]["cancelled"].as_bool().unwrap());
}

#[tokio::test]
async fn e2e_disconnect_gracefully() {
    let mock = start_mock_gateway().await;
    let url = format!("ws://127.0.0.1:{}/ws", mock.addr.port());

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (mut tx, _rx) = ws_stream.split();

    tx.send(Message::Close(None)).await.unwrap();
}

#[tokio::test]
async fn e2e_multiple_sequential_chats() {
    let mock = start_mock_gateway().await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    for i in 1..=3 {
        let req = json!({"id": format!("r{i}"), "method": "chat", "params": {"messages": [{"role": "user", "content": format!("message {i}")}]}});
        tx.send(Message::Text(req.to_string())).await.unwrap();
        let msgs = collect_messages(&mut rx, 3, Duration::from_secs(2)).await;
        assert_eq!(msgs.len(), 3, "turn {i} should have 3 messages");
        assert_eq!(msgs.last().unwrap()["type"], "turn_end");
    }

    let received = mock.received.lock().await;
    let chat_count = received.iter().filter(|v| v["method"] == "chat").count();
    assert_eq!(chat_count, 3);
}

#[tokio::test]
async fn e2e_concurrent_clients() {
    let mock = start_mock_gateway().await;

    let mut handles = Vec::new();
    for i in 0..3 {
        let addr = mock.addr;
        handles.push(tokio::spawn(async move {
            let (mut tx, mut rx) = connect_and_handshake(addr).await;
            let req = json!({"id": format!("c{i}"), "method": "chat", "params": {"messages": [{"role": "user", "content": "concurrent"}]}});
            tx.send(Message::Text(req.to_string())).await.unwrap();
            let msgs = collect_messages(&mut rx, 3, Duration::from_secs(2)).await;
            assert_eq!(msgs.len(), 3, "client {i} should get full chat flow");
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
}

#[tokio::test]
async fn e2e_embedded_gateway_probe() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await;
    assert!(resp.is_err(), "should fail since nothing is listening");
}

// ── Edge-case E2E tests ─────────────────────────────────────────────

#[tokio::test]
async fn e2e_cjk_tool_params_no_truncation_panic() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        while let Some(Ok(msg)) = rx.next().await {
            if let Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
                if parsed["method"] == "chat" {
                    let cjk_path = "你".repeat(100);
                    let args = serde_json::to_string(&json!({"path": cjk_path})).unwrap();
                    let _ = tx.send(Message::Text(turn_start("s1", None).to_string())).await;
                    let _ = tx.send(Message::Text(json!({"type":"tool_executing","data":{"tool_name":"file_read","call_id":"c1","args": args}}).to_string())).await;
                    let _ = tx.send(Message::Text(json!({"type":"tool_result","data":{"success":true}}).to_string())).await;
                    let _ = tx.send(Message::Text(turn_end(100).to_string())).await;
                    break;
                }
            }
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "read cjk file"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 4, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"tool_executing"));
    assert!(types.contains(&"turn_end"));
    handle.await.unwrap();
}

#[tokio::test]
async fn e2e_rapid_delta_flood() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        while let Some(Ok(msg)) = rx.next().await {
            if let Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
                if parsed["method"] == "chat" {
                    let _ = tx.send(Message::Text(turn_start("s1", None).to_string())).await;
                    for i in 0..200 {
                        let _ = tx.send(Message::Text(content_delta(&format!("w{i} ")).to_string())).await;
                    }
                    let _ = tx.send(Message::Text(turn_end(500).to_string())).await;
                    break;
                }
            }
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "flood"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 202, Duration::from_secs(5)).await;
    assert_eq!(msgs.last().unwrap()["type"], "turn_end");
    let delta_count = msgs.iter().filter(|v| v["type"] == "content_delta").count();
    assert_eq!(delta_count, 200);
    handle.await.unwrap();
}

#[tokio::test]
async fn e2e_server_abrupt_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        if let Some(Ok(_)) = rx.next().await {
            let _ = tx.send(Message::Text(turn_start("s1", None).to_string())).await;
            let _ = tx.send(Message::Text(content_delta("partial...").to_string())).await;
            // Abruptly drop: just return without sending Close frame
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "abort"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 10, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"turn_start"));
    assert!(types.contains(&"content_delta"));
    assert!(!types.contains(&"turn_end"));
    handle.await.unwrap();
}

#[tokio::test]
async fn e2e_thinking_then_content_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        while let Some(Ok(msg)) = rx.next().await {
            if let Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
                if parsed["method"] == "chat" {
                    let _ = tx.send(Message::Text(turn_start("s1", None).to_string())).await;
                    let _ = tx.send(Message::Text(content_delta_reasoning("Let me think step by step...\n1. First\n2. Second").to_string())).await;
                    let _ = tx.send(Message::Text(content_delta("The answer is 42.").to_string())).await;
                    let _ = tx.send(Message::Text(turn_end(300).to_string())).await;
                    break;
                }
            }
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "think and answer"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 4, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert_eq!(types, vec!["turn_start", "content_delta", "content_delta", "turn_end"]);
    assert!(msgs[1]["data"]["delta"]["choices"][0]["delta"]["reasoning_content"].is_string());
    assert!(msgs[2]["data"]["delta"]["choices"][0]["delta"]["content"].is_string());
    handle.await.unwrap();
}

#[tokio::test]
async fn e2e_error_codes_propagated() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        while let Some(Ok(msg)) = rx.next().await {
            if let Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
                if parsed["method"] == "chat" {
                    let _ = tx.send(Message::Text(json!({"type":"error","error":{"message":"rate limited","code":429}}).to_string())).await;
                    break;
                }
            }
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "hi"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 1, Duration::from_secs(2)).await;
    assert_eq!(msgs[0]["type"], "error");
    assert_eq!(msgs[0]["error"]["code"], 429);
    handle.await.unwrap();
}

#[tokio::test]
async fn e2e_large_payload_single_message() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws_stream.split();

        let _ = tx.send(Message::Text(json!({"type":"connected","data":{"version":"0.0.6","protocol":"fastclaw-ws/1"}}).to_string())).await;

        while let Some(Ok(msg)) = rx.next().await {
            if let Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
                if parsed["method"] == "chat" {
                    let big_content = "A".repeat(50_000);
                    let _ = tx.send(Message::Text(turn_start("s1", None).to_string())).await;
                    let _ = tx.send(Message::Text(content_delta(&big_content).to_string())).await;
                    let _ = tx.send(Message::Text(turn_end(200).to_string())).await;
                    break;
                }
            }
        }
    });

    let (mut tx, mut rx) = connect_and_handshake(addr).await;
    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "big"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 3, Duration::from_secs(3)).await;
    assert_eq!(msgs.len(), 3);
    let content = msgs[1]["data"]["delta"]["choices"][0]["delta"]["content"]
        .as_str()
        .unwrap();
    assert_eq!(content.len(), 50_000);
    handle.await.unwrap();
}
