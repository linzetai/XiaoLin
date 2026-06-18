## 1. Benchmark Crate 基础搭建

- [x] 1.1 创建 `crates/xiaolin-benchmark/` crate，添加 Cargo.toml 依赖（xiaolin-agent、xiaolin-core、xiaolin-protocol、serde_yaml、tokio）
- [x] 1.2 定义 `BenchmarkTask` YAML 反序列化结构体（id、prompt、graders、metrics、environment）
- [x] 1.3 实现 YAML 加载和验证逻辑（缺必填字段报错）
- [x] 1.4 `cargo check` + `cargo clippy -- -D warnings` 通过

## 2. BenchmarkRuntime 执行层

- [x] 2.1 从 `xiaolin-gateway/tests/e2e_scenarios.rs` 提取 `ScriptedProvider` 为 `xiaolin-benchmark/src/providers/scripted.rs`
- [x] 2.2 实现 `BenchmarkRuntime` 结构体，封装 `AgentRuntime::execute_unified` 调用（临时目录、mock 注入、AgentStep 流订阅）
- [x] 2.3 实现 scripted 模式：从 fixture 加载响应序列
- [x] 2.4 实现 live 模式：使用配置的 LLM provider，增加 max_cost_usd guard
- [x] 2.5 `cargo check` + `cargo clippy -- -D warnings` 通过

## 3. Grader 体系

- [x] 3.1 定义 `Grader` trait（input: BenchmarkRunResult → output: GradeResult { pass, reason }）
- [x] 3.2 实现 `OutputContains` / `OutputNotContains` grader（正则匹配 assistant 输出）
- [x] 3.3 实现 `ToolTrace` grader（must_include / must_not_include / order 验证）
- [x] 3.4 实现 `TokenBudget` / `TurnLimit` grader
- [x] 3.5 实现 `FilesystemCheck` grader（文件存在/内容/不变性验证）
- [x] 3.6 实现 multi-grader 组合逻辑（全 pass 才算 pass）
- [x] 3.7 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 4. Metrics Collector

- [x] 4.1 实现 `MetricsCollector` 结构体，从 `AgentStep` 流采集：token_usage、tool_calls、duration、context_pressure
- [x] 4.2 实现 `BenchmarkRunMetrics` 聚合结构（含 tool_success_rate、per_tool 统计）
- [x] 4.3 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 5. 报告生成

- [x] 5.1 实现 JSONL run report 输出（task_id、pass、graders、metrics per task）
- [x] 5.2 实现 CLI 摘要打印（pass/fail 表 + metrics 统计）
- [x] 5.3 `cargo check` + `cargo clippy -- -D warnings` 通过

## 6. 初始 Benchmark 任务

- [x] 6.1 创建 `benchmarks/tasks/tool-routing/` 目录，编写 2 个任务 YAML（read-file-not-shell、search-not-grep）
- [x] 6.2 创建 `benchmarks/tasks/context-efficiency/` 目录，编写 2 个任务 YAML（simple-task-budget、multi-step-efficiency）
- [x] 6.3 创建 `benchmarks/tasks/error-recovery/` 目录，编写 1 个任务 YAML（stale-file-recovery）
- [x] 6.4 为每个 scripted 模式任务创建 `benchmarks/fixtures/` 下的 ScriptedProvider 响应序列和 workspace fixture
- [x] 6.5 `cargo test` 验证所有任务 YAML 可解析

## 7. 集成验证

- [x] 7.1 实现 `cargo bench --bench agent_benchmark` 入口，加载所有任务并执行
- [x] 7.2 在 scripted 模式下跑完所有初始任务，确认 pass/fail 符合预期
- [x] 7.3 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过
