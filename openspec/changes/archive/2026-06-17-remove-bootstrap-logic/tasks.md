## 1. 移除 workspace.rs 中的 bootstrap 逻辑

- [x] 1.1 删除 `DEFAULT_BOOTSTRAP_FILENAME` 常量和 `DEFAULT_BOOTSTRAP_TEMPLATE` 模板
- [x] 1.2 从 `BOOTSTRAP_FILES` 数组中移除 BOOTSTRAP.md 条目
- [x] 1.3 从 `WorkspaceBootstrap` 结构体中移除 `bootstrap: Option<String>` 字段
- [x] 1.4 将 `ensure_bootstrap()` 重命名为 `ensure_workspace()`，移除创建 BOOTSTRAP.md 的逻辑
- [x] 1.5 更新 `load_bootstrap()` 中 match 分支，移除 BOOTSTRAP.md 读取

## 2. 移除 engine.rs 中的 bootstrap 注入

- [x] 2.1 删除读取 BOOTSTRAP.md 的代码（`Self::read_file(root, DEFAULT_BOOTSTRAP_FILENAME)`）
- [x] 2.2 删除 "Bootstrap Pending" 消息注入的整个 `if let Some(ref bootstrap_content)` 代码块（包括刚添加的 `identity_already_configured` 防御逻辑）

## 3. 更新调用方

- [x] 3.1 `crates/xiaolin-gateway/src/state/builder.rs` — 将 `ensure_bootstrap()` 调用改为 `ensure_workspace()`
- [x] 3.2 `crates/xiaolin-app/src-tauri/src/commands/agent.rs` — 将 `ensure_bootstrap()` 调用改为 `ensure_workspace()`

## 4. 清理测试

- [x] 4.1 更新 `workspace.rs` 中的 `ensure_bootstrap_creates_identity_files` 测试 → 重命名并验证不再创建 BOOTSTRAP.md
- [x] 4.2 移除或更新 `load_bootstrap_includes_bootstrap_file` 测试
- [x] 4.3 运行 `cargo test` 和 `cargo clippy -- -D warnings` 验证无报错
