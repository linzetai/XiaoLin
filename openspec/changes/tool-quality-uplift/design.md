## Context

对约 60 个 LLM 可调用工具的逐项审计（暴露层级 / `prompt()` 覆盖 / schema `enum` / `typed_err` / `search_hint` / 成功返回质量）得出：

- **底座成熟**：分层暴露 + tool_search(BM25)、大输出落盘+预览、per-message 预算、流式并行执行。
- **"优质层"已一流**：filesystem 全家、`search_in_files`（`context_lines`+`semantic_context`）、`web_search`（snippet+契约）、`memory_search`（完整三元组 + 软失败内嵌反循环引导）。
- **三处集中漏点**：
  1. code-intel 的 **LSP 引擎路径只返回 `{path,line,column}`**，无片段——`go_to_definition`/`find_references` 甚至已把整文件读入 `full_file.output` 却丢弃。
  2. **错误恢复引导不统一**：仅 filesystem 用 `typed_err`+`recovery_hint`，memory 用软失败内嵌，shell/lsp/network/task 一律泛化 `err()`（`Unknown`）。
  3. **薄 prompt**：仅约 14/60 工具重写 `prompt()`。

约束：不破坏现有 prompt-cache 策略（返回内容属 tool result，不进 system prompt，无缓存污染风险）；返回字段变更须遵循质量守则 #6（同步前端 3 层类型）。

## Goals / Non-Goals

**Goals:**
- 把已验证的"优质层"范式系统性补齐到落后工具，**减少可避免的追加/重试调用**，同时提升完成质量。
- 优先实施"数据已在手、近零成本"的改动（code-intel 片段化）。
- 建立可量化收益的遥测，闭环验证"又省又好"。

**Non-Goals:**
- 不重写工具底座（暴露分层、落盘、预算机制保持不变）。
- 不改 schema 的 `enum` 覆盖（审计显示已基本覆盖，零散补缺归入实现细节，不单列能力）。
- 不涉及 subagent 缓存/UI（由 `subagent-optimization` 负责）。

## Decisions

### D1：复用 `search_in_files` 的 context 切片逻辑补片段，统一三路径
- **选择**：优先**复用/提取** `search_in_files` 已有的 ±context_lines 切片逻辑（`filesystem.rs` L4170+）为共享模块，而非从零写 `attach_snippet`；`lsp` 工具内部 symbol_index/ripgrep/lsp 三路径统一调用它填充 `snippet`。
- **关键事实修正**：LLM 调用的是统一 **`lsp`** 工具（三个 struct 未单独注册）；`find_references` 的 ripgrep 回退当前返回字段是 **`text`** 而非 `snippet`，需对齐重命名。
- **IO 成本分级**：仅"**入参文件内**的 LSP 结果"可从已持有的 `full_file.output` 零 IO 切片；**跨文件结果是常态**，必须按需读取目标文件，需 **按 path 缓存的 snippet 加载器 + 每次调用读文件次数上限**，并复用 `read_file` 大小上限。symbol_index 路径无现成文件内容，以 `signature` 作降级或按需读文件。
- **备选**：让前端补片段——否决，前端无源码访问语义，且无法服务非 UI 消费方（subagent、工具链）。

### D2：统一错误恢复构造入口，收敛两套范式
- **选择**：在 `xiaolin-core/tool.rs` 增加 `ToolResult::err_with_recovery(error_type, message, hint)`（硬失败）；软失败仍走结果内嵌 `*_error` 字段，但其内容约定与硬失败 hint 同构（均含"下一步 + 可选反循环"）。
- **理由**：filesystem 与 memory 两套范式都好但不一致；统一入口降低各工具迁移成本，且让前端与遥测能统一消费 `error_type`。
- **备选**：强制所有工具改用硬 `typed_err`——否决，memory 的"部分成功 + 部分软失败"语义无法用单一硬失败表达。

### D3：`prompt()` 补齐采用"模板化要点"，并冻结范围
- **选择**：为薄工具补 `prompt()` 时统一覆盖四要点（when-to-use / 工具配合 / 反模式 / 参数交互），对齐 filesystem/web_search 现有风格。**冻结清单**：优先 `lsp`/`file_outline`/`code_sections`/subagent 全家/`skill`/`identity`，**排除已有 rich prompt 的工具**（shell_exec、web_search、read_file 等），不追求"全部 60 个"。
- **理由**：一致结构降低模型理解成本，便于审计覆盖率；冻结范围避免维护与 token 成本失控。
- **prompt-cache 影响**：补 `prompt()` 增大 tool schema 体积，影响 eager tool 列表 token 数与前缀缓存——需与 `prompt-cache-maximize-hits` change 交叉评估（仅 tool schema，不涉及 tool result）。

### D4：批量读取作为新工具 `read_files`，而非重载 `read_file`
- **选择**：新增独立 `read_files`（入参 `paths: string[]`），逐文件复用 `read_file` 校验/上限/去重，部分失败不影响其余，总量超预算走落盘。
- **理由**：保持 `read_file` 单文件契约稳定（避免影响其大量既有调用与缓存键）；新工具语义清晰。
- **备选**：让 `read_file.file_path` 接受 string|array——否决，会复杂化 schema 与去重缓存键，且破坏单文件返回形状。

### D5：接线既有遥测，不重建检测器
- **关键事实修正**：`xiaolin_observe::record_tool_call` 是**死代码（无调用方）**。真实遥测已存在：`cost_store`（SQLite 日聚合 success/failure/duration）、`query_state`（**已实现同工具同参数重复检测 + Warn/ForceStop**）、`runtime/observer`、`MetricsCollector::record_error`。
- **选择**：在失败分支（`tool_round.rs` 失败钩子）补 `error_type` 维度计数（标签受 `ToolErrorType` 枚举约束，低基数）；"连续重试"指标**导出 `query_state` 既有计数**，不重写检测逻辑。先评估清理 observe 死代码。
- **理由**：避免与 runtime 既有重复检测体系并行；低基数标签防 metrics 爆炸；足以计算每任务平均调用数、重试率、`error_type=Unknown` 占比的前后对比。
- **备选**：新建独立遥测体系——否决，与 `query_state`/`cost_store` 重复且割裂。

## Risks / Trade-offs

- **返回体积增大（片段）** → 片段行数默认 ±5 且设单条字符上限，整体仍受 `DEFAULT_MAX_RESULT_SIZE_CHARS` 约束；大量结果时按上限截断片段而非省略 path/line。
- **前端类型不同步导致编译错误（规则 #6）** → 实施 code-intel 片段时同步更新 `transport.ts`/`api.ts`/code-intel 渲染消费方；新增 `snippet` 为可选字段，向后兼容。
- **跨文件引用补片段引入额外 IO** → 仅对"片段文件 ≠ 已加载文件"的引用按需读取，并复用 read_file 上限；可设每次批量片段的读文件次数上限。
- **错误约定推广面广，易遗漏** → 以"统一构造入口 + 遥测 unknown 占比"双手段兜底：迁移后 `error_type=Unknown` 的占比应显著下降，作为完成度指标。
- **批量读取被滥用读超大文件集** → 复用单文件大小上限 + 聚合预算落盘，且文档说明用于"少量相关文件"。

## Migration Plan

分阶段、可独立交付（每阶段单独可验证、可回滚）：
1. **阶段1（最高 ROI）**：`attach_snippet` + code-intel 三工具三路径片段化（含前端 3 层类型同步）。
2. **阶段2**：`err_with_recovery` 入口 + shell/lsp/network/task 错误路径迁移。
3. **阶段3**：薄工具 `prompt()` 补齐。
4. **阶段4**：`read_files` 新工具 + 注册清单（规则 #5）。
5. **阶段5**：遥测增量，采样对比。

回滚：每阶段独立；片段字段为可选，回滚仅需停止填充；新工具回滚仅需取消注册。

## Telemetry Baseline（阶段5采样对比）

实施 tool-quality-uplift 前后，用以下指标验证「又省又好」目标，无需专用采样脚本——从进程内结构化指标或 Prometheus 端点读取即可。

### 对比指标

| 指标 | 来源 | 计算方式 |
|------|------|----------|
| 每任务工具调用数 | `QueryLoopState::total_tool_calls`（运行时）/ `xiaolin_tool_calls_total`（Prometheus） | 任务结束时 `total_tool_calls`，或 `sum(xiaolin_tool_calls_total)` 增量 ÷ 任务数 |
| 失败率 | `xiaolin_tool_calls_total{success="false"}` ÷ 全部 tool_calls | Prometheus `rate` 或结构化 counter 比值 |
| `error_type=unknown` 占比 | `xiaolin_tool_failures_total{error_type="unknown"}` ÷ 全部 failures | 阶段2 迁移后应显著下降 |
| 重复调用检测 | `xiaolin_tool_repetitions_total{action="warn\|force_stop"}` | 优化后 warn/force_stop 计数应下降 |

### 读取方式

1. **结构化指标（推荐）**：`xiaolin_observe::render_structured_metrics_prometheus()` 或 gateway `/api/v1/metrics`，查找：
   - `xiaolin_tool_failures_total{tool="...",error_type="..."}`
   - `xiaolin_tool_repetitions_total{action="warn|force_stop",tool="..."}`
2. **全局 Prometheus recorder**：`xiaolin_observe::render_metrics()`（需 `init_observability` 后），查找 `xiaolin_tool_calls_total{tool="...",success="true|false"}`。
3. **SQLite 日聚合**：`cost_store` 表按日 success/failure/duration（无 error_type 维度，作辅助对照）。

### 采样建议

- 基线：change 落地前采集 1–3 天 `/metrics` 快照或导出结构化文本。
- 对照：各阶段合并后同样窗口复采，对比上表四项比率的相对变化。
- 单任务 debug：`QueryLoopState::repetition_stats()` 返回 `(warn_count, force_stop_count)`。

## Open Questions

- 片段默认上下文行数（±5？）与单条字符上限的具体取值，是否需做成 `BehaviorConfig` 可配置项？
- 批量读取入口最终形态：独立 `read_files` 是否需要也注册为 deferred（经 tool_search 暴露）以控制 eager 列表体积？
- "连续重试"阈值定义（同工具同参数 N 次 / 时间窗口）需结合实际遥测样本标定。
