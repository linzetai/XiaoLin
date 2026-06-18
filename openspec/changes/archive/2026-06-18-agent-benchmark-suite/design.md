## Context

XiaoLin 已有丰富的 metrics 基础设施但缺乏统一的 benchmark harness。Claude Code 和 Codex 都没有 in-repo benchmark suite，分别靠外部 harness + SDK metrics 和分层 mock 测试栈。XiaoLin 的 `AgentRuntime::execute_unified` 提供了不经 UI/WS 的 programmatic 入口，`ScriptedProvider` 提供了确定性 mock LLM，这两者是 benchmark 的核心支撑。

## Goals / Non-Goals

**Goals:**
- 建立可量化、可重复的 agent 能力评估机制
- 用 YAML 定义任务，支持 CI 自动化回归
- 在 `agent-capability-boost` 实施前后跑同一任务集，量化改进
- Metrics 兼容已有 `TurnSummary`，不重复造轮子

**Non-Goals:**
- 不建 SWE-bench 级别的外部评测平台（仅 env 标签预留接口）
- 不替换现有 `tests/e2e/` Tauri MCP 测试
- 不做 benchmark 结果的可视化仪表盘（初期 JSONL + CLI 报告）
- 不做多模型对比（单一配置跑同一任务集）

## Decisions

### D1: 执行层 — 直连 AgentRuntime，不经 Gateway

**选择**：benchmark harness 直接构造 `AgentRuntime` + mock 依赖，调用 `execute_unified_with_cost_store`。

**理由**：
- 最快路径，不需启动 HTTP/WS server
- 完整 `AgentStep` 流可订阅，无信息损失
- CI 友好（纯 Rust test）

**替代**：经 Gateway HTTP API → 多了网络层和 session 管理开销，benchmark 不需要测这些。

### D2: 任务格式 — YAML spec + workspace fixture

**选择**：每个任务用 YAML 定义，包含 prompt、graders、metrics thresholds。workspace fixture 是一个目录树，benchmark 运行时复制到临时目录。

**理由**：
- 人类可读、易编辑
- 可扩展（新增 grader 类型不改格式）
- workspace fixture 复用已有 temp dir 基础设施

### D3: Grader 架构 — trait + 多 grader 组合

**选择**：定义 `Grader` trait，每个任务可配多个 grader，全部 pass 才算 pass。

**内建 grader**：
- `OutputContains` / `OutputNotContains`：正则匹配 assistant 最终输出
- `ToolTrace`：检查工具调用序列（must_include / must_not_include / order）
- `FilesystemCheck`：验证文件是否被创建/修改/保持不变
- `TokenBudget`：token 消耗不超阈值
- `TurnLimit`：turn 数不超阈值

### D4: 两种运行模式

| 模式 | LLM | 用途 | 确定性 | CI |
|------|-----|------|--------|-----|
| `scripted` | ScriptedProvider（mock 响应序列） | 行为回归 | ✅ 确定 | ✅ |
| `live` | 真实 API（按 config 配置） | 能力评估 | ❌ 不确定 | ❌ |

Scripted 模式的响应存放在 `benchmarks/fixtures/{task_id}/responses.json`。
Live 模式需要配置 API key，默认不在 CI 跑。

### D5: 报告格式 — JSONL + CLI 摘要

每次 benchmark run 输出：
- `benchmarks/runs/{run_id}/results.jsonl`：每个任务一行 JSON
- CLI 打印：pass/fail 摘要 + metrics 对比表

## Risks / Trade-offs

| 风险 | 影响 | 缓解 |
|------|------|------|
| ScriptedProvider 与真实 LLM 行为偏差 | 回归测试可能漏报 | Scripted 仅保证行为不退化，能力评估用 live 模式 |
| Benchmark 任务设计偏差 | 评分不反映真实能力 | 从实际用户任务中提取，覆盖多维度 |
| `execute_unified` 参数多且不稳定 | 维护成本 | 封装 `BenchmarkRuntime` 适配层 |
| Live 模式费用 | API 成本 | 设 max_cost_usd 限制，CI 不跑 live |
