## 1. 合并过薄 crate

- [x] 1.1 将 `xiaolin-path` 源码移入 `xiaolin-core/src/path.rs`，更新 `lib.rs` 导出
- [x] 1.2 全局替换 `xiaolin_path::` 为 `xiaolin_core::path::`，更新所有下游 Cargo.toml
- [x] 1.3 删除 `crates/xiaolin-path` 目录和 workspace 成员
- [x] 1.4 将 `xiaolin-hardening` 源码移入 `xiaolin-core/src/hardening.rs`，更新 `lib.rs` 导出
- [x] 1.5 全局替换 `xiaolin_hardening::` 为 `xiaolin_core::hardening::`，更新所有下游 Cargo.toml
- [x] 1.6 删除 `crates/xiaolin-hardening` 目录和 workspace 成员
- [x] 1.7 `cargo check --workspace` 通过

## 2. 创建 xiaolin-tools-fs

- [x] 2.1 创建 `crates/xiaolin-tools-fs/` 目录和 Cargo.toml（依赖 xiaolin-core, xiaolin-treesitter）
- [x] 2.2 迁移工具模块：filesystem, shell, shell_readonly, shell_security, shell_path_validation, terminal, worktree, exec_command + file_state_cache
- [x] 2.3 导出模块和公共类型，xiaolin-agent 通过 re-export 保持 API 兼容
- [x] 2.4 在 `xiaolin-agent` 中依赖 `xiaolin-tools-fs` 并 re-export 模块
- [x] 2.5 迁移相关测试（随模块内联 `#[cfg(test)]` 一并迁移）
- [x] 2.6 `cargo check --workspace` 通过

## 3. 创建 xiaolin-tools-network

- [x] 3.1 创建 `crates/xiaolin-tools-network/` 目录和 Cargo.toml（依赖 xiaolin-core, xiaolin-security）
- [x] 3.2 迁移工具模块：network（http_fetch, web_search, web_fetch）
- [x] 3.3 导出公共类型，xiaolin-agent 通过 re-export 保持 API 兼容
- [x] 3.4 在 `xiaolin-agent` 中依赖 `xiaolin-tools-network` 并 re-export
- [x] 3.5 `cargo check --workspace` 通过

## 4. 创建 xiaolin-tools-browser

- [x] 4.1 创建 `crates/xiaolin-tools-browser/` 目录和 Cargo.toml（依赖 xiaolin-core, headless_chrome）
- [x] 4.2 迁移工具模块：browser（完整的 CDP 自动化栈）
- [x] 4.3 导出公共类型，xiaolin-agent 通过 `pub use xiaolin_tools_browser as browser` 保持兼容
- [x] 4.4 在 `xiaolin-agent` 中 feature-gate `browser = ["dep:xiaolin-tools-browser"]`
- [x] 4.5 `cargo check --workspace` 通过

## 5. 创建 xiaolin-tools-code

- [x] 5.1 创建 `crates/xiaolin-tools-code/` 目录和 Cargo.toml（依赖 xiaolin-core, xiaolin-treesitter, xiaolin-tools-fs）
- [x] 5.2 迁移工具模块：code_intel, lsp_manager, notebook + symbol_index
- [x] 5.3 导出公共类型，xiaolin-agent 通过 re-export 保持 API 兼容
- [x] 5.4 在 `xiaolin-agent` 中依赖 `xiaolin-tools-code` 并 re-export
- [x] 5.5 `cargo check --workspace` 通过

## 6. MCP 并发改进

- [x] 6.1 在 `McpClient` stdio 传输添加 `pending: HashMap` 请求映射（与 SSE 统一）
- [x] 6.2 实现后台 `stdio_reader_loop` task，按 JSON-RPC id 分发响应到 oneshot channel
- [x] 6.3 `call_tool`/`send_request` 改为 `&self`，生成唯一 id、注册 oneshot、发送请求、await 响应
- [x] 6.4 添加请求超时机制（默认 30s），统一 `await_pending_response` 辅助函数
- [x] 6.5 reader task EOF 时清理所有 pending requests，发送 `-32603` 错误响应
- [x] 6.6 `SharedMcpClient` 从 `Arc<Mutex<McpClient>>` 改为 `Arc<McpClient>`
- [x] 6.7 23 个测试全部通过（含 SSE mock、POST error、resource/prompt 测试）

## 7. Gateway 模块边界清理

- [x] 7.1 审查 gateway 模块结构，确认已按职责组织（routes/, ws/, state/ + 独立工具模块）
- [x] 7.2 验证模块间仅通过 AppState 和工具函数交互，无不当直接交叉引用
- [x] 7.3 `cargo clippy --workspace -- -D warnings` 零警告通过

## 8. 最终验证

- [x] 8.1 `cargo check --workspace` 通过
- [x] 8.2 `cargo clippy --workspace -- -D warnings` 零警告
- [x] 8.3 `cargo test --workspace` 通过（151 passed，1 failed 为预存在的 migration 测试 bug）
- [x] 8.4 修复多处预存在的测试编译问题（session-actor typed_data、core test_support 导入、sandbox 字段）
