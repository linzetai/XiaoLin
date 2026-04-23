use std::net::SocketAddr;

use fastclaw_agent::{CompletionParams, LlmProvider};
use fastclaw_core::types::{ChatChoice, ChatResponse, DeltaContent, StreamChoice, StreamDelta};
use fastclaw_gateway::{build_app, AppState};
use fastclaw_security::{ApiKeyAuth, AuthConfig};
use serde_json::{json, Value};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Mock LLM provider that returns a fixed response without network access
// ---------------------------------------------------------------------------

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
                message: fastclaw_core::types::ChatMessage {
                    role: fastclaw_core::types::Role::Assistant,
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
                        role: Some(fastclaw_core::types::Role::Assistant),
                        content: Some("Mock streamed".into()),
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

// ---------------------------------------------------------------------------
// Test harness: spin up a real TCP server with mock LLM
// ---------------------------------------------------------------------------

struct TestServer {
    addr: SocketAddr,
    _tmp: tempfile::TempDir,
}

impl TestServer {
    async fn start(auth: ApiKeyAuth) -> Self {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let state = AppState::for_test(Box::new(MockProvider), tmp.path())
            .await
            .expect("build test AppState");
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

    fn http_url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn ws_url(&self, query: &str) -> String {
        let q = if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        };
        format!("ws://{}/ws{}", self.addr, q)
    }
}

fn auth_disabled() -> ApiKeyAuth {
    ApiKeyAuth::new(&AuthConfig {
        enabled: false,
        api_keys: vec![],
    })
}

fn auth_enabled(keys: Vec<&str>) -> ApiKeyAuth {
    ApiKeyAuth::new(&AuthConfig {
        enabled: true,
        api_keys: keys.into_iter().map(String::from).collect(),
    })
}

// ---------------------------------------------------------------------------
// HTTP integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_chat_invalid_json_returns_400_app_error_shape() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(srv.http_url("/api/v1/chat"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{not-json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    let err = body
        .get("error")
        .and_then(|v| v.as_object())
        .expect("error object");
    assert_eq!(err.get("type").and_then(|v| v.as_str()), Some("bad_request"));
    let msg = err
        .get("message")
        .and_then(|v| v.as_str())
        .expect("message");
    assert!(!msg.is_empty());
}

#[tokio::test]
async fn http_health_no_auth() {
    let srv = TestServer::start(auth_disabled()).await;
    let resp = reqwest::get(srv.http_url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn http_health_with_auth_enabled() {
    let srv = TestServer::start(auth_enabled(vec!["key1"])).await;
    let resp = reqwest::get(srv.http_url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200, "/health should bypass auth");
}

#[tokio::test]
async fn http_auth_bearer_required() {
    let srv = TestServer::start(auth_enabled(vec!["secret"])).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(srv.http_url("/api/v1/agents"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "missing key should be 401");

    let resp = client
        .get(srv.http_url("/api/v1/agents"))
        .header("Authorization", "Bearer wrong-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "wrong key should be 401");

    let resp = client
        .get(srv.http_url("/api/v1/agents"))
        .header("Authorization", "Bearer secret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "correct key should pass");
}

#[tokio::test]
async fn http_auth_x_api_key_header() {
    let srv = TestServer::start(auth_enabled(vec!["mykey"])).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(srv.http_url("/api/v1/tools"))
        .header("X-API-Key", "mykey")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn http_agents_and_tools() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .get(srv.http_url("/api/v1/agents"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agents = resp["agents"].as_array().unwrap();
    assert!(!agents.is_empty());
    assert_eq!(agents[0]["agentId"], "main");

    let resp: Value = client
        .get(srv.http_url("/api/v1/tools"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(resp["tools"].is_array());
}

#[tokio::test]
async fn http_runtime_routes_crud() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();

    let list0: Value = client
        .get(srv.http_url("/api/v1/routes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list0["routes"].as_array().unwrap().len(), 0);

    let created: Value = client
        .post(srv.http_url("/api/v1/routes"))
        .json(&json!({
            "agentId": "main",
            "match": { "channel": "telegram" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["agentId"], "main");

    let list1: Value = client
        .get(srv.http_url("/api/v1/routes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list1["routes"].as_array().unwrap().len(), 1);

    let updated: Value = client
        .put(srv.http_url(&format!("/api/v1/routes/{id}")))
        .json(&json!({
            "agentId": "main",
            "match": { "channel": "discord" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["match"]["channel"], "discord");

    let del: Value = client
        .delete(srv.http_url(&format!("/api/v1/routes/{id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del["deleted"], true);

    let list2: Value = client
        .get(srv.http_url("/api/v1/routes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list2["routes"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_session_lifecycle() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();

    let chat_body = json!({
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });
    let resp: Value = client
        .post(srv.http_url("/api/v1/chat"))
        .json(&chat_body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session_id = resp["_meta"]["sessionId"].as_str().unwrap().to_string();
    assert!(!session_id.is_empty());

    let resp: Value = client
        .get(srv.http_url("/api/v1/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sessions = resp["sessions"].as_array().unwrap();
    assert!(sessions.iter().any(|s| s["id"] == session_id));

    let resp: Value = client
        .get(srv.http_url(&format!("/api/v1/sessions/{session_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["id"], session_id);

    let resp: Value = client
        .get(srv.http_url(&format!("/api/v1/sessions/{session_id}/messages")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msgs = resp["messages"].as_array().unwrap();
    assert!(
        msgs.len() >= 2,
        "should have user + assistant messages, got {}",
        msgs.len()
    );

    let resp: Value = client
        .delete(srv.http_url(&format!("/api/v1/sessions/{session_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["deleted"], true);
}

// ---------------------------------------------------------------------------
// WebSocket integration tests
// ---------------------------------------------------------------------------

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header, HeaderValue},
        Message,
    },
};

async fn ws_connect(
    url: &str,
) -> (
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let (ws, _) = connect_async(url).await.expect("ws connect");
    ws.split()
}

/// WebSocket handshake with `Authorization: Bearer` (upgrade is API-key protected).
async fn ws_connect_bearer(
    url: &str,
    bearer_token: &str,
) -> (
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let mut req = url.into_client_request().expect("ws request build");
    let auth = HeaderValue::from_str(&format!("Bearer {bearer_token}")).expect("auth header");
    req.headers_mut().insert(header::AUTHORIZATION, auth);
    let (ws, _) = connect_async(req).await.expect("ws connect");
    ws.split()
}

async fn recv_json(
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

async fn send_json(
    tx: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    val: Value,
) {
    tx.send(Message::Text(val.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn ws_connect_and_welcome() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut _tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let welcome = recv_json(&mut rx).await;
    assert_eq!(welcome["type"], "connected");
    assert!(welcome["data"]["methods"].is_array());
}

#[tokio::test]
async fn ws_ping_pong() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    send_json(&mut tx, json!({"id": "p1", "method": "ping"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "pong");
    assert_eq!(resp["id"], "p1");
}

#[tokio::test]
async fn ws_auth_token_query() {
    let srv = TestServer::start(auth_enabled(vec!["ws-key"])).await;

    // Connect with valid token
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("token=ws-key")).await;
    let welcome = recv_json(&mut rx).await;
    assert_eq!(welcome["type"], "connected");
    assert_eq!(
        welcome["data"]["authRequired"], false,
        "pre-authed via token"
    );

    // agents should work
    send_json(&mut tx, json!({"id": "a1", "method": "agents"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "agents");
}

#[tokio::test]
async fn ws_auth_required_without_token() {
    let srv = TestServer::start(auth_enabled(vec!["ws-key"])).await;

    // HTTP upgrade must include API key when auth is enabled (no query `token`).
    let (mut tx, mut rx) = ws_connect_bearer(&srv.ws_url(""), "ws-key").await;
    let welcome = recv_json(&mut rx).await;
    assert_eq!(welcome["data"]["authRequired"], true);

    // Non-auth methods should be rejected
    send_json(&mut tx, json!({"id": "a1", "method": "agents"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert_eq!(resp["error"]["code"], 401);

    // Ping still works
    send_json(&mut tx, json!({"id": "p1", "method": "ping"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "pong");

    // In-band auth with wrong key
    send_json(
        &mut tx,
        json!({"id": "auth1", "method": "auth", "params": {"token": "bad"}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "auth.failed");

    // In-band auth with correct key
    send_json(
        &mut tx,
        json!({"id": "auth2", "method": "auth", "params": {"token": "ws-key"}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "auth.ok");

    // Now agents should work
    send_json(&mut tx, json!({"id": "a2", "method": "agents"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "agents");
}

#[tokio::test]
async fn ws_unknown_method() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    send_json(&mut tx, json!({"id": "u1", "method": "nonexistent"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn ws_agents_and_models() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    send_json(&mut tx, json!({"id": "a1", "method": "agents"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "agents");
    let agents = resp["data"]["agents"].as_array().unwrap();
    assert!(!agents.is_empty());

    send_json(&mut tx, json!({"id": "m1", "method": "models.list"})).await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "models.list");
    assert!(resp["data"]["models"].is_array());
}

#[tokio::test]
async fn ws_chat_stream_and_session_persistence() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    send_json(
        &mut tx,
        json!({
            "id": "c1",
            "method": "chat",
            "params": {
                "messages": [{"role": "user", "content": "test message"}]
            }
        }),
    )
    .await;

    let mut got_start = false;
    let mut got_delta = false;
    let got_complete;
    let mut session_id = String::new();

    loop {
        let msg = recv_json(&mut rx).await;
        match msg["type"].as_str().unwrap() {
            "chat.start" => {
                got_start = true;
                session_id = msg["data"]["sessionId"].as_str().unwrap().to_string();
            }
            "chat.delta" => got_delta = true,
            "chat.complete" => {
                got_complete = true;
                break;
            }
            "chat.error" => panic!("unexpected chat error: {msg}"),
            other => panic!("unexpected event type: {other}"),
        }
    }
    assert!(got_start, "should receive chat.start");
    assert!(got_delta, "should receive at least one chat.delta");
    assert!(got_complete, "should receive chat.complete");
    assert!(!session_id.is_empty());

    // Verify session messages are persisted
    send_json(
        &mut tx,
        json!({
            "id": "sm1",
            "method": "sessions.messages",
            "params": {"sessionId": &session_id}
        }),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "sessions.messages");
    let messages = resp["data"]["messages"].as_array().unwrap();
    assert!(
        messages.len() >= 2,
        "expected user+assistant, got {}",
        messages.len()
    );
}

#[tokio::test]
async fn ws_session_crud() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    // Chat to create a session
    send_json(
        &mut tx,
        json!({
            "id": "c1", "method": "chat",
            "params": {"messages": [{"role": "user", "content": "hello"}]}
        }),
    )
    .await;
    let mut session_id = String::new();
    loop {
        let msg = recv_json(&mut rx).await;
        if msg["type"] == "chat.start" {
            session_id = msg["data"]["sessionId"].as_str().unwrap().to_string();
        }
        if msg["type"] == "chat.complete" {
            break;
        }
        if msg["type"] == "chat.error" {
            panic!("chat error: {msg}");
        }
    }

    // sessions.list
    send_json(
        &mut tx,
        json!({"id": "sl1", "method": "sessions.list", "params": {}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "sessions.list");
    let sessions = resp["data"]["sessions"].as_array().unwrap();
    assert!(sessions.iter().any(|s| s["id"] == session_id));

    // sessions.get
    send_json(
        &mut tx,
        json!({"id": "sg1", "method": "sessions.get", "params": {"sessionId": &session_id}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "sessions.get");
    assert_eq!(resp["data"]["id"], session_id);

    // sessions.delete
    send_json(
        &mut tx,
        json!({"id": "sd1", "method": "sessions.delete", "params": {"sessionId": &session_id}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "sessions.delete");
    assert_eq!(resp["data"]["deleted"], true);

    // sessions.get should now 404
    send_json(
        &mut tx,
        json!({"id": "sg2", "method": "sessions.get", "params": {"sessionId": &session_id}}),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert_eq!(resp["error"]["code"], 404);
}

#[tokio::test]
async fn ws_subscribe_unsubscribe() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    send_json(
        &mut tx,
        json!({
            "id": "sub1", "method": "subscribe",
            "params": {"events": ["sessions.changed"]}
        }),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "subscribe.ok");

    send_json(
        &mut tx,
        json!({
            "id": "unsub1", "method": "unsubscribe",
            "params": {"events": ["sessions.changed"]}
        }),
    )
    .await;
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "unsubscribe.ok");
    let subs = resp["data"]["subscriptions"].as_array().unwrap();
    assert!(subs.is_empty());
}

#[tokio::test]
async fn ws_parse_error() {
    let srv = TestServer::start(auth_disabled()).await;
    let (mut tx, mut rx) = ws_connect(&srv.ws_url("")).await;
    let _welcome = recv_json(&mut rx).await;

    tx.send(Message::Text("not valid json".into()))
        .await
        .unwrap();
    let resp = recv_json(&mut rx).await;
    assert_eq!(resp["type"], "error");
    assert_eq!(resp["error"]["code"], -32700);
}

// ---------------------------------------------------------------------------
// Channel webhook E2E tests (Feishu)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_webhook_unknown_channel_returns_404() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(srv.http_url("/webhook/nonexistent"))
        .json(&json!({"test": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    let msg = body["error"]["message"].as_str().unwrap();
    assert!(msg.contains("nonexistent"), "unexpected body: {body}");
}

#[tokio::test]
async fn channel_list_returns_empty_without_env() {
    let srv = TestServer::start(auth_disabled()).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .get(srv.http_url("/api/v1/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(resp["channels"].is_array());
    assert_eq!(resp["count"], 0, "no channels without FEISHU env vars");
}

// ---------------------------------------------------------------------------
// Feishu plugin unit-level E2E tests (ChannelPlugin trait)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_plugin_url_verification_challenge() {
    use fastclaw_core::channel::{ChannelPlugin, WebhookResult};

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "cli_test".into(),
        app_secret: "secret_test".into(),
        verification_token: None,
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let payload = json!({
        "challenge": "abc123-challenge-token",
        "token": "",
        "type": "url_verification"
    });

    match plugin.handle_webhook(payload).await.unwrap() {
        WebhookResult::Challenge(v) => {
            assert_eq!(v["challenge"], "abc123-challenge-token");
        }
        other => panic!("expected Challenge, got {:?}", other),
    }
}

#[tokio::test]
async fn feishu_plugin_parses_text_message() {
    use fastclaw_core::channel::{ChannelPlugin, WebhookResult};

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "cli_test".into(),
        app_secret: "secret_test".into(),
        verification_token: Some("test_token".into()),
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let payload = json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_001",
            "event_type": "im.message.receive_v1",
            "token": "test_token",
            "create_time": "1234567890"
        },
        "event": {
            "sender": {
                "sender_id": {
                    "open_id": "ou_user123",
                    "user_id": "uid_123",
                    "union_id": "on_123"
                },
                "sender_type": "user",
                "tenant_key": "tenant_abc"
            },
            "message": {
                "message_id": "om_msg001",
                "root_id": "",
                "parent_id": "",
                "create_time": "1234567890",
                "chat_id": "oc_group001",
                "chat_type": "group",
                "message_type": "text",
                "content": "{\"text\":\"Hello FastClaw!\"}"
            }
        }
    });

    match plugin.handle_webhook(payload).await.unwrap() {
        WebhookResult::Messages(msgs) => {
            assert_eq!(msgs.len(), 1);
            let msg = &msgs[0];
            assert_eq!(msg.channel_id, "feishu");
            assert_eq!(msg.sender_id, "ou_user123");
            assert_eq!(msg.chat_id, "oc_group001");
            assert_eq!(msg.message_id, "om_msg001");
            assert_eq!(msg.text, "Hello FastClaw!");
            assert_eq!(msg.msg_type, "text");
        }
        other => panic!("expected Messages, got {:?}", other),
    }
}

#[tokio::test]
async fn feishu_plugin_ignores_non_text_messages() {
    use fastclaw_core::channel::{ChannelPlugin, WebhookResult};

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "cli_test".into(),
        app_secret: "secret_test".into(),
        verification_token: None,
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let payload = json!({
        "header": {
            "event_type": "im.message.receive_v1",
            "token": ""
        },
        "event": {
            "sender": {"sender_id": {"open_id": "ou_user"}},
            "message": {
                "message_id": "om_img001",
                "chat_id": "oc_group",
                "message_type": "image",
                "content": "{\"image_key\":\"img_v3_xxx\"}"
            }
        }
    });

    match plugin.handle_webhook(payload).await.unwrap() {
        WebhookResult::Ignored => {}
        other => panic!("expected Ignored for image message, got {:?}", other),
    }
}

#[tokio::test]
async fn feishu_plugin_rejects_bad_token() {
    use fastclaw_core::channel::ChannelPlugin;
    use std::collections::BTreeMap;

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "cli_test".into(),
        app_secret: "secret_test".into(),
        verification_token: Some("correct_token".into()),
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let payload = json!({
        "header": {
            "event_type": "im.message.receive_v1",
            "token": "wrong_token"
        },
        "event": {
            "sender": {"sender_id": {"open_id": "ou_user"}},
            "message": {
                "message_id": "om_001",
                "chat_id": "oc_group",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    });

    let raw_body = serde_json::to_vec(&payload).unwrap();
    let headers = BTreeMap::new();
    let result = plugin.verify_webhook(&headers, &raw_body).await;
    assert!(result.is_err(), "wrong token should be rejected by verify_webhook");
}

#[tokio::test]
async fn feishu_plugin_ignores_unknown_event_types() {
    use fastclaw_core::channel::{ChannelPlugin, WebhookResult};

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "cli_test".into(),
        app_secret: "secret_test".into(),
        verification_token: None,
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let payload = json!({
        "header": {
            "event_type": "im.chat.member.bot.added_v1",
            "token": ""
        },
        "event": {
            "chat_id": "oc_group"
        }
    });

    match plugin.handle_webhook(payload).await.unwrap() {
        WebhookResult::Ignored => {}
        other => panic!("expected Ignored for non-message event, got {:?}", other),
    }
}

#[tokio::test]
async fn feishu_plugin_capabilities_and_meta() {
    use fastclaw_core::channel::ChannelPlugin;

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "test".into(),
        app_secret: "test".into(),
        verification_token: None,
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let meta = plugin.meta();
    assert_eq!(meta.id, "feishu");
    assert_eq!(meta.name, "Feishu");
    assert!(meta.aliases.contains(&"lark".to_string()));

    let caps = plugin.capabilities();
    assert!(caps.direct_message);
    assert!(caps.group_chat);
    assert!(caps.media);
    assert!(caps.threads);
    assert!(caps.streaming);
}

#[tokio::test]
async fn feishu_plugin_provides_tools() {
    use fastclaw_core::channel::ChannelPlugin;

    let plugin = fastclaw_feishu::FeishuPlugin::new(fastclaw_feishu::FeishuPluginConfig {
        app_id: "test".into(),
        app_secret: "test".into(),
        verification_token: None,
        encrypt_key: None,
        agent_id: "main".into(),
        connection_mode: "webhook".into(),
        domain: "https://open.feishu.cn".into(),
        reply_mode: "mention_only".into(),
        user_access_token: None,
    });

    let tools = plugin.tools();
    assert_eq!(tools.len(), 9, "IM + user-scoped Feishu tools");

    let names: Vec<&str> = tools
        .iter()
        .map(|t| fastclaw_core::tool::Tool::name(t.as_ref()))
        .collect();
    assert!(names.contains(&"feishu_send_message"));
    assert!(names.contains(&"feishu_reply_message"));
    assert!(names.contains(&"feishu_get_chat_messages"));
    assert!(names.contains(&"feishu_task_create"));
    assert!(names.contains(&"feishu_doc_get_content"));
}

#[tokio::test]
async fn feishu_tool_validates_empty_args() {
    use fastclaw_core::tool::Tool;

    let client = std::sync::Arc::new(fastclaw_feishu::FeishuClient::new("t", "s"));
    let send_tool = fastclaw_feishu::FeishuSendMessageTool::new(client.clone());
    let reply_tool = fastclaw_feishu::FeishuReplyMessageTool::new(client.clone());
    let get_tool = fastclaw_feishu::FeishuGetChatMessagesTool::new(client);

    let r = send_tool
        .execute(r#"{"receive_id":"","receive_id_type":"chat_id","text":""}"#)
        .await;
    assert!(!r.success, "empty receive_id and text should fail");

    let r = reply_tool.execute(r#"{"message_id":"","text":""}"#).await;
    assert!(!r.success, "empty message_id and text should fail");

    let r = get_tool.execute(r#"{"chat_id":""}"#).await;
    assert!(!r.success, "empty chat_id should fail");
}

#[tokio::test]
async fn feishu_tool_rejects_invalid_json() {
    use fastclaw_core::tool::Tool;

    let client = std::sync::Arc::new(fastclaw_feishu::FeishuClient::new("t", "s"));
    let send_tool = fastclaw_feishu::FeishuSendMessageTool::new(client);

    let r = send_tool.execute("not json at all").await;
    assert!(!r.success, "invalid json should fail");
    assert!(r.output.contains("invalid arguments"));
}

// ---------------------------------------------------------------------------
// Feishu messaging module tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_messaging_inbound_parse() {
    use fastclaw_feishu::messaging::inbound::parse_message_event;

    let event = json!({
        "sender": {
            "sender_id": {"open_id": "ou_sender"}
        },
        "message": {
            "message_id": "om_abc",
            "chat_id": "oc_chat1",
            "chat_type": "group",
            "message_type": "text",
            "content": "{\"text\":\"parsed correctly\"}"
        }
    });

    let ctx = parse_message_event(&event).unwrap();
    assert_eq!(ctx.text, "parsed correctly");
    assert_eq!(ctx.chat_id, "oc_chat1");
    assert_eq!(ctx.sender_open_id, "ou_sender");
    assert_eq!(ctx.message_type, "text");
}

#[tokio::test]
async fn feishu_messaging_dedup() {
    use fastclaw_feishu::messaging::inbound::MessageDedup;
    use std::time::Duration;

    let mut dedup = MessageDedup::new(Duration::from_secs(5));
    assert!(dedup.check("om_1"), "first occurrence should be new");
    assert!(
        !dedup.check("om_1"),
        "second occurrence should be duplicate"
    );
    assert!(dedup.check("om_2"), "different message should be new");
    assert_eq!(dedup.size(), 2);

    dedup.dispose();
    assert_eq!(dedup.size(), 0);
    assert!(dedup.check("om_1"), "after dispose, should accept again");
}

#[tokio::test]
async fn feishu_messaging_mention_extraction() {
    use fastclaw_feishu::messaging::inbound::{extract_message_body, mentioned_bot};
    use fastclaw_feishu::messaging::MentionRef;

    let bot_id = "ou_bot";
    let mentions = vec![
        MentionRef {
            key: "@_user_1".into(),
            open_id: bot_id.into(),
            name: "TestBot".into(),
        },
        MentionRef {
            key: "@_user_2".into(),
            open_id: "ou_human".into(),
            name: "Alice".into(),
        },
    ];

    assert!(mentioned_bot(&mentions, bot_id));
    assert!(!mentioned_bot(&mentions, "ou_someone_else"));

    let body = extract_message_body("@_user_1 请帮我查一下日程", &mentions, bot_id);
    assert_eq!(body, "请帮我查一下日程");
    assert!(!body.contains("@_user_1"));
}

// ---------------------------------------------------------------------------
// Feishu event handler tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_event_handler_parsing() {
    use fastclaw_feishu::channel::{parse_event, FeishuEventType};

    assert_eq!(
        parse_event(&json!({"challenge": "abc"})),
        FeishuEventType::UrlVerification
    );
    assert_eq!(
        parse_event(&json!({"header": {"event_type": "im.message.receive_v1"}})),
        FeishuEventType::ImMessageReceive
    );
    assert_eq!(
        parse_event(&json!({"header": {"event_type": "im.message.reaction.created_v1"}})),
        FeishuEventType::ImMessageReactionCreated
    );
    assert_eq!(
        parse_event(&json!({"header": {"event_type": "card.action.trigger"}})),
        FeishuEventType::InteractiveCard
    );

    match parse_event(&json!({"header": {"event_type": "custom.event"}})) {
        FeishuEventType::Unknown(s) => assert_eq!(s, "custom.event"),
        other => panic!("expected Unknown, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Feishu core types tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_core_types_brand() {
    use fastclaw_feishu::core::types::{FeishuProbeResult, LarkBrand};

    assert_eq!(LarkBrand::default(), LarkBrand::Feishu);
    assert!(LarkBrand::Feishu.base_url().contains("feishu.cn"));
    assert!(LarkBrand::Lark.base_url().contains("larksuite.com"));

    let r = FeishuProbeResult {
        ok: true,
        app_id: Some("cli_abc".into()),
        bot_name: Some("FastBot".into()),
        bot_open_id: None,
        error: None,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    assert!(json_str.contains("cli_abc"));
    assert!(json_str.contains("FastBot"));
}

// ---------------------------------------------------------------------------
// Feishu config schema tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_config_schema_valid() {
    use fastclaw_feishu::core::config_schema::feishu_config_json_schema;

    let schema = feishu_config_json_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "app_id"));
    assert!(required.iter().any(|v| v == "app_secret"));
    assert!(schema["properties"]["brand"].is_object());
    assert!(schema["properties"]["connection_mode"].is_object());
}

// ---------------------------------------------------------------------------
// Feishu commands (diagnose) tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn feishu_diagnose_format() {
    use fastclaw_feishu::commands::diagnose::{
        format_report_cli, DiagCheck, DiagStatus, DiagnosisReport,
    };

    let report = DiagnosisReport {
        overall_status: DiagStatus::Degraded,
        checks: vec![
            DiagCheck {
                name: "token".into(),
                status: DiagStatus::Healthy,
                message: "ok".into(),
            },
            DiagCheck {
                name: "webhook".into(),
                status: DiagStatus::Degraded,
                message: "timeout".into(),
            },
        ],
    };

    let out = format_report_cli(&report);
    assert!(out.contains("Degraded"));
    assert!(out.contains("✓ token"));
    assert!(out.contains("△ webhook"));
}
