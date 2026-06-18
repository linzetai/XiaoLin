## Why

XiaoLin agent 缺乏量化评估机制，无法回答「当前能力几分」和「改动后提升了多少」。`agent-capability-boost` 提出了多项优化（shell prompt、MCP defer、API error 防护等），但没有基准线就无法验证效果。已有的 `TurnSummary`、`CostStore`、`RuntimeObserver` 提供了丰富的 metrics 数据，`ScriptedProvider` 和 `AgentRuntime::execute_unified` 提供了 programmatic 入口，具备建设 benchmark 的基础。

## What Changes

- **新增 `xiaolin-benchmark` crate**：独立的 benchmark harness，不依赖 UI/WebSocket，直连 `AgentRuntime`
- **定义 YAML 任务格式**：每个 benchmark 任务用 YAML 描述 prompt、graders、metrics thresholds
- **实现三类 Grader**：output_contains（输出断言）、tool_trace（工具使用追踪）、filesystem_check（文件系统验证）
- **实现 Metrics Collector**：从 `AgentStep` 流采集 token、工具调用、耗时等指标，输出 JSONL 报告
- **提供 5-10 个初始 benchmark 任务**：覆盖 tool-routing、context-efficiency、error-recovery 等维度
- **支持两种模式**：ScriptedProvider（确定性回归，CI 可用）和 Real LLM（能力评估，手动触发）

## Capabilities

### New Capabilities
- `benchmark-harness`: Agent benchmark 执行框架，含 harness、metrics collector、graders、report generator
- `benchmark-scenarios`: 标准化 benchmark 任务定义与初始任务集

### Modified Capabilities

## Impact

- **新增 crate**: `crates/xiaolin-benchmark/`
- **复用**: `ScriptedProvider` 从 `xiaolin-gateway/tests/e2e_scenarios.rs` 提取为共享
- **复用**: `AgentRuntime::execute_unified_with_cost_store` 作为执行入口
- **复用**: `TurnSummary`、`AgentStep`、`RuntimeObserver` 作为 metrics 数据源
- **新增目录**: `benchmarks/tasks/` 存放 YAML 任务定义
- **新增目录**: `benchmarks/fixtures/` 存放 ScriptedProvider 响应和 workspace 模板
