//! Real LLM end-to-end tests — large-project-scale harness stress tests.
//!
//! These tests use the `wps-llm` plugin (zhipu/glm-5) to run real coding agent
//! workflows against a copy of the FastClaw source tree. They exercise the full
//! harness at realistic complexity (20–60+ tool calls per scenario).
//!
//! Run with:  `cargo test -p fastclaw-gateway --test e2e_real_llm -- --ignored --test-threads=1 --nocapture`

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fastclaw_agent::LlmPluginRegistry;
use fastclaw_core::llm_plugin::LlmPluginConfig;
use fastclaw_gateway::{build_app, AppState};
use fastclaw_security::{ApiKeyAuth, AuthConfig};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ===========================================================================
// Metrics
// ===========================================================================

#[derive(Debug, Default)]
struct ScenarioMetrics {
    tool_calls_made: u32,
    iterations: u32,
    tool_errors: u32,
    tool_starts: u32,
    compact_triggered: bool,
    context_usage_events: u32,
    final_used_tokens: u32,
    final_limit_tokens: u32,
    elapsed: Duration,
    completed_normally: bool,
    final_text: String,
}

impl std::fmt::Display for ScenarioMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "╔══════════════════════════════════════════════╗")?;
        writeln!(f, "║         SCENARIO METRICS REPORT              ║")?;
        writeln!(f, "╠══════════════════════════════════════════════╣")?;
        writeln!(f, "║ Tool calls made:       {:>6}                ║", self.tool_calls_made)?;
        writeln!(f, "║ Iterations:            {:>6}                ║", self.iterations)?;
        writeln!(f, "║ Tool starts observed:  {:>6}                ║", self.tool_starts)?;
        writeln!(f, "║ Tool errors:           {:>6}                ║", self.tool_errors)?;
        writeln!(f, "║ Compact triggered:     {:>6}                ║", self.compact_triggered)?;
        writeln!(f, "║ Context usage events:  {:>6}                ║", self.context_usage_events)?;
        writeln!(f, "║ Final tokens:     {:>6}/{:<6}             ║", self.final_used_tokens, self.final_limit_tokens)?;
        writeln!(f, "║ Elapsed:           {:>6.1}s                  ║", self.elapsed.as_secs_f64())?;
        writeln!(f, "║ Completed normally:    {:>6}                ║", self.completed_normally)?;
        writeln!(f, "╚══════════════════════════════════════════════╝")?;
        if !self.final_text.is_empty() {
            let preview: String = self.final_text.chars().take(200).collect();
            writeln!(f, "Final response preview: {preview}...")?;
        }
        Ok(())
    }
}

// ===========================================================================
// RealLlmServer — harness that loads the wps-llm plugin and starts gateway
// ===========================================================================

#[allow(dead_code)]
struct RealLlmServer {
    addr: SocketAddr,
    _tmp: tempfile::TempDir,
    work_dir: PathBuf,
}

impl RealLlmServer {
    async fn start(work_dir: PathBuf) -> Self {
        let plugin_config_path = dirs::home_dir()
            .expect("home dir")
            .join(".fastclaw/plugins/llm/wps-llm-provider.json");
        assert!(
            plugin_config_path.exists(),
            "wps-llm plugin config not found at {}",
            plugin_config_path.display()
        );
        let config_str =
            std::fs::read_to_string(&plugin_config_path).expect("read plugin config");
        let plugin_config: LlmPluginConfig =
            serde_json::from_str(&config_str).expect("parse plugin config");

        let registry = LlmPluginRegistry::from_configs(vec![plugin_config]);
        let provider = registry
            .create_provider("wps-llm")
            .expect("create wps-llm provider");

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
        Self {
            addr,
            _tmp: tmp,
            work_dir,
        }
    }

    fn ws_url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }

    #[allow(dead_code)]
    fn http_url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

// ===========================================================================
// WS helper functions (adapted from e2e_scenarios.rs)
// ===========================================================================

async fn ws_recv_json(
    rx: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    timeout_secs: u64,
) -> Option<Value> {
    match tokio::time::timeout(Duration::from_secs(timeout_secs), rx.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str(&t).ok(),
        _ => None,
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

/// Drive a WS chat scenario to completion, collecting all events and metrics.
/// `recv_timeout` is per-message (some tool calls can take 30+ seconds).
async fn ws_chat_collect(
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
    work_dir: Option<&str>,
    recv_timeout: u64,
) -> (Option<String>, ScenarioMetrics) {
    let mut params = json!({
        "messages": [{"role": "user", "content": user_msg}],
        "model": "zhipu/glm-5"
    });
    if let Some(sid) = session_id {
        params["sessionId"] = json!(sid);
    }
    if let Some(wd) = work_dir {
        params["workDir"] = json!(wd);
    }
    ws_send_json(tx, json!({"id": id, "method": "chat", "params": params})).await;

    let start = Instant::now();
    let mut metrics = ScenarioMetrics::default();
    let mut session_id_out = None;
    let mut accumulated_text = String::new();

    loop {
        let msg = match ws_recv_json(rx, recv_timeout).await {
            Some(m) => m,
            None => {
                eprintln!("[ws_chat_collect] recv timeout after {}s", recv_timeout);
                break;
            }
        };

        let ty = msg["type"].as_str().unwrap_or("").to_string();

        match ty.as_str() {
            "turn_start" => {
                if let Some(sid) = msg["data"]["session_id"]
                    .as_str()
                    .or_else(|| msg["data"]["sessionId"].as_str())
                {
                    session_id_out = Some(sid.to_string());
                }
            }
            "content_delta" => {
                if let Some(content) = msg["data"]["delta"]["choices"][0]["delta"]["content"].as_str() {
                    accumulated_text.push_str(content);
                }
            }
            "tool_executing" => {
                metrics.tool_starts += 1;
                let tool = msg["data"]["tool_name"].as_str().unwrap_or("?");
                eprintln!("  [tool.start] {} (#{} total)", tool, metrics.tool_starts);
            }
            "tool_result" => {
                if msg["data"]["success"].as_bool() == Some(false) {
                    metrics.tool_errors += 1;
                }
            }
            "context_usage_update" => {
                metrics.context_usage_events += 1;
                if let Some(used) = msg["data"]["used_tokens"].as_u64() {
                    metrics.final_used_tokens = used as u32;
                }
                if let Some(limit) = msg["data"]["limit_tokens"].as_u64() {
                    metrics.final_limit_tokens = limit as u32;
                }
                if msg["data"]["compressed"].as_bool() == Some(true) {
                    metrics.compact_triggered = true;
                }
            }
            "context_warning" => {
                metrics.compact_triggered = true;
            }
            "turn_end" => {
                metrics.completed_normally = true;
                if let Some(tc) = msg["data"]["summary"]["tool_calls_made"].as_u64() {
                    metrics.tool_calls_made = tc as u32;
                }
                if let Some(it) = msg["data"]["summary"]["iterations"].as_u64() {
                    metrics.iterations = it as u32;
                }
                break;
            }
            "error" => {
                eprintln!("[error] {}", msg["error"]);
                break;
            }
            _ => {}
        }
    }

    metrics.elapsed = start.elapsed();
    metrics.final_text = accumulated_text;
    (session_id_out, metrics)
}

// ===========================================================================
// Source tree copy utilities
// ===========================================================================

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Create a git worktree of the FastClaw repo for isolated testing.
/// Falls back to a full source copy if git worktree fails.
fn create_work_copy(dst: &Path) {
    let src = source_root();

    // Try git worktree first (fast, shares objects)
    let worktree_result = std::process::Command::new("git")
        .args(["worktree", "add", "--detach", &dst.to_string_lossy()])
        .current_dir(&src)
        .output();

    match worktree_result {
        Ok(output) if output.status.success() => {
            eprintln!("[setup] Created git worktree at {}", dst.display());
            return;
        }
        _ => {
            eprintln!("[setup] git worktree failed, falling back to source copy");
        }
    }

    // Fallback: copy essential directories
    let entries = ["Cargo.toml", "Cargo.lock", "crates", "extensions"];
    for entry in entries {
        let from = src.join(entry);
        if !from.exists() {
            continue;
        }
        let to = dst.join(entry);
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
    if src.join(".cargo").exists() {
        copy_dir_recursive(&src.join(".cargo"), &dst.join(".cargo"));
    }
}

/// Clean up a git worktree when the test is done.
#[allow(dead_code)]
fn cleanup_worktree(dst: &Path) {
    let src = source_root();
    let _ = std::process::Command::new("git")
        .args(["worktree", "remove", "--force", &dst.to_string_lossy()])
        .current_dir(&src)
        .output();
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dir");
    for entry in std::fs::read_dir(src).expect("read dir") {
        let entry = entry.expect("dir entry");
        let ft = entry.file_type().expect("file type");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            // Skip target/ and .git/ to avoid copying gigabytes of build artifacts
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "target" || name_str == ".git" || name_str == "node_modules" {
                continue;
            }
            copy_dir_recursive(&from, &to);
        } else if ft.is_file() {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
}

/// Dedicated target directory for E2E test builds, shared across runs
/// to avoid re-compiling everything from scratch each time.
fn e2e_target_dir() -> PathBuf {
    source_root().join("target").join("e2e-real-llm")
}

/// Run `cargo check` in the given directory and return (success, stderr).
fn run_cargo_check(work_dir: &Path) -> (bool, String) {
    let output = std::process::Command::new("cargo")
        .arg("check")
        .arg("--message-format=short")
        .current_dir(work_dir)
        .env("CARGO_TARGET_DIR", e2e_target_dir())
        .output()
        .expect("run cargo check");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stderr)
}

/// Run a specific cargo test in the given directory and return (success, combined output).
fn run_cargo_test(work_dir: &Path, test_filter: &str) -> (bool, String) {
    let output = std::process::Command::new("cargo")
        .args(["test", "--", test_filter, "--nocapture"])
        .current_dir(work_dir)
        .env("CARGO_TARGET_DIR", e2e_target_dir())
        .output()
        .expect("run cargo test");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

// ===========================================================================
// Bug seeding utilities
// ===========================================================================

#[allow(dead_code)]
struct SeedInfo {
    file: PathBuf,
    original: String,
    mutated: String,
    description: String,
}

/// Seed a compile error: change a const type from u32 to String in query_state.rs
fn seed_type_error(work_dir: &Path) -> SeedInfo {
    let file = work_dir.join("crates/fastclaw-agent/src/runtime/query_state.rs");
    let original = std::fs::read_to_string(&file).expect("read query_state.rs");

    let mutated = original.replace(
        "const TOOL_REPEAT_WARN_THRESHOLD: u32 = 3;",
        "const TOOL_REPEAT_WARN_THRESHOLD: String = String::new();",
    );
    assert_ne!(
        original, mutated,
        "seed_type_error: replacement not found in query_state.rs"
    );
    std::fs::write(&file, &mutated).expect("write seeded file");

    SeedInfo {
        file,
        original,
        mutated,
        description: "Changed TOOL_REPEAT_WARN_THRESHOLD from u32 to String in query_state.rs"
            .into(),
    }
}

/// Seed a logic bug: flip subtraction to addition in compute_compression_threshold
fn seed_logic_bug(work_dir: &Path) -> SeedInfo {
    let file = work_dir.join("crates/fastclaw-agent/src/runtime/context_compressor.rs");
    let original = std::fs::read_to_string(&file).expect("read context_compressor.rs");

    let mutated = original.replacen(
        "(threshold - 0.10).max(0.35)",
        "(threshold + 0.10).max(0.35)",
        1,
    );
    assert_ne!(
        original, mutated,
        "seed_logic_bug: replacement not found in context_compressor.rs"
    );
    std::fs::write(&file, &mutated).expect("write seeded file");

    SeedInfo {
        file,
        original,
        mutated,
        description: "Flipped (threshold - 0.10) to (threshold + 0.10) in compute_compression_threshold".into(),
    }
}

// ===========================================================================
// Scenario 1: Large project bug fix (compile error)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_1_bugfix_compile_error() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 1: Large Project Bug Fix (Compile Error)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    // Seed the bug
    let seed = seed_type_error(&work_dir);
    eprintln!("[seed] {}", seed.description);

    // Get the error output to include in the prompt
    let (seeded_ok, cargo_stderr) = run_cargo_check(&work_dir);
    assert!(!seeded_ok, "Seeded source should NOT compile");
    eprintln!(
        "[seed] cargo check failed as expected ({} bytes of stderr)",
        cargo_stderr.len()
    );

    // Truncate error output for the prompt (first 3000 chars)
    let error_preview: String = cargo_stderr.chars().take(3000).collect();

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();

    // Consume welcome message
    let welcome = ws_recv_json(&mut rx, 5).await.expect("welcome");
    assert_eq!(welcome["type"], "connected");

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "你是一个 Rust 专家。这个项目 cargo check 编译失败了，错误日志如下：\n\
         ```\n{error_preview}\n```\n\n\
         请分析错误原因，定位到出问题的代码，修复它，然后再次运行 cargo check 确认修复成功。\n\
         项目根目录: {work_dir}\n\n\
         注意：运行 cargo 命令时，请在命令前加上 `CARGO_TARGET_DIR={target_dir}` 环境变量，\
         例如: CARGO_TARGET_DIR={target_dir} cargo check --message-format=short",
        work_dir = work_dir.display(),
        target_dir = target_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(300), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "bugfix-1",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (session_id, metrics) = timeout.await.expect("scenario 1 timed out (300s)");

    eprintln!("\n{metrics}");

    // Assertions
    assert!(
        metrics.completed_normally,
        "Scenario should complete normally (not error/timeout)"
    );
    assert!(
        metrics.tool_calls_made >= 3,
        "Should make at least 3 tool calls (read/edit/shell), got {}",
        metrics.tool_calls_made
    );
    assert!(
        metrics.tool_starts >= 3,
        "Should observe at least 3 tool starts, got {}",
        metrics.tool_starts
    );
    assert!(
        !metrics.final_text.is_empty(),
        "Should produce a final text response"
    );

    // Verify the fix: try cargo check on the modified source
    let (fixed_ok, fix_stderr) = run_cargo_check(&work_dir);
    eprintln!(
        "[verify] Post-fix cargo check: {}",
        if fixed_ok { "PASS" } else { "FAIL" }
    );
    if !fixed_ok {
        eprintln!("[verify] Remaining errors:\n{}", &fix_stderr[..fix_stderr.len().min(1000)]);
    }

    eprintln!(
        "[result] session={}, tool_calls={}, iterations={}, fixed={}",
        session_id.as_deref().unwrap_or("?"),
        metrics.tool_calls_made,
        metrics.iterations,
        fixed_ok
    );
}

// ===========================================================================
// Scenario 2: Add a new built-in tool
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_2_add_new_tool() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 2: Add New Built-in Tool (checksum_tool)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "你是一个 Rust 专家。请给这个项目新增一个 built-in tool，叫做 `checksum_tool`：\n\
         - 功能：接受一个 `file_path` 参数，计算该文件的 SHA256 校验和并返回十六进制字符串\n\
         - 位置：在 crates/fastclaw-agent/src/builtin_tools/ 目录下新建 checksum.rs\n\
         - 参考现有 tool 的实现模式（先读取 filesystem.rs 中的 ReadFileTool 了解 Tool trait 的实现方式）\n\
         - 实现 Tool trait 的所有必需方法: name(), description(), parameters_schema(), execute()\n\
         - 在 builtin_tools/mod.rs 中添加 `mod checksum;` 声明和 `use checksum::ChecksumTool;`\n\
         - 在 `register_builtin_tools_with_sandbox` 函数中注册: `registry.register_deferred(Arc::new(ChecksumTool));`\n\
         - 最后运行 cargo check 确认编译通过\n\n\
         项目根目录: {work_dir}\n\
         运行 cargo 命令时请加上 CARGO_TARGET_DIR={target_dir}",
        work_dir = work_dir.display(),
        target_dir = target_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(600), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "feature-1",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (session_id, metrics) = timeout.await.expect("scenario 2 timed out (600s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 5,
        "Should make at least 5 tool calls, got {}",
        metrics.tool_calls_made
    );

    // Verify file was created
    let checksum_file = work_dir.join("crates/fastclaw-agent/src/builtin_tools/checksum.rs");
    let file_exists = checksum_file.exists();
    eprintln!(
        "[verify] checksum.rs exists: {}",
        file_exists
    );

    if file_exists {
        let content = std::fs::read_to_string(&checksum_file).unwrap_or_default();
        let has_trait_impl = content.contains("fn execute") || content.contains("fn name");
        eprintln!("[verify] checksum.rs has Tool impl: {}", has_trait_impl);
    }

    // Verify mod.rs was modified
    let mod_file = work_dir.join("crates/fastclaw-agent/src/builtin_tools/mod.rs");
    let mod_content = std::fs::read_to_string(&mod_file).unwrap_or_default();
    let mod_declared = mod_content.contains("mod checksum") || mod_content.contains("checksum");
    eprintln!("[verify] mod.rs references checksum: {}", mod_declared);

    // Try cargo check
    let (check_ok, check_stderr) = run_cargo_check(&work_dir);
    eprintln!(
        "[verify] Post-feature cargo check: {}",
        if check_ok { "PASS" } else { "FAIL" }
    );
    if !check_ok {
        let preview: String = check_stderr.chars().take(1000).collect();
        eprintln!("[verify] Errors:\n{preview}");
    }

    eprintln!(
        "[result] session={}, tool_calls={}, iterations={}, check_pass={}, file_exists={}",
        session_id.as_deref().unwrap_or("?"),
        metrics.tool_calls_made,
        metrics.iterations,
        check_ok,
        file_exists
    );
}

// ===========================================================================
// Scenario 3: Cross-file refactoring
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_3_refactor_extract_module() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 3: Cross-File Refactoring (Extract Module)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "你是一个 Rust 重构专家。请将 crates/fastclaw-agent/src/autofix.rs 中的 \
         `CompilerDiagnostic` struct 和相关的类型/函数提取到一个新的子模块中。\n\n\
         具体步骤：\n\
         1. 先读取 autofix.rs 了解文件结构\n\
         2. 创建目录 crates/fastclaw-agent/src/autofix/\n\
         3. 将原 autofix.rs 重命名/移动为 autofix/mod.rs\n\
         4. 创建 autofix/diagnostics.rs，把以下类型移入：Severity, CompilerKind, CompilerDiagnostic\n\
         5. 在 autofix/mod.rs 中添加 `mod diagnostics;` 并 `pub(crate) use diagnostics::*;` 确保外部引用不变\n\
         6. 运行 cargo check 确认编译通过\n\n\
         项目根目录: {work_dir}\n\
         运行 cargo 命令时请加上 CARGO_TARGET_DIR={target_dir}",
        work_dir = work_dir.display(),
        target_dir = target_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(600), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "refactor-1",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (session_id, metrics) = timeout.await.expect("scenario 3 timed out (600s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 8,
        "Should make at least 8 tool calls for a refactor, got {}",
        metrics.tool_calls_made
    );

    // Check that the refactored structure exists
    let autofix_dir = work_dir.join("crates/fastclaw-agent/src/autofix");
    let mod_exists = autofix_dir.join("mod.rs").exists();
    let diag_exists = autofix_dir.join("diagnostics.rs").exists();
    eprintln!("[verify] autofix/mod.rs exists: {}", mod_exists);
    eprintln!("[verify] autofix/diagnostics.rs exists: {}", diag_exists);

    if diag_exists {
        let content = std::fs::read_to_string(autofix_dir.join("diagnostics.rs")).unwrap_or_default();
        let has_diagnostic = content.contains("CompilerDiagnostic");
        eprintln!(
            "[verify] diagnostics.rs contains CompilerDiagnostic: {}",
            has_diagnostic
        );
    }

    let (check_ok, check_stderr) = run_cargo_check(&work_dir);
    eprintln!(
        "[verify] Post-refactor cargo check: {}",
        if check_ok { "PASS" } else { "FAIL" }
    );
    if !check_ok {
        let preview: String = check_stderr.chars().take(1000).collect();
        eprintln!("[verify] Errors:\n{preview}");
    }

    eprintln!(
        "[result] session={}, tool_calls={}, iterations={}, check_pass={}",
        session_id.as_deref().unwrap_or("?"),
        metrics.tool_calls_made,
        metrics.iterations,
        check_ok
    );
}

// ===========================================================================
// Scenario 4: Test-driven debugging (logic bug)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_4_debug_failing_test() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 4: Test-Driven Debugging (Logic Bug)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    // Seed the logic bug
    let seed = seed_logic_bug(&work_dir);
    eprintln!("[seed] {}", seed.description);

    // Run the test to capture the failure output
    let test_filter = "dynamic_threshold_lowers_with_large_system_prompt";
    let (test_ok, test_output) = run_cargo_test(&work_dir, test_filter);
    if test_ok {
        eprintln!(
            "[skip] Test '{}' still passes after seeding — the specific test may not exist or the \
             mutation didn't affect it. Skipping scenario.",
            test_filter
        );
        return;
    }
    eprintln!(
        "[seed] Test failed as expected ({} bytes of output)",
        test_output.len()
    );

    let output_preview: String = test_output.chars().take(3000).collect();

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "你是一个 Rust 调试专家。这个项目的一个单元测试失败了，测试输出如下：\n\
         ```\n{output_preview}\n```\n\n\
         请分析测试输出，找到测试所在的源文件，阅读相关代码，定位 bug 原因并修复。\n\
         修复后重新运行这个测试确认通过。\n\n\
         项目根目录: {work_dir}\n\
         运行 cargo 命令时请加上 CARGO_TARGET_DIR={target_dir}\n\
         运行测试命令: CARGO_TARGET_DIR={target_dir} cargo test -p fastclaw-agent -- {test_filter} --nocapture",
        work_dir = work_dir.display(),
        target_dir = target_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(600), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "debug-1",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (session_id, metrics) = timeout.await.expect("scenario 4 timed out (600s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 4,
        "Should make at least 4 tool calls (read/edit/test), got {}",
        metrics.tool_calls_made
    );

    // Verify the fix
    let fixed_content = std::fs::read_to_string(&seed.file).unwrap_or_default();
    let has_correct_subtraction = fixed_content.contains("threshold - 0.10")
        || fixed_content.contains("threshold - 0.1");
    eprintln!(
        "[verify] File contains correct subtraction: {}",
        has_correct_subtraction
    );

    let (test_pass, _) = run_cargo_test(&work_dir, test_filter);
    eprintln!(
        "[verify] Post-fix test: {}",
        if test_pass { "PASS" } else { "FAIL" }
    );

    eprintln!(
        "[result] session={}, tool_calls={}, iterations={}, test_pass={}",
        session_id.as_deref().unwrap_or("?"),
        metrics.tool_calls_made,
        metrics.iterations,
        test_pass
    );
}

// ===========================================================================
// Scenario 5: Endurance test — 50+ round multi-turn session
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_5_endurance_multi_turn() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 5: Endurance Test (Multi-Turn Session, 50+ tools)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let cargo_prefix = format!("CARGO_TARGET_DIR={}", target_dir.display());
    let turns: Vec<String> = vec![
        format!("读取 {}/crates/fastclaw-core/src/config.rs 的前 80 行，告诉我 FastClawConfig 有哪些字段", work_dir.display()),
        "在 FastClawConfig struct 中新增一个字段 `pub e2e_test_marker: Option<String>`，加上 `#[serde(default)]` 属性".into(),
        format!("运行 {} cargo check --message-format=short 确认编译通过", cargo_prefix),
        "搜索项目中哪些文件使用了 FastClawConfig（用 grep 搜索）".into(),
        format!("读取 {}/crates/fastclaw-gateway/src/state/mod.rs 的前 50 行", work_dir.display()),
        "在 state/mod.rs 的 for_test 函数中，给 config 设置 e2e_test_marker = Some(\"harness-v1\".to_string())".into(),
        format!("运行 {} cargo check --message-format=short 确认编译通过", cargo_prefix),
        format!("读取 {}/crates/fastclaw-agent/src/runtime/query_state.rs 的前 40 行", work_dir.display()),
        "告诉我 QueryLoopState 结构体有多少个字段".into(),
        "在 QueryLoopState 中新增一个字段 `pub e2e_turn_count: u32`，并在 new() 中初始化为 0".into(),
        format!("运行 {} cargo check --message-format=short 确认编译通过", cargo_prefix),
        "回忆一下：我们在第 2 步给 FastClawConfig 添加了什么字段？".into(),
        format!("读取 {}/crates/fastclaw-core/src/config.rs 确认 e2e_test_marker 字段确实存在", work_dir.display()),
        "把 FastClawConfig 中 e2e_test_marker 的类型从 Option<String> 改为 Option<u32>".into(),
        format!("运行 {} cargo check —— 如果有编译错误，请修复所有引用这个字段的地方", cargo_prefix),
        "回忆一下我们这次对话中做了哪些修改？请列一个清单".into(),
        "把所有我们的修改都撤销回去（删除 e2e_test_marker 字段，删除 e2e_turn_count 字段），恢复到原始状态".into(),
        format!("运行 {} cargo check 确认恢复后编译通过", cargo_prefix),
    ];

    let mut session_id: Option<String> = None;
    let mut total_metrics = ScenarioMetrics::default();
    let start = Instant::now();

    for (i, turn) in turns.iter().enumerate() {
        eprintln!("\n--- Turn {}/{} ---", i + 1, turns.len());
        let turn_preview: String = turn.chars().take(80).collect();
        eprintln!("[prompt] {}", turn_preview);

        let timeout = tokio::time::timeout(Duration::from_secs(300), async {
            ws_chat_collect(
                &mut tx,
                &mut rx,
                &format!("turn-{}", i + 1),
                turn,
                session_id.as_deref(),
                Some(&work_dir.to_string_lossy()),
                120,
            )
            .await
        });

        match timeout.await {
            Ok((sid, turn_metrics)) => {
                if session_id.is_none() {
                    session_id = sid;
                }
                eprintln!(
                    "  [turn {}] tools={}, iterations={}, errors={}, elapsed={:.1}s",
                    i + 1,
                    turn_metrics.tool_calls_made,
                    turn_metrics.iterations,
                    turn_metrics.tool_errors,
                    turn_metrics.elapsed.as_secs_f64()
                );
                total_metrics.tool_calls_made += turn_metrics.tool_calls_made;
                total_metrics.iterations += turn_metrics.iterations;
                total_metrics.tool_errors += turn_metrics.tool_errors;
                total_metrics.tool_starts += turn_metrics.tool_starts;
                total_metrics.context_usage_events += turn_metrics.context_usage_events;
                if turn_metrics.compact_triggered {
                    total_metrics.compact_triggered = true;
                }
                total_metrics.final_used_tokens = turn_metrics.final_used_tokens;
                total_metrics.final_limit_tokens = turn_metrics.final_limit_tokens;
                total_metrics.completed_normally = turn_metrics.completed_normally;
                total_metrics.final_text = turn_metrics.final_text;

                if !turn_metrics.completed_normally {
                    eprintln!("  [WARN] Turn {} did not complete normally, stopping", i + 1);
                    break;
                }
            }
            Err(_) => {
                eprintln!("  [TIMEOUT] Turn {} timed out after 300s, stopping", i + 1);
                break;
            }
        }
    }

    total_metrics.elapsed = start.elapsed();
    eprintln!("\n--- ENDURANCE TEST AGGREGATE ---");
    eprintln!("{total_metrics}");

    // Assertions
    assert!(
        total_metrics.tool_calls_made >= 10,
        "Endurance test should accumulate at least 10 tool calls across all turns, got {}",
        total_metrics.tool_calls_made
    );
    assert!(
        total_metrics.iterations >= 15,
        "Should have at least 15 LLM iterations across all turns, got {}",
        total_metrics.iterations
    );
    assert!(
        total_metrics.context_usage_events >= 5,
        "Should receive multiple context usage updates, got {}",
        total_metrics.context_usage_events
    );

    // Verify final state: the source should be clean after undo
    let (final_check_ok, _) = run_cargo_check(&work_dir);
    eprintln!(
        "[verify] Final cargo check (after undo): {}",
        if final_check_ok { "PASS" } else { "FAIL" }
    );

    eprintln!(
        "[result] session={}, total_tool_calls={}, total_iterations={}, compact={}, elapsed={:.1}s",
        session_id.as_deref().unwrap_or("?"),
        total_metrics.tool_calls_made,
        total_metrics.iterations,
        total_metrics.compact_triggered,
        total_metrics.elapsed.as_secs_f64()
    );
}

// ===========================================================================
// Scenario 6: Task Decomposer — complex multi-step request
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_6_task_decomposer() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 6: Task Decomposer (Complex Multi-Step Request)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "请完成以下复杂任务（多步骤）：\n\n\
         1. 读取 {work_dir}/crates/fastclaw-core/src/config.rs，了解 FastClawConfig 结构\n\
         2. 在 FastClawConfig 中新增一个字段 `pub decomposer_test_flag: Option<bool>`，加上 `#[serde(default)]`\n\
         3. 在 {work_dir}/crates/fastclaw-core/src/config.rs 的 impl Default 中初始化这个字段为 None\n\
         4. 运行 CARGO_TARGET_DIR={target_dir} cargo check --message-format=short 验证编译通过\n\
         5. 搜索整个项目中哪些文件 import 了 FastClawConfig\n\
         6. 汇总结果告诉我\n\n\
         注意：先通读文件理解结构，再做修改，最后验证。项目根目录: {work_dir}",
        work_dir = work_dir.display(),
        target_dir = target_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(300), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "decomposer-6",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (_session_id, metrics) = timeout.await.expect("scenario 6 timed out (300s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario 6 should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 4,
        "Complex task should trigger at least 4 tool calls (read/edit/shell/grep), got {}",
        metrics.tool_calls_made
    );
    assert!(
        !metrics.final_text.is_empty(),
        "Should produce a summary response"
    );

    let (check_ok, _) = run_cargo_check(&work_dir);
    eprintln!(
        "[verify] Post-task cargo check: {}",
        if check_ok { "PASS" } else { "FAIL" }
    );
}

// ===========================================================================
// Scenario 7: ValidationPipeline — tool output validation
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_7_validation_pipeline() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 7: ValidationPipeline (Tool Output Validation)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    // Ask the agent to run some shell commands — the ValidationPipeline should
    // append warnings for potentially dangerous patterns.
    let prompt = format!(
        "请在项目目录 {work_dir} 中执行以下操作：\n\
         1. 运行 `ls -la` 查看项目根目录内容\n\
         2. 运行 `wc -l crates/fastclaw-agent/src/runtime/mod.rs` 统计行数\n\
         3. 创建一个临时文件 {work_dir}/test_validation.txt，内容为 'hello validation pipeline'\n\
         4. 读取刚创建的文件确认内容正确\n\
         5. 删除这个临时文件\n\
         6. 告诉我每步的结果",
        work_dir = work_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(180), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "validation-7",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            90,
        )
        .await
    });

    let (_session_id, metrics) = timeout.await.expect("scenario 7 timed out (180s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario 7 should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 4,
        "Should use at least 4 tool calls for the multi-step shell task, got {}",
        metrics.tool_calls_made
    );
}

// ===========================================================================
// Scenario 8: Undo Engine — repeated failures trigger rollback
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_8_undo_engine_rollback() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 8: Undo Engine (Repeated Failures + Rollback)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    // Seed an intentionally hard-to-fix error to trigger repeated failures
    let target_file = work_dir.join("crates/fastclaw-agent/src/runtime/query_state.rs");
    let original = std::fs::read_to_string(&target_file).expect("read query_state.rs");
    let mutated = original.replace(
        "pub fn new(max_iterations: u32) -> Self {",
        "pub fn new(max_iterations: NonExistentType) -> Self {",
    );
    assert_ne!(original, mutated, "mutation did not apply");
    std::fs::write(&target_file, &mutated).expect("write mutated file");

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    let target_dir = e2e_target_dir();
    let prompt = format!(
        "这个项目编译失败了。请修复 {file} 中的编译错误。\n\
         运行 CARGO_TARGET_DIR={target_dir} cargo check --message-format=short 查看错误。\n\
         项目根目录: {work_dir}\n\n\
         注意：如果某种修复方法不行，请尝试其他方案。",
        file = target_file.display(),
        target_dir = target_dir.display(),
        work_dir = work_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(300), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "undo-8",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            120,
        )
        .await
    });

    let (_session_id, metrics) = timeout.await.expect("scenario 8 timed out (300s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario 8 should complete (even if fix failed)"
    );
    assert!(
        metrics.tool_calls_made >= 3,
        "Should attempt multiple tool calls trying to fix, got {}",
        metrics.tool_calls_made
    );
}

// ===========================================================================
// Scenario 9: Context Assembly — project detection and magic docs
// ===========================================================================

#[tokio::test]
#[ignore]
async fn scenario_9_context_assembly() {
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("SCENARIO 9: Context Assembly (Project Hints + Documentation)");
    eprintln!("{}\n", "=".repeat(60));

    let work_tmp = tempfile::tempdir().expect("work tmpdir");
    let work_dir = work_tmp.path().to_path_buf();
    eprintln!("[setup] Copying source tree to {} ...", work_dir.display());
    create_work_copy(&work_dir);

    let srv = RealLlmServer::start(work_dir.clone()).await;
    let (ws, _) = connect_async(&srv.ws_url()).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();
    let _ = ws_recv_json(&mut rx, 5).await;

    // Ask about the project structure — context assembly should detect
    // Cargo.toml and inject "Rust project (Cargo)" hint automatically.
    let prompt = format!(
        "分析一下 {work_dir} 这个项目的结构：\n\
         1. 这是什么语言的项目？\n\
         2. 主要包含哪些 crate？\n\
         3. 项目的 Cargo.toml 中定义了哪些 workspace members？\n\
         4. 简要说明每个 crate 的用途\n\n\
         请直接读取文件回答，不要猜测。",
        work_dir = work_dir.display()
    );

    let timeout = tokio::time::timeout(Duration::from_secs(180), async {
        ws_chat_collect(
            &mut tx,
            &mut rx,
            "context-9",
            &prompt,
            None,
            Some(&work_dir.to_string_lossy()),
            90,
        )
        .await
    });

    let (_session_id, metrics) = timeout.await.expect("scenario 9 timed out (180s)");
    eprintln!("\n{metrics}");

    assert!(
        metrics.completed_normally,
        "Scenario 9 should complete normally"
    );
    assert!(
        metrics.tool_calls_made >= 2,
        "Should read at least Cargo.toml and some crate files, got {}",
        metrics.tool_calls_made
    );
    assert!(
        !metrics.final_text.is_empty(),
        "Should produce a comprehensive project analysis"
    );
    // The response should mention Rust since context_assembly detects Cargo.toml
    let lower_text = metrics.final_text.to_lowercase();
    assert!(
        lower_text.contains("rust") || lower_text.contains("cargo"),
        "Response should identify the project as Rust/Cargo-based"
    );
}
