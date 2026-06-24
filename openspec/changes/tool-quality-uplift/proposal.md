## Why

XiaoLin 的工具底座（分层暴露、tool_search、大输出落盘、流式并行）和"优质层"工具（filesystem、search_in_files、web_search、memory）已达一流水准，部分工具甚至自带反循环错误引导。但通过对约 60 个 LLM 可调用工具的逐项审计发现：**这套已被验证的优秀范式没有被贯彻到所有工具**，导致 agent 被迫产生大量本可避免的追加/重试调用，既费 token 又拖慢任务。核心问题集中在三处可量化的"漏点"，修复成本低（多为对齐已有范本）、ROI 高。

## What Changes

### ① code-intel 返回片段化（最高 ROI）
- LLM 实际调用的是统一 **`lsp` 工具**（`UnifiedLspTool`，委托 go_to_definition/find_references/workspace_symbols 等内部实现）；其 **LSP 引擎路径只返回 `{path, line, column}`**，无代码片段，逼出"跳转→read_file"的第二次调用
- 成本分两类：**入参文件内的结果**可直接从已加载的 `full_file.output` 切片（零额外 IO）；**跨文件结果**（跳转到其他文件，常态）必须按需读取目标文件，故需**按 path 缓存 + 读次数上限**控制开销
- 统一约定：**凡返回代码位置的结果，必须附带该位置的代码片段**；三条引擎路径返回结构对齐（`find_references` 的 ripgrep 回退当前字段为 `text`，需重命名/对齐为 `snippet`；symbol_index 路径以 `signature` 作降级或按需读文件补片段）
- `find_references` 上限 2000 条 × 片段可能超预算，需 **top-K 带片段 / 按文件聚合 / 单条片段上限**策略

### ② 错误恢复引导统一推广
- 当前仅 `filesystem.rs` 使用 `typed_err` + `recovery_hint`；`memory` 用另一套"软失败内嵌 What-to-do-next + 反循环"范式；二者互不统一，且 **shell / lsp / network / task / subagent 一律用泛化 `ToolResult::err(String)`（`error_type=Unknown`）**
- 定义统一错误约定：失败结果必须携带 ① 结构化 `ToolErrorType` 或软失败标记，② "下一步怎么做"的恢复提示，③ 对可能死循环的场景加反循环指令；推广到 shell/lsp/network/task

### ③ 薄 prompt 工具补齐行为指导
- 全仓库仅约 14/60 工具重写 `prompt()`；code-intel（lsp/outline/sections）、subagent 全家、skill/identity 等只有简短 `description()`
- 为这些工具补齐 `prompt()`：何时使用、与其他工具的配合（如"读大文件前先 file_outline"）、反模式、参数交互

### ④ 批量文件读取入口
- `read_file` 一次只读一个文件；理解一个模块常需 3-5 次调用。新增批量读取（`read_files` 或 `read_file` 接受路径数组），把 N 次往返压成 1 次

### ⑤ 工具质量遥测（接线既有机制，非新建）
- 现状澄清：`xiaolin_observe::record_tool_call` **是死代码（无调用方）**；真实遥测已存在于 `cost_store`（SQLite 日聚合 success/failure/duration）、`query_state`（**已做同工具同参数重复检测 + Warn/ForceStop**）、`runtime/observer`、`MetricsCollector::record_error`
- 本能力**扩展而非重建**：在失败分支补按 `ToolErrorType` 的维度（接到现有 dispatch/tool_round 钩子），并**导出 `query_state` 既有的重复计数**为可观测指标，用于**量化优化前后的调用次数下降**与 `error_type=Unknown` 占比

## Capabilities

### New Capabilities
- `code-intel-snippet-context`: 统一 `lsp` 工具（及其内部 go_to_definition/find_references/workspace_symbols 实现）的位置类返回统一附带代码片段，消除"跳转后再 read_file"的强制追加调用；三条引擎路径返回结构对齐，跨文件片段带缓存与读次数上限
- `tool-error-recovery-convention`: 跨工具的失败返回统一约定——结构化错误类型或软失败标记 + "下一步怎么做"恢复提示 + 反循环指令，并推广到 shell/lsp/network/task
- `tool-prompt-enrichment`: 薄描述工具补齐 `prompt()` 行为指导（when-to-use / 工具配合 / 反模式 / 参数交互），提升首选正确率、减少弯路
- `batch-file-read`: 批量文件读取入口，单次调用读取多个文件，结构性减少读文件往返
- `tool-quality-telemetry`: 在既有遥测（`cost_store` / `query_state` / `tool_round`）上扩展错误类型维度并导出重复调用指标，支撑优先级决策与调用次数下降的量化验证（非新建平行体系）

### Modified Capabilities
<!-- 本 change 主要新增工具返回/错误/提示约定，不改变现有 spec 的需求级行为；故无修改能力 -->

## Impact

- **后端 crates/xiaolin-tools-code**：
  - `code_intel.rs`：`go_to_definition` / `find_references` / `workspace_symbols` 三工具的 LSP 路径返回结构补 `snippet`；新增共享 `attach_snippet(path, line, ±ctx)` helper（复用已在内存的 `full_file.output`，零额外 IO）；补 `prompt()`
- **后端 crates/xiaolin-core**：
  - `tool.rs`：可能新增错误恢复约定的辅助构造（如 `ToolResult::err_with_recovery(error_type, msg, hint)`），统一两套错误范式
- **后端 crates/xiaolin-tools-fs / xiaolin-tools-network / xiaolin-agent**：
  - shell（`runtimes/shell.rs` / `shell.rs`）、network（`lib.rs`）、task（`builtin_tools/task.rs`）、subagent（`subagent.rs`）的错误路径替换泛化 `err()` 为带 recovery 的结构化错误
  - 新增批量读取工具（`xiaolin-tools-fs/src/filesystem.rs`）并在 `builtin_tools/mod.rs` 注册（遵循规则 #5 完整注册清单）
- **遥测扩展点（既有机制，非 observe 死代码）**：
  - `crates/xiaolin-session/src/cost_store.rs`（日聚合）、`crates/xiaolin-agent/src/runtime/query_state.rs`（重复检测+ForceStop）、`runtime/tool_round.rs`（失败分支钩子）、`crates/xiaolin-observe/src/metrics_collector.rs`（`record_error`）——在失败路径补 `error_type` 维度并导出 query_state 重复计数；先评估是否清理 `observe::record_tool_call` 死代码
- **遵循质量守则**：返回字段变更需同步前端 3 层类型（规则 #6）——`go_to_definition`/`find_references` 结果新增 `snippet` 字段，确认前端 code-intel 渲染消费方
- **关联**：与现有 `tool-exposure` / `deferred-pipeline` / `search-enhancement` 能力正交互补；与进行中的 `subagent-optimization` 无冲突（后者聚焦 subagent 缓存/UI，本 change 聚焦通用工具返回质量）
- **风险**：
  - 返回新增 `snippet` 会增加单次返回体积——需确认仍在 `DEFAULT_MAX_RESULT_SIZE_CHARS` 预算内，且片段行数（建议 ±5）可配置上限；`find_references` 大量结果需 top-K/聚合策略
  - 补齐 `prompt()` 会增大 tool schema 体积，可能影响 eager tool 列表的 token 数与 prompt-cache 命中——需与 `prompt-cache-maximize-hits` change 交叉评估（仅影响 tool schema，不影响 tool result）
  - 跨文件 snippet 引入额外文件 IO——需读次数上限兜底
