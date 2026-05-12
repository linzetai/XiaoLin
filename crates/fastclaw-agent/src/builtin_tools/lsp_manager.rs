use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, RwLock};
use url::Url;

#[derive(Clone)]
pub struct LspSessionManager {
    availability: Arc<RwLock<HashMap<String, bool>>>,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<PersistentLspSession>>>>>,
    rust_analyzer_cmd: Arc<RwLock<Option<String>>>,
    sessions_created: Arc<AtomicU64>,
    sessions_reused: Arc<AtomicU64>,
    requests_total: Arc<AtomicU64>,
    requests_failed: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct LspLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

impl LspSessionManager {
    pub fn global() -> &'static LspSessionManager {
        static INSTANCE: OnceLock<LspSessionManager> = OnceLock::new();
        INSTANCE.get_or_init(|| LspSessionManager {
            availability: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            rust_analyzer_cmd: Arc::new(RwLock::new(None)),
            sessions_created: Arc::new(AtomicU64::new(0)),
            sessions_reused: Arc::new(AtomicU64::new(0)),
            requests_total: Arc::new(AtomicU64::new(0)),
            requests_failed: Arc::new(AtomicU64::new(0)),
        })
    }

    pub async fn workspace_symbols(
        &self,
        sample_path: &str,
        query: &str,
        workspace_root: &str,
    ) -> anyhow::Result<Option<Vec<LspLocation>>> {
        if !self.supports_path(sample_path).await {
            return Ok(None);
        }
        let session_start = Instant::now();
        let (session, reused) = match self.get_or_create_session(workspace_root).await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut guard = session.lock().await;
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        let result = guard
            .request(
                "workspace/symbol",
                serde_json::json!({
                    "query": query,
                }),
            )
            .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                self.requests_failed.fetch_add(1, Ordering::Relaxed);
                drop(guard);
                self.invalidate_session(workspace_root).await;
                return Err(e);
            }
        };

        let items = result.as_array().cloned().unwrap_or_default();
        let out = items
            .iter()
            .filter_map(|item| item.get("location"))
            .filter_map(parse_lsp_location)
            .collect::<Vec<_>>();
        tracing::debug!(
            target: "fastclaw.code_intel.lsp",
            op = "workspace_symbols",
            reused_session = reused,
            elapsed_ms = session_start.elapsed().as_millis() as u64,
            result_count = out.len(),
            "lsp request completed"
        );
        Ok(Some(out))
    }

    pub async fn go_to_definition(
        &self,
        path: &str,
        line: usize,
        column: usize,
        file_text: &str,
        workspace_root: &str,
    ) -> anyhow::Result<Option<Vec<LspLocation>>> {
        if !self.supports_path(path).await {
            return Ok(None);
        }
        let session_start = Instant::now();
        let (session, reused) = match self.get_or_create_session(workspace_root).await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut guard = session.lock().await;
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = guard.did_open(path, "rust", file_text).await {
            self.requests_failed.fetch_add(1, Ordering::Relaxed);
            drop(guard);
            self.invalidate_session(workspace_root).await;
            return Err(e);
        }
        let result = guard
            .request(
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": { "uri": path_to_uri(path)? },
                    "position": { "line": line.saturating_sub(1), "character": column.saturating_sub(1) }
                }),
            )
            .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                self.requests_failed.fetch_add(1, Ordering::Relaxed);
                drop(guard);
                self.invalidate_session(workspace_root).await;
                return Err(e);
            }
        };
        let out = parse_lsp_locations(&result);
        tracing::debug!(
            target: "fastclaw.code_intel.lsp",
            op = "go_to_definition",
            reused_session = reused,
            elapsed_ms = session_start.elapsed().as_millis() as u64,
            result_count = out.len(),
            "lsp request completed"
        );
        Ok(Some(out))
    }

    pub async fn find_references(
        &self,
        path: &str,
        line: usize,
        column: usize,
        file_text: &str,
        include_declaration: bool,
        workspace_root: &str,
    ) -> anyhow::Result<Option<Vec<LspLocation>>> {
        if !self.supports_path(path).await {
            return Ok(None);
        }
        let session_start = Instant::now();
        let (session, reused) = match self.get_or_create_session(workspace_root).await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut guard = session.lock().await;
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = guard.did_open(path, "rust", file_text).await {
            self.requests_failed.fetch_add(1, Ordering::Relaxed);
            drop(guard);
            self.invalidate_session(workspace_root).await;
            return Err(e);
        }
        let result = guard
            .request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": { "uri": path_to_uri(path)? },
                    "position": { "line": line.saturating_sub(1), "character": column.saturating_sub(1) },
                    "context": { "includeDeclaration": include_declaration }
                }),
            )
            .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                self.requests_failed.fetch_add(1, Ordering::Relaxed);
                drop(guard);
                self.invalidate_session(workspace_root).await;
                return Err(e);
            }
        };
        let out = parse_lsp_locations(&result);
        tracing::debug!(
            target: "fastclaw.code_intel.lsp",
            op = "find_references",
            reused_session = reused,
            elapsed_ms = session_start.elapsed().as_millis() as u64,
            result_count = out.len(),
            "lsp request completed"
        );
        Ok(Some(out))
    }

    pub async fn hover(
        &self,
        path: &str,
        line: usize,
        column: usize,
        workspace_root: &str,
    ) -> anyhow::Result<Option<String>> {
        if !self.supports_path(path).await {
            return Ok(None);
        }
        let (session, _reused) = match self.get_or_create_session(workspace_root).await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut guard = session.lock().await;
        self.requests_total.fetch_add(1, Ordering::Relaxed);

        let result = guard
            .request(
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": { "uri": path_to_uri(path)? },
                    "position": { "line": line.saturating_sub(1), "character": column.saturating_sub(1) }
                }),
            )
            .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                self.requests_failed.fetch_add(1, Ordering::Relaxed);
                drop(guard);
                self.invalidate_session(workspace_root).await;
                return Err(e);
            }
        };
        if result.is_null() {
            return Ok(None);
        }
        let content = result.get("contents").and_then(|c| {
            if let Some(s) = c.as_str() {
                Some(s.to_string())
            } else if let Some(obj) = c.as_object() {
                obj.get("value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                Some(c.to_string())
            }
        });
        Ok(content)
    }

    async fn supports_path(&self, path: &str) -> bool {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        if ext != "rs" {
            return false;
        }
        let key = "rust-analyzer".to_string();
        {
            let g = self.availability.read().await;
            if let Some(v) = g.get(&key) {
                return *v;
            }
        }
        let discovered = self.discover_rust_analyzer_command().await;
        let available = discovered.is_some();
        if let Some(cmd) = discovered {
            let mut cmd_lock = self.rust_analyzer_cmd.write().await;
            *cmd_lock = Some(cmd);
        }
        let mut g = self.availability.write().await;
        g.insert(key, available);
        available
    }

    async fn get_or_create_session(
        &self,
        workspace_root: &str,
    ) -> anyhow::Result<(Arc<Mutex<PersistentLspSession>>, bool)> {
        let cmd = self
            .get_rust_analyzer_command()
            .await
            .ok_or_else(|| anyhow::anyhow!("rust-analyzer command unavailable"))?;
        {
            let g = self.sessions.read().await;
            if let Some(s) = g.get(workspace_root) {
                self.sessions_reused.fetch_add(1, Ordering::Relaxed);
                return Ok((s.clone(), true));
            }
        }

        let mut w = self.sessions.write().await;
        if let Some(s) = w.get(workspace_root) {
            self.sessions_reused.fetch_add(1, Ordering::Relaxed);
            return Ok((s.clone(), true));
        }
        let session = PersistentLspSession::spawn(&cmd, workspace_root).await?;
        let arc = Arc::new(Mutex::new(session));
        w.insert(workspace_root.to_string(), arc.clone());
        self.sessions_created.fetch_add(1, Ordering::Relaxed);
        Ok((arc, false))
    }

    async fn invalidate_session(&self, workspace_root: &str) {
        let mut w = self.sessions.write().await;
        w.remove(workspace_root);
    }

    pub fn stats_snapshot(&self) -> serde_json::Value {
        let cmd = self
            .rust_analyzer_cmd
            .try_read()
            .ok()
            .and_then(|g| g.clone());
        serde_json::json!({
            "sessionsCreated": self.sessions_created.load(Ordering::Relaxed),
            "sessionsReused": self.sessions_reused.load(Ordering::Relaxed),
            "requestsTotal": self.requests_total.load(Ordering::Relaxed),
            "requestsFailed": self.requests_failed.load(Ordering::Relaxed),
            "rustAnalyzerCommand": cmd,
        })
    }

    async fn get_rust_analyzer_command(&self) -> Option<String> {
        {
            let cmd = self.rust_analyzer_cmd.read().await;
            if let Some(v) = cmd.clone() {
                return Some(v);
            }
        }
        let discovered = self.discover_rust_analyzer_command().await?;
        let mut cmd = self.rust_analyzer_cmd.write().await;
        *cmd = Some(discovered.clone());
        Some(discovered)
    }

    async fn discover_rust_analyzer_command(&self) -> Option<String> {
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(env_bin) = std::env::var("FASTCLAW_RUST_ANALYZER_BIN") {
            if !env_bin.trim().is_empty() {
                candidates.push(env_bin);
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let file = bundled_rust_analyzer_file_name();
                candidates.push(exe_dir.join("lsp").join(file).to_string_lossy().to_string());
                candidates.push(
                    exe_dir
                        .join("resources")
                        .join("lsp")
                        .join(file)
                        .to_string_lossy()
                        .to_string(),
                );
                candidates.push(
                    exe_dir
                        .join("../Resources/lsp")
                        .join(file)
                        .to_string_lossy()
                        .to_string(),
                );
                candidates.push(
                    exe_dir
                        .join("../../Resources/lsp")
                        .join(file)
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
        if let Ok(resource_dir) = std::env::var("FASTCLAW_RESOURCE_DIR") {
            let file = bundled_rust_analyzer_file_name();
            candidates.push(
                std::path::Path::new(&resource_dir)
                    .join("lsp")
                    .join(file)
                    .to_string_lossy()
                    .to_string(),
            );
        }
        candidates.push("rust-analyzer".to_string());

        for c in candidates {
            if ensure_executable_if_local_path(&c).is_ok() && validate_command_available(&c).await {
                return Some(c);
            }
        }
        None
    }
}

fn bundled_rust_analyzer_file_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "rust-analyzer.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "rust-analyzer"
    }
}

fn ensure_executable_if_local_path(command: &str) -> anyhow::Result<()> {
    let path = Path::new(command);
    if !path.is_absolute() && path.components().count() <= 1 {
        return Ok(());
    }
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let md = std::fs::metadata(path)?;
        let mut perms = md.permissions();
        let mode = perms.mode();
        if mode & 0o111 == 0 {
            perms.set_mode(mode | 0o755);
            std::fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

async fn validate_command_available(command: &str) -> bool {
    let mut cmd = Command::new(command);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn parse_lsp_location(v: &serde_json::Value) -> Option<LspLocation> {
    let uri = v.get("uri").and_then(|u| u.as_str())?;
    let range = v.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line").and_then(|l| l.as_u64())? as usize + 1;
    let col = start.get("character").and_then(|c| c.as_u64())? as usize + 1;
    let path = Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(LspLocation {
        path: path.to_string_lossy().to_string(),
        line,
        column: col,
    })
}

fn parse_lsp_locations(v: &serde_json::Value) -> Vec<LspLocation> {
    if v.is_null() {
        return Vec::new();
    }
    if let Some(arr) = v.as_array() {
        return arr.iter().filter_map(parse_lsp_location).collect();
    }
    parse_lsp_location(v).into_iter().collect()
}

fn path_to_uri(path: &str) -> anyhow::Result<String> {
    Ok(Url::from_file_path(path)
        .map_err(|_| anyhow::anyhow!("invalid file path for uri: {path}"))?
        .to_string())
}

struct PersistentLspSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    seq: u64,
}

impl PersistentLspSession {
    async fn spawn(command: &str, workspace_root: &str) -> anyhow::Result<Self> {
        let mut cmd = Command::new(command);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to get lsp stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to get lsp stdout"))?;
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            seq: 1,
        };
        session.initialize(workspace_root).await?;
        Ok(session)
    }

    async fn initialize(&mut self, workspace_root: &str) -> anyhow::Result<()> {
        let root_uri = Url::from_file_path(workspace_root)
            .map_err(|_| anyhow::anyhow!("invalid workspace root: {workspace_root}"))?
            .to_string();
        let _ = self
            .request(
                "initialize",
                serde_json::json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {}
                }),
            )
            .await?;
        self.notify("initialized", serde_json::json!({})).await?;
        Ok(())
    }

    async fn did_open(&mut self, path: &str, language_id: &str, text: &str) -> anyhow::Result<()> {
        self.notify(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": path_to_uri(path)?,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await
    }

    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.seq;
        self.seq += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write_message(&req).await?;
        loop {
            let msg = self.read_message().await?;
            if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(anyhow::anyhow!("lsp error: {}", err));
                }
                return Ok(msg
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null));
            }
        }
    }

    async fn notify(&mut self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.write_message(&req).await
    }

    async fn write_message(&mut self, v: &serde_json::Value) -> anyhow::Result<()> {
        let body = serde_json::to_vec(v)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_message(&mut self) -> anyhow::Result<serde_json::Value> {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(Duration::from_secs(2), self.stdout.read_line(&mut line))
                .await??;
            if n == 0 {
                return Err(anyhow::anyhow!("lsp stream closed"));
            }
            if line == "\r\n" {
                break;
            }
            let lower = line.to_lowercase();
            if lower.starts_with("content-length:") {
                content_length = line
                    .split(':')
                    .nth(1)
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
            }
        }
        if content_length == 0 {
            return Err(anyhow::anyhow!("invalid lsp content-length"));
        }
        let mut buf = vec![0u8; content_length];
        tokio::time::timeout(Duration::from_secs(2), self.stdout.read_exact(&mut buf)).await??;
        Ok(serde_json::from_slice(&buf)?)
    }
}

impl Drop for PersistentLspSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
