## 1. 阶段1：code-intel 返回片段化（最高 ROI，作用对象=统一 `lsp` 工具）

- [x] 1.1 新增共享 `xiaolin-tools-fs/src/snippet.rs::line_snippet`（按整行切片，UTF-8 安全；单条 `MAX_SNIPPET_CHARS=600` char 安全截断）+ 4 单测
- [x] 1.2 `code_intel.rs` 新增 `SnippetLoader`：输入文件零 IO 切 `full_file.output`（canonicalize 比对）；跨文件按 path 缓存 + `MAX_CROSS_FILE_SNIPPET_READS=20` 读次上限 + `MAX_SNIPPET_SOURCE_BYTES=2MB` 大小上限
- [x] 1.3 `go_to_definition`：LSP 路径用 loader 填 `snippet`；symbol_index（单/多）路径用 `signature` 降级补 `snippet`
- [x] 1.4 `find_references`：LSP 路径逐条填 `snippet`；symbol_index 用 signature 降级；**ripgrep 回退 `text`→`snippet` 重命名**（缺失显式空串）
- [x] 1.5 `workspace_symbols`：symbol_index 用 signature 降级、LSP 路径用 loader、ripgrep 路径已含 `snippet`；三路径统一含 `path`/`line`/`snippet`
- [x] 1.6 大量结果策略：`snippet_context_for_index` 前 `TOP_K_CONTEXT_SNIPPETS=50` 条带 ±5 行上下文，其余单行；读次上限外显式空串，path/line 恒保留
- [x] 1.7 前端：确认无 code-intel 专用渲染消费方（`lsp` 走通用 ToolCallCard；SearchPanel 的 snippet 是消息搜索无关），`snippet` 为新增字段 JSON 向后兼容 → 零前端改动
- [x] 1.8 预算：单条 600 char 上限 + top-K + 读次上限三重约束，整体仍受运行时 `DEFAULT_MAX_RESULT_SIZE_CHARS` 落盘兜底；path/line 不参与截断

## 2. 阶段2：错误恢复约定统一推广

- [x] 2.1 在 `xiaolin-core/tool.rs` 新增 `ToolResult::err_with_recovery(error_type, message, hint)` 并补文档与测试
- [x] 2.2 `shell_exec`（`runtimes/shell.rs` 与 `tools-fs/shell.rs`）失败路径迁移为结构化错误 + 恢复提示 — 经 `ToolRuntimeError::to_tool_result()` 在 dispatcher 统一转换；stub 已迁移
- [x] 2.3 `lsp` / code-intel 失败路径迁移为结构化错误 + 恢复提示 — 全部 `ToolResult::err` 已迁移为 `err_with_recovery` 或专用 helper（invalid_json / invalid_params / execution_failed / lsp_unavailable / parse_search_failed）
- [x] 2.4 `web_fetch` / `web_search`（`tools-network/lib.rs`）失败路径迁移
- [x] 2.5 `task_*`（`builtin_tools/task.rs`）与 `subagent.rs` 失败路径迁移
- [x] 2.6 对"重试无益"类失败（后端不可用/配置缺失/连接故障）补充反循环指令，与 `memory_search` 范式对齐
- [x] 2.7 校验软失败内嵌 `*_error` 字段也满足"下一步 + 反循环"约定

## 3. 阶段3：薄 prompt 工具补齐行为指导

- [x] 3.1 为 `file_outline` / `code_sections` 补 `prompt()`，明示"读大文件前先获取结构再定向 read_file"
- [x] 3.2 为 `lsp` / `go_to_definition` / `find_references` / `workspace_symbols` 补 `prompt()`（when/配合/反模式/参数交互）
- [x] 3.3 为 subagent 全家（`spawn_subagent`/`wait_agent`/`resume_subagent`/`send_message` 等）补 `prompt()`
- [x] 3.4 为 `skill` / `identity` 补 `prompt()`（或确认现有 `description()` 已达 prompt 质量）
- [x] 3.5 确认所有补齐工具的 `description()` 仍为简短 UI 文案，prompt 与 description 职责分离

## 4. 阶段4：批量文件读取

- [x] 4.1 新增 `read_files` 工具（入参 `paths: string[]`），逐文件复用 `read_file` 工作区校验/大小上限/行号/去重缓存
- [x] 4.2 每个文件内容清晰标注路径边界；部分文件失败给结构化错误，不影响其余成功文件
- [x] 4.3 多文件总量超 per-message 预算时走既有落盘+预览机制
- [x] 4.4 在 `builtin_tools/mod.rs` 注册新工具（遵循规则 #5 完整注册清单，含 search_hint/exposure 决策）
- [x] 4.5 同步前端类型（规则 #6）与必要的 UI 渲染

## 5. 阶段5：工具质量遥测（接线既有机制，可与阶段2 合并交付）

- [x] 5.1 先评估 `xiaolin_observe::record_tool_call` 死代码：清理或接线（确认无生产调用方）
- [x] 5.2 在 `tool_round.rs` 失败分支按 `ToolErrorType` 维度上报（接 `cost_store` / `MetricsCollector::record_error`，标签受枚举约束，低基数）
- [x] 5.3 **导出 `query_state` 既有的重复调用检测计数**为可观测指标，不新建并行检测器
- [x] 5.4 采样对比：实施前后每任务平均工具调用数、失败重试率、`error_type=Unknown` 占比

## 6. 回归与验证

- [ ] 6.1 `cargo clippy -- -D warnings` 零警告（规则 no-dead-code），新增 pub item 均有调用方
- [ ] 6.2 `cargo test` 覆盖 attach_snippet、err_with_recovery、read_files 部分失败、片段预算截断
- [ ] 6.3 用 Tauri MCP 通过真实 UI 回归（规则 e2e）：经统一 `lsp` 工具触发 goToDefinition/findReferences，确认返回含 `snippet` 且不再紧跟 read_file
- [ ] 6.4 更新 `docs/bugfix.md`（如修复过程中发现 Bug）与本 change 完成状态
