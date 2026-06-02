## 1. 合并过薄 crate

- [x] 1.1 将 `xiaolin-path` 源码移入 `xiaolin-core/src/path.rs`，更新 `lib.rs` 导出
- [x] 1.2 全局替换 `xiaolin_path::` 为 `xiaolin_core::path::`，更新所有下游 Cargo.toml
- [x] 1.3 删除 `crates/xiaolin-path` 目录和 workspace 成员
- [x] 1.4 将 `xiaolin-hardening` 源码移入 `xiaolin-core/src/hardening.rs`，更新 `lib.rs` 导出
- [x] 1.5 全局替换 `xiaolin_hardening::` 为 `xiaolin_core::hardening::`，更新所有下游 Cargo.toml
- [x] 1.6 删除 `crates/xiaolin-hardening` 目录和 workspace 成员
- [x] 1.7 `cargo check --workspace` 通过

## 2. 创建 xiaolin-tools-fs

- [ ] 2.1 创建 `crates/xiaolin-tools-fs/` 目录和 Cargo.toml（依赖 xiaolin-core, xiaolin-security）
- [ ] 2.2 迁移工具模块：filesystem, shell, shell_readonly, shell_security, shell_path_validation, terminal, worktree, exec_command
- [ ] 2.3 导出 `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)`
- [ ] 2.4 在 `xiaolin-agent` 中调用 `xiaolin_tools_fs::register()`
- [ ] 2.5 迁移相关测试
- [ ] 2.6 `cargo check --workspace` 通过

## 3. 创建 xiaolin-tools-network

- [ ] 3.1 创建 `crates/xiaolin-tools-network/` 目录和 Cargo.toml（依赖 xiaolin-core）
- [ ] 3.2 迁移工具模块：network（http_fetch, web_search, web_fetch）
- [ ] 3.3 导出 `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)`
- [ ] 3.4 在 `xiaolin-agent` 中调用 `xiaolin_tools_network::register()`
- [ ] 3.5 `cargo check --workspace` 通过

## 4. 创建 xiaolin-tools-browser

- [ ] 4.1 创建 `crates/xiaolin-tools-browser/` 目录和 Cargo.toml（依赖 xiaolin-core，feature-gated）
- [ ] 4.2 迁移工具模块：browser（完整的 CDP 自动化栈）
- [ ] 4.3 导出 `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)`
- [ ] 4.4 在 `xiaolin-agent` 中 feature-gate 调用 `xiaolin_tools_browser::register()`
- [ ] 4.5 `cargo check --workspace` 通过（含和不含 `browser` feature）

## 5. 创建 xiaolin-tools-code

- [ ] 5.1 创建 `crates/xiaolin-tools-code/` 目录和 Cargo.toml（依赖 xiaolin-core, xiaolin-treesitter）
- [ ] 5.2 迁移工具模块：code_intel, lsp_manager, notebook
- [ ] 5.3 导出 `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)`
- [ ] 5.4 在 `xiaolin-agent` 中调用 `xiaolin_tools_code::register()`
- [ ] 5.5 `cargo check --workspace` 通过

## 6. MCP 并发改进

- [ ] 6.1 在 `McpClient` 内部添加 `pending: HashMap<Value, oneshot::Sender<JsonRpcResponse>>` 请求映射
- [ ] 6.2 实现后台 reader task，按 JSON-RPC id 分发响应到对应 oneshot channel
- [ ] 6.3 `call_tool` 方法改为生成唯一 id、注册 oneshot、发送请求、await 响应
- [ ] 6.4 添加请求超时机制（默认 30s）
- [ ] 6.5 添加 server 进程崩溃时清理所有 pending requests 的逻辑
- [ ] 6.6 移除 `Arc<Mutex<McpClient>>` 外层包装
- [ ] 6.7 添加并发调用测试

## 7. Gateway 模块边界清理

- [ ] 7.1 将 `xiaolin-gateway/src/` 按职责整理为 `chat/`, `admin/`, `mcp/`, `cron/` 子模块
- [ ] 7.2 确保各子模块之间通过 `AppState` 交互而非直接交叉引用
- [ ] 7.3 `cargo clippy --workspace -- -D warnings` 通过

## 8. 最终验证

- [ ] 8.1 `cargo check --workspace` 通过
- [ ] 8.2 `cargo clippy --workspace -- -D warnings` 零警告
- [ ] 8.3 `cargo test --workspace` 通过
- [ ] 8.4 前端 `npx tsc --noEmit` 通过
