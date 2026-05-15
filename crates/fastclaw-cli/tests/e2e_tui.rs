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

// ── Mock Gateway ────────────────────────────────────────────────────

struct MockGateway {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Value>>>,
    _handle: tokio::task::JoinHandle<()>,
}

/// Scenario determines what the mock gateway does for "chat" requests.
#[derive(Clone)]
enum ChatScenario {
    /// Normal: start -> delta -> complete
    Normal,
    /// Full flow with tool call: start -> delta -> tool.start -> tool.progress -> tool.done -> delta -> complete
    WithToolCall,
    /// Sub-agent: start -> subagent.start -> subagent.delta -> subagent.complete -> delta -> complete
    WithSubAgent,
    /// Error mid-stream: start -> delta -> error
    ErrorMidStream,
    /// Slow response: start -> (3s delay) -> delta -> complete
    SlowResponse,
    /// Hang forever: start only, never complete
    Hang,
    /// Context warning: start -> context.warning -> delta -> complete
    WithContextWarning,
    /// Multiple deltas with suggestions: start -> delta*5 -> suggestions -> complete
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
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid,"model":"test-model"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Hello from mock!"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":500,"inputTokensEstimate":20,"outputTokensEstimate":10}}).to_string())).await;
                                    }
                                    ChatScenario::WithToolCall => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Let me check the file.\n"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.tool.start","data":{"tool":"file_read","callId":"c1","params":{"path":"/src/main.rs"}}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.tool.progress","data":{"content":"reading 1024 bytes"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.tool.done","data":{"success":true,"elapsedMs":200}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"The file contains a main function."}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":1500,"inputTokensEstimate":100,"outputTokensEstimate":50}}).to_string())).await;
                                    }
                                    ChatScenario::WithSubAgent => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.subagent.start","data":{"runId":"r1","label":"code-review"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.subagent.delta","data":{"runId":"r1","content":"Reviewing code..."}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.subagent.tool.start","data":{"runId":"r1","tool":"grep","args":{"pattern":"TODO"}}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.subagent.tool.done","data":{"runId":"r1","success":true}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.subagent.complete","data":{"runId":"r1","elapsedMs":5000}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Sub-agent found 3 TODOs."}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":6000,"inputTokensEstimate":200,"outputTokensEstimate":80}}).to_string())).await;
                                    }
                                    ChatScenario::ErrorMidStream => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Starting to process..."}}).to_string())).await;
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.error","error":{"message":"upstream provider timeout"}}).to_string())).await;
                                    }
                                    ChatScenario::SlowResponse => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        tokio::time::sleep(Duration::from_secs(3)).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Sorry for the delay!"}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":3200,"inputTokensEstimate":30,"outputTokensEstimate":15}}).to_string())).await;
                                    }
                                    ChatScenario::Hang => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        // Never sends complete — simulates hanging
                                    }
                                    ChatScenario::WithContextWarning => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.context.warning","data":{"usedPercent":90.5}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.context_usage","data":{"usedTokens":115000,"limitTokens":128000}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":"Context is almost full."}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":800,"inputTokensEstimate":50,"outputTokensEstimate":20}}).to_string())).await;
                                    }
                                    ChatScenario::WithSuggestions => {
                                        let _ = tx.send(Message::Text(json!({"type":"chat.start","data":{"sessionId":sid}}).to_string())).await;
                                        for i in 1..=5 {
                                            let _ = tx.send(Message::Text(json!({"type":"chat.delta","data":{"content":format!("chunk{i} ")}}).to_string())).await;
                                        }
                                        let _ = tx.send(Message::Text(json!({"type":"chat.suggestions","data":{"items":["add tests","refactor code","fix bug"]}}).to_string())).await;
                                        let _ = tx.send(Message::Text(json!({"type":"chat.complete","data":{"elapsedMs":2000,"inputTokensEstimate":60,"outputTokensEstimate":40}}).to_string())).await;
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
                            "chat.cancel" => {
                                let target = parsed["params"]["requestId"].as_str().unwrap_or("");
                                let resp = json!({
                                    "type": "chat.cancel",
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
    assert_eq!(types, vec!["chat.start", "chat.delta", "chat.complete"]);
    assert_eq!(msgs[0]["data"]["sessionId"], "test-session-001");
    assert_eq!(msgs[1]["data"]["content"], "Hello from mock!");
    assert_eq!(msgs[2]["data"]["elapsedMs"], 500);

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
            "chat.start",
            "chat.delta",
            "chat.tool.start",
            "chat.tool.progress",
            "chat.tool.done",
            "chat.delta",
            "chat.complete"
        ]
    );
    assert_eq!(msgs[2]["data"]["tool"], "file_read");
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
    assert!(types.contains(&"chat.subagent.start"));
    assert!(types.contains(&"chat.subagent.delta"));
    assert!(types.contains(&"chat.subagent.complete"));
    assert!(types.last() == Some(&"chat.complete"));
}

#[tokio::test]
async fn e2e_error_mid_stream() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::ErrorMidStream).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "do something"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 4, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"chat.start"));
    assert!(types.contains(&"chat.delta"));
    assert!(types.contains(&"chat.error"));
    // Should NOT contain "chat.complete" — error terminates the stream
    assert!(!types.contains(&"chat.complete"));

    let error_msg = msgs
        .iter()
        .find(|v| v["type"] == "chat.error")
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
    assert_eq!(types, vec!["chat.start", "chat.delta", "chat.complete"]);
}

#[tokio::test]
async fn e2e_hang_timeout_only_start() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::Hang).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "hang"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    // Should only receive chat.start, then nothing within 2s
    let msgs = collect_messages(&mut rx, 2, Duration::from_secs(2)).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["type"], "chat.start");
}

#[tokio::test]
async fn e2e_context_warning_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithContextWarning).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "context test"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 5, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"chat.context.warning"));
    assert!(types.contains(&"chat.context_usage"));
    assert!(types.contains(&"chat.complete"));
}

#[tokio::test]
async fn e2e_suggestions_flow() {
    let mock = start_mock_gateway_with_scenario(ChatScenario::WithSuggestions).await;
    let (mut tx, mut rx) = connect_and_handshake(mock.addr).await;

    let req = json!({"id": "r1", "method": "chat", "params": {"messages": [{"role": "user", "content": "suggest"}]}});
    tx.send(Message::Text(req.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 8, Duration::from_secs(2)).await;
    let types: Vec<&str> = msgs.iter().filter_map(|v| v["type"].as_str()).collect();
    assert!(types.contains(&"chat.suggestions"));

    let suggestion_msg = msgs.iter().find(|v| v["type"] == "chat.suggestions").unwrap();
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

    // Consume chat.start
    let _ = collect_messages(&mut rx, 1, Duration::from_secs(1)).await;

    // Send cancel
    let cancel = json!({"id": "cancel-1", "method": "chat.cancel", "params": {"requestId": "r1"}});
    tx.send(Message::Text(cancel.to_string())).await.unwrap();

    let msgs = collect_messages(&mut rx, 1, Duration::from_secs(2)).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["type"], "chat.cancel");
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
        assert_eq!(msgs.last().unwrap()["type"], "chat.complete");
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
