

use base64::Engine;
use sha2::Digest;

/// OAuth 2.0 Authorization Server metadata (RFC 8414 subset).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OAuthMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Token response from the token endpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Persisted token data (stored to disk for reuse across restarts).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: Option<u64>,
    pub server_url: String,
}

/// PKCE pair: verifier (secret) + challenge (sent to auth server).
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE S256 challenge pair.
    pub fn generate() -> Self {
        use rand::Rng;
        let mut rng = rand::rng();
        let verifier_bytes: Vec<u8> = (0..64).map(|_| rng.random::<u8>()).collect();
        let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&verifier_bytes);

        let mut hasher = sha2::Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        Self {
            code_verifier,
            code_challenge,
        }
    }
}

/// Full OAuth client for a single MCP server.
pub struct McpOAuthClient {
    http: reqwest::Client,
    server_url: String,
    metadata: Option<OAuthMetadata>,
}

impl McpOAuthClient {
    pub fn new(server_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            server_url: server_url.trim_end_matches('/').to_string(),
            metadata: None,
        }
    }

    /// Discover OAuth metadata from `/.well-known/oauth-authorization-server`.
    pub async fn discover_metadata(&mut self) -> anyhow::Result<&OAuthMetadata> {
        let url = reqwest::Url::parse(&self.server_url)?;
        let well_known = format!(
            "{}://{}{}/.well-known/oauth-authorization-server",
            url.scheme(),
            url.authority(),
            url.path().trim_end_matches('/')
        );
        tracing::debug!(url = %well_known, "discovering OAuth metadata");

        let resp = self.http.get(&well_known).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "OAuth metadata discovery failed: HTTP {} from {}",
                resp.status(),
                well_known
            );
        }
        let meta: OAuthMetadata = resp.json().await?;
        tracing::info!(
            auth_endpoint = %meta.authorization_endpoint,
            token_endpoint = %meta.token_endpoint,
            "discovered OAuth metadata"
        );
        self.metadata = Some(meta);
        Ok(self.metadata.as_ref().unwrap())
    }

    /// Build the authorization URL with PKCE parameters.
    pub fn build_authorization_url(
        &self,
        pkce: &PkceChallenge,
        redirect_uri: &str,
        state: &str,
        client_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let meta = self
            .metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OAuth metadata not discovered yet"))?;

        let effective_client_id =
            client_id.map_or_else(|| self.server_url.clone(), |s| s.to_string());

        let params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", &effective_client_id),
            ("code_challenge_method", "S256"),
            ("code_challenge", &pkce.code_challenge),
            ("redirect_uri", redirect_uri),
            ("state", state),
        ];

        let auth_url = reqwest::Url::parse_with_params(&meta.authorization_endpoint, &params)
            .map_err(|e| anyhow::anyhow!("failed to build auth URL: {e}"))?;

        Ok(auth_url.to_string())
    }

    /// Exchange an authorization code for tokens.
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<TokenResponse> {
        let meta = self
            .metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OAuth metadata not discovered yet"))?;

        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
        ];

        let resp = self
            .http
            .post(&meta.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("token exchange failed: HTTP {status}: {body}");
        }

        let token: TokenResponse = resp.json().await?;
        tracing::info!("OAuth token exchange successful");
        Ok(token)
    }

    /// Refresh an access token using a refresh token.
    pub async fn refresh_token(&self, refresh_token: &str) -> anyhow::Result<TokenResponse> {
        let meta = self
            .metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OAuth metadata not discovered yet"))?;

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];

        let resp = self
            .http
            .post(&meta.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("token refresh failed: HTTP {status}: {body}");
        }

        let token: TokenResponse = resp.json().await?;
        tracing::info!("OAuth token refresh successful");
        Ok(token)
    }

    /// Perform RFC 7591 Dynamic Client Registration.
    ///
    /// Only called when:
    /// 1. No explicit `client_id` is configured for this server
    /// 2. The server's OAuth metadata includes a `registration_endpoint`
    ///
    /// Returns the registered client credentials, which should be persisted
    /// for subsequent OAuth flows.
    pub async fn register_client(
        &self,
        redirect_uris: &[String],
    ) -> anyhow::Result<ClientRegistration> {
        let meta = self
            .metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OAuth metadata not discovered yet"))?;
        let endpoint = meta
            .registration_endpoint
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("server does not support dynamic client registration"))?;

        let body = serde_json::json!({
            "client_name": "XiaoLin",
            "redirect_uris": redirect_uris,
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });

        tracing::info!(endpoint = %endpoint, "performing dynamic client registration");

        let resp = self.http.post(endpoint).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("DCR failed: HTTP {status}: {err_body}");
        }

        let reg: ClientRegistration = resp.json().await?;
        tracing::info!(
            client_id = %reg.client_id,
            "dynamic client registration successful"
        );
        Ok(reg)
    }

    /// Check if the server supports dynamic client registration.
    pub fn supports_dcr(&self) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.registration_endpoint.as_ref())
            .is_some()
    }

    pub fn metadata(&self) -> Option<&OAuthMetadata> {
        self.metadata.as_ref()
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }
}

/// Result of RFC 7591 Dynamic Client Registration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClientRegistration {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_id_issued_at: Option<u64>,
    #[serde(default)]
    pub client_secret_expires_at: Option<u64>,
}

/// Start a local HTTP server on a random port to receive the OAuth callback.
///
/// Returns `(redirect_uri, code_receiver)` where:
/// - `redirect_uri` is `http://127.0.0.1:<port>/callback`
/// - `code_receiver` is a oneshot receiver that yields `(code, state)` when the callback arrives
pub async fn start_callback_server() -> anyhow::Result<(
    String,
    tokio::sync::oneshot::Receiver<(String, String)>,
)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let (tx, rx) = tokio::sync::oneshot::channel::<(String, String)>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let done_clone = done.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            let done = done_clone.clone();
            tokio::spawn(async move {
                handle_callback_connection(stream, tx, &done).await;
            });
            if done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
        }
        tracing::debug!(port, "OAuth callback server stopped");
    });

    tracing::info!(port, "OAuth callback server started");
    Ok((redirect_uri, rx))
}

type CallbackSender =
    std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<(String, String)>>>>;

async fn handle_callback_connection(
    mut stream: tokio::net::TcpStream,
    tx: CallbackSender,
    done: &std::sync::atomic::AtomicBool,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 4096];
    let Ok(n) = stream.read(&mut buf).await else {
        return;
    };
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("");

    let fake_base = format!("http://localhost{path}");
    let parsed = reqwest::Url::parse(&fake_base).unwrap_or_else(|_| {
        reqwest::Url::parse("http://localhost/?code=&state=").unwrap()
    });
    let mut code = String::new();
    let mut state = String::new();
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = v.into_owned(),
            "state" => state = v.into_owned(),
            _ => {}
        }
    }

    let body = if code.is_empty() {
        "<html><body><h2>Authorization failed</h2><p>No code received. You can close this window.</p></body></html>"
    } else {
        "<html><body><h2>Authorization successful!</h2><p>You can close this window and return to XiaoLin.</p></body></html>"
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;

    if !code.is_empty() {
        if let Some(sender) = tx.lock().await.take() {
            let _ = sender.send((code, state));
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

// ─── Token Storage ─────────────────────────────────────────────────

/// Abstraction for token persistence, allowing swappable backends.
#[async_trait::async_trait]
pub trait TokenStore: Send + Sync {
    async fn load(&self, server_id: &str) -> Option<StoredToken>;
    async fn save(&self, server_id: &str, token: &StoredToken) -> anyhow::Result<()>;
    async fn delete(&self, server_id: &str) -> anyhow::Result<()>;
}

/// File-based token storage (plaintext JSON, legacy default).
pub struct FileTokenStore;

#[async_trait::async_trait]
impl TokenStore for FileTokenStore {
    async fn load(&self, server_id: &str) -> Option<StoredToken> {
        load_stored_token(server_id)
    }

    async fn save(&self, server_id: &str, token: &StoredToken) -> anyhow::Result<()> {
        save_stored_token(server_id, token)
    }

    async fn delete(&self, server_id: &str) -> anyhow::Result<()> {
        remove_stored_token(server_id);
        Ok(())
    }
}

/// Load a stored token for the given MCP server ID.
pub fn load_stored_token(server_id: &str) -> Option<StoredToken> {
    let path = token_file_path(server_id);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save a token for the given MCP server ID.
pub fn save_stored_token(server_id: &str, token: &StoredToken) -> anyhow::Result<()> {
    let path = token_file_path(server_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(token)?;
    std::fs::write(&path, content)?;
    tracing::debug!(server_id, path = %path.display(), "saved OAuth token");
    Ok(())
}

/// Remove stored token for the given MCP server ID.
pub fn remove_stored_token(server_id: &str) {
    let path = token_file_path(server_id);
    let _ = std::fs::remove_file(&path);
}

fn token_file_path(server_id: &str) -> std::path::PathBuf {
    let sanitized = server_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.xiaolin.desktop")
        .join("mcp-tokens")
        .join(format!("{sanitized}.json"))
}

/// List all server IDs that have stored tokens (for migration).
pub fn list_stored_token_ids() -> Vec<String> {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.xiaolin.desktop")
        .join("mcp-tokens");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_format() {
        let pkce = PkceChallenge::generate();
        assert!(pkce.code_verifier.len() >= 43);
        assert!(pkce.code_verifier.len() <= 128);
        assert!(!pkce.code_challenge.is_empty());
        assert_ne!(pkce.code_verifier, pkce.code_challenge);
    }

    #[test]
    fn pkce_challenge_s256_correct() {
        let pkce = PkceChallenge::generate();
        let mut hasher = sha2::Sha256::new();
        hasher.update(pkce.code_verifier.as_bytes());
        let hash = hasher.finalize();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
        assert_eq!(pkce.code_challenge, expected);
    }

    #[test]
    fn pkce_challenge_uniqueness() {
        let a = PkceChallenge::generate();
        let b = PkceChallenge::generate();
        assert_ne!(a.code_verifier, b.code_verifier);
    }

    #[test]
    fn oauth_metadata_deserialize() {
        let json = r#"{
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "code_challenge_methods_supported": ["S256"],
            "response_types_supported": ["code"]
        }"#;
        let meta: OAuthMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.authorization_endpoint, "https://auth.example.com/authorize");
        assert_eq!(meta.token_endpoint, "https://auth.example.com/token");
        assert!(meta.code_challenge_methods_supported.contains(&"S256".to_string()));
    }

    #[test]
    fn stored_token_roundtrip() {
        let token = StoredToken {
            access_token: "abc".into(),
            refresh_token: Some("xyz".into()),
            expires_at: Some(1700000000),
            server_url: "https://mcp.example.com".into(),
        };
        let json = serde_json::to_string(&token).unwrap();
        let back: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access_token, "abc");
        assert_eq!(back.refresh_token.as_deref(), Some("xyz"));
    }

    #[tokio::test]
    async fn callback_server_starts_and_receives_code() {
        let (redirect_uri, rx) = start_callback_server().await.unwrap();
        assert!(redirect_uri.starts_with("http://127.0.0.1:"));

        let callback_url = format!("{redirect_uri}?code=test_code_123&state=test_state_456");
        let resp = reqwest::get(&callback_url).await.unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert!(body.contains("successful"));

        let (code, state) = rx.await.unwrap();
        assert_eq!(code, "test_code_123");
        assert_eq!(state, "test_state_456");
    }

    #[test]
    fn client_registration_deserialize() {
        let json = r#"{
            "client_id": "abc123",
            "client_secret": "secret456",
            "client_id_issued_at": 1700000000
        }"#;
        let reg: ClientRegistration = serde_json::from_str(json).unwrap();
        assert_eq!(reg.client_id, "abc123");
        assert_eq!(reg.client_secret.as_deref(), Some("secret456"));
        assert_eq!(reg.client_id_issued_at, Some(1700000000));
        assert_eq!(reg.client_secret_expires_at, None);
    }

    #[test]
    fn supports_dcr_checks_registration_endpoint() {
        let mut client = McpOAuthClient::new("https://mcp.example.com");
        assert!(!client.supports_dcr());

        client.metadata = Some(OAuthMetadata {
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            registration_endpoint: Some("https://auth.example.com/register".into()),
            code_challenge_methods_supported: vec![],
            response_types_supported: vec![],
            grant_types_supported: vec![],
        });
        assert!(client.supports_dcr());
    }

    #[tokio::test]
    async fn register_client_with_mock_dcr_endpoint() {
        use axum::body::Bytes;
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::Router;

        async fn handle_register(body: Bytes) -> (StatusCode, String) {
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["client_name"], "XiaoLin");
            assert!(v["redirect_uris"].is_array());
            assert_eq!(v["grant_types"][0], "authorization_code");
            assert_eq!(v["response_types"][0], "code");

            let resp = serde_json::json!({
                "client_id": "dcr-client-id-001",
                "client_secret": "dcr-secret-xyz",
                "client_id_issued_at": 1700000000,
                "redirect_uris": v["redirect_uris"]
            });
            (StatusCode::CREATED, serde_json::to_string(&resp).unwrap())
        }

        let app = Router::new().route("/register", post(handle_register));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let reg_endpoint = format!("http://127.0.0.1:{}/register", addr.port());
        let mut client = McpOAuthClient::new("https://mcp.example.com");
        client.metadata = Some(OAuthMetadata {
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            registration_endpoint: Some(reg_endpoint),
            code_challenge_methods_supported: vec!["S256".into()],
            response_types_supported: vec!["code".into()],
            grant_types_supported: vec!["authorization_code".into()],
        });

        assert!(client.supports_dcr());
        let reg = client
            .register_client(&["http://127.0.0.1:9999/callback".into()])
            .await
            .expect("DCR should succeed");
        assert_eq!(reg.client_id, "dcr-client-id-001");
        assert_eq!(reg.client_secret.as_deref(), Some("dcr-secret-xyz"));
        assert_eq!(reg.client_id_issued_at, Some(1700000000));
    }

    #[tokio::test]
    async fn register_client_dcr_failure_returns_error() {
        use axum::body::Bytes;
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::Router;

        async fn handle_register(_body: Bytes) -> (StatusCode, String) {
            (StatusCode::BAD_REQUEST, r#"{"error":"invalid_client_metadata"}"#.into())
        }

        let app = Router::new().route("/register", post(handle_register));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let reg_endpoint = format!("http://127.0.0.1:{}/register", addr.port());
        let mut client = McpOAuthClient::new("https://mcp.example.com");
        client.metadata = Some(OAuthMetadata {
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            registration_endpoint: Some(reg_endpoint),
            code_challenge_methods_supported: vec![],
            response_types_supported: vec![],
            grant_types_supported: vec![],
        });

        let result = client
            .register_client(&["http://127.0.0.1:9999/callback".into()])
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("DCR failed") || err.contains("400"), "unexpected error: {err}");
    }

    #[test]
    fn file_token_store_save_load_delete() {
        let unique_id = format!("__test_token_store_{}", std::process::id());

        let loaded = load_stored_token(&unique_id);
        assert!(loaded.is_none(), "should be empty initially");

        let token = StoredToken {
            access_token: "tok-abc".into(),
            refresh_token: Some("ref-xyz".into()),
            expires_at: Some(9999999999),
            server_url: "https://example.com".into(),
        };
        save_stored_token(&unique_id, &token).unwrap();

        let loaded = load_stored_token(&unique_id);
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.access_token, "tok-abc");
        assert_eq!(loaded.refresh_token.as_deref(), Some("ref-xyz"));

        remove_stored_token(&unique_id);
        let loaded = load_stored_token(&unique_id);
        assert!(loaded.is_none(), "should be empty after delete");
    }

    #[test]
    fn build_authorization_url_works() {
        let mut client = McpOAuthClient::new("https://mcp.example.com");
        client.metadata = Some(OAuthMetadata {
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            registration_endpoint: None,
            code_challenge_methods_supported: vec!["S256".into()],
            response_types_supported: vec!["code".into()],
            grant_types_supported: vec![],
        });

        let pkce = PkceChallenge::generate();
        let url = client
            .build_authorization_url(&pkce, "http://127.0.0.1:9999/callback", "test_state", None)
            .unwrap();
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("client_id="));

        let url_explicit = client
            .build_authorization_url(&pkce, "http://127.0.0.1:9999/callback", "test_state", Some("my-app"))
            .unwrap();
        assert!(url_explicit.contains("client_id=my-app"));
    }
}
