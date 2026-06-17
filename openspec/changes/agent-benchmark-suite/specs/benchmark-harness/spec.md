## ADDED Requirements

### Requirement: BenchmarkRuntime execution wrapper
系统 SHALL 提供 `BenchmarkRuntime` 结构体，封装 `AgentRuntime::execute_unified_with_cost_store` 的调用，自动处理临时目录、mock 依赖注入、AgentStep 流订阅。

#### Scenario: Execute benchmark task with ScriptedProvider
- **WHEN** 使用 scripted 模式执行 benchmark 任务
- **THEN** BenchmarkRuntime SHALL 加载 fixture 响应序列到 ScriptedProvider
- **AND** 创建 workspace 临时目录并复制 fixture 文件
- **AND** 调用 AgentRuntime 并收集完整 AgentStep 流

#### Scenario: Execute benchmark task with real LLM
- **WHEN** 使用 live 模式执行 benchmark 任务
- **THEN** BenchmarkRuntime SHALL 使用配置的 LLM provider
- **AND** 遵守 max_cost_usd 和 timeout_ms 限制

### Requirement: Grader trait and built-in graders
系统 SHALL 定义 `Grader` trait 并提供至少 5 种内建 grader。

#### Scenario: OutputContains grader passes
- **WHEN** assistant 最终输出包含所有指定 pattern
- **THEN** OutputContains grader SHALL 返回 pass

#### Scenario: ToolTrace grader detects wrong tool
- **WHEN** agent 调用了 must_not_include 中的工具
- **THEN** ToolTrace grader SHALL 返回 fail 并报告违规工具名

#### Scenario: TokenBudget grader enforces limit
- **WHEN** 任务的 total_tokens 超过 threshold
- **THEN** TokenBudget grader SHALL 返回 fail

#### Scenario: Multiple graders compose
- **WHEN** 任务配置了多个 graders
- **THEN** 仅当所有 grader 都 pass 时任务 SHALL 判定为 pass

### Requirement: Metrics collector from AgentStep stream
系统 SHALL 从 `AgentStep` 流中采集标准 metrics，包含 token usage、tool calls、duration、context pressure。

#### Scenario: Collect token usage
- **WHEN** AgentStep::TurnEnd 事件到达
- **THEN** metrics collector SHALL 记录 prompt_tokens、completion_tokens、total_tokens、cached_input_tokens

#### Scenario: Collect tool call trace
- **WHEN** AgentStep::ToolResult 事件到达
- **THEN** metrics collector SHALL 记录 tool_name、success、duration

### Requirement: JSONL run report
每次 benchmark run SHALL 输出 JSONL 格式报告，每个任务一行，包含 task_id、pass/fail、grader 详情、metrics。

#### Scenario: Generate run report
- **WHEN** benchmark suite 执行完成
- **THEN** 系统 SHALL 生成 `results.jsonl` 文件
- **AND** 每行包含 task_id、pass、graders 结果数组、metrics 对象

#### Scenario: CLI summary output
- **WHEN** benchmark run 完成
- **THEN** CLI SHALL 打印 pass/fail 摘要表和 metrics 统计
