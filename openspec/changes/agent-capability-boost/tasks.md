## 1. Shell 工具 Prompt 增强 (P0)

- [ ] 1.1 在 `crates/xiaolin-tools-fs/src/shell.rs`（或 `exec_command.rs`）为 shell 工具实现 `prompt()` 方法，~2k 字，涵盖工具路由、git 规范、并行指引、反模式
- [ ] 1.2 验证 prompt 不与 system prompt 的 `using_tools` 决策树冲突/重复，必要时精简 system prompt 中的 shell 段
- [ ] 1.3 `cargo check` + `cargo clippy -- -D warnings` 通过

## 2. TodoWrite Prompt 示例增强 (P0)

- [ ] 2.1 在 todo_write 工具的 prompt 中追加 3 段 `<example>` + `<reasoning>` 叙事示例（正面/负面/边界场景）
- [ ] 2.2 确保现有 when/skip 规则保留不变，示例仅作为补充
- [ ] 2.3 `cargo check` + `cargo clippy -- -D warnings` 通过

## 3. MCP 工具默认 Defer + 消除双重注入 (P0)

- [ ] 3.1 `McpToolBridge` override `exposure()` → `_meta.alwaysLoad` ? Direct : Deferred
- [ ] 3.2 移除或降级 `maybe_defer_mcp_tools()` 为 safety net（MCP 已默认 deferred，仅处理 legacy）
- [ ] 3.3 `inject_mcp_tools_prompt()` 对 eager MCP 工具不再重复 schema，仅列名字
- [ ] 3.4 `tool_search` select 模式返回完整 schema（name + description + parameters）
- [ ] 3.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 4. API Error 防护 (P2)

- [ ] 4.1 `llm_call.rs` PTL 连接失败路径共用 `has_attempted_reactive_compact` guard
- [ ] 4.2 `QueryLoopState` 新增 `max_output_recovery_exhausted: bool`，恢复耗尽后设为 true
- [ ] 4.3 `stop_hooks.rs` 传入 `max_output_recovery_exhausted` 和 `has_attempted_reactive_compact` + context 占比，满足条件时跳过 continuation hooks
- [ ] 4.4 补充单元测试：PTL 连接失败仅 compact 一次、max_output 耗尽后 truncation hook 不触发
- [ ] 4.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 5. Stale Detection Content Fallback (P2)

- [ ] 5.1 `file_state_cache.rs` 的 `check_stale()` 增加 content hash 比对分支：mtime 变但 hash 同 → Fresh
- [ ] 5.2 补充单元测试：mtime 变 content 同 → Fresh；mtime 变 content 异 → Stale
- [ ] 5.3 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 6. Context Collapse 激活 (P1)

- [ ] 6.1 `BehaviorConfig` 新增 `enable_collapse: bool`（默认 false），`turn_setup.rs` 读取并传递给 `PipelineConfig`
- [ ] 6.2 实现 `CollapseSummarizer` 桥接到 LLM provider（新文件 `collapse_summarizer.rs`）
- [ ] 6.3 `unified_compact.rs` 在 microcompact 后、LLM autocompact 前调用 `CollapseEngine::collapse()` + `project()`
- [ ] 6.4 确保 collapse 启用时 autocompact 被正确抑制（互斥逻辑验证）
- [ ] 6.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 7. Post-compact 恢复完善 (P1)

- [ ] 7.1 `post_compact_restore.rs` 补充 deferred tools delta 重发逻辑
- [ ] 7.2 compact 后 `readFileState` / `FileStateCache` 重置，防止 stale cache
- [ ] 7.3 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过
