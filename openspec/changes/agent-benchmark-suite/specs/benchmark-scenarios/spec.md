## ADDED Requirements

### Requirement: YAML task definition format
Benchmark 任务 SHALL 使用 YAML 格式定义，包含 id、prompt、graders、metrics thresholds、environment overrides。

#### Scenario: Parse valid task YAML
- **WHEN** 读取 `benchmarks/tasks/{suite}/{task}.yaml`
- **THEN** 系统 SHALL 解析为 `BenchmarkTask` 结构体
- **AND** 包含 id、prompt、graders 列表、metrics 配置

#### Scenario: Reject invalid task YAML
- **WHEN** YAML 缺少必填字段（id 或 prompt）
- **THEN** 系统 SHALL 返回解析错误并指明缺失字段

### Requirement: Tool-routing benchmark scenarios
系统 SHALL 包含至少 2 个 tool-routing 场景，验证 agent 在应使用内置工具时不误用 shell_exec。

#### Scenario: Read file via read_file not shell
- **WHEN** prompt 要求读取文件内容
- **THEN** agent SHALL 使用 read_file 工具
- **AND** ToolTrace grader 验证 must_not_include shell_exec

#### Scenario: Search files via search_in_files not grep
- **WHEN** prompt 要求在文件中搜索关键词
- **THEN** agent SHALL 使用 search_in_files 工具
- **AND** ToolTrace grader 验证 must_not_include shell_exec

### Requirement: Context-efficiency benchmark scenarios
系统 SHALL 包含至少 2 个 context-efficiency 场景，验证 token 使用效率。

#### Scenario: Simple task within token budget
- **WHEN** 执行简单任务（如添加注释）
- **THEN** total_tokens SHALL 不超过配置的 threshold
- **AND** turn 数 SHALL 不超过 3

#### Scenario: Complex task token efficiency
- **WHEN** 执行多步任务（如实现新功能）
- **THEN** token 消耗 SHALL 在合理范围内（per-task threshold）

### Requirement: Error-recovery benchmark scenarios
系统 SHALL 包含至少 1 个 error-recovery 场景，验证 agent 在工具失败后的恢复能力。

#### Scenario: Recover from stale file error
- **WHEN** edit_file 因 stale detection 失败
- **THEN** agent SHALL re-read 文件并重试编辑
- **AND** 最终成功完成任务
