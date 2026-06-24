## ADDED Requirements

### Requirement: 在既有遥测上扩展错误类型维度

工具调用遥测 SHALL 在**既有遥测机制**（`cost_store` 日聚合、`tool_round` 失败钩子、`MetricsCollector::record_error`）之上扩展失败调用的 `ToolErrorType` 维度，使运营方能区分失败原因分布（如参数错误 vs 文件未找到 vs 后端不可用），用于定位需优先改进的工具。注意 `xiaolin_observe::record_tool_call` 当前为无调用方的死代码，本能力 MUST NOT 以其为基线，实施时应评估清理或接线。

#### Scenario: 失败时带错误类型上报
- **WHEN** 任一工具返回失败结果
- **THEN** 遥测 MUST 在失败处理路径（如 `tool_round` 失败分支）记录 `{tool_name, error_type}` 维度的计数，错误类型来自统一的 `ToolErrorType`

#### Scenario: 错误类型标签受控
- **WHEN** 上报错误类型标签
- **THEN** 标签集 MUST 来自有限的 `ToolErrorType` 枚举（避免高基数标签），未分类时归入 `unknown`

### Requirement: 导出既有重复调用检测为可观测指标

`query_state` 已实现"同工具同参数重复调用检测 + Warn/ForceStop"。本能力 SHALL **导出**该既有检测结果为可观测指标，而非重写检测逻辑，作为衡量优化前后调用次数下降的关键指标。

#### Scenario: 复用 query_state 的重复检测
- **WHEN** `query_state` 在同一任务内检测到对同一工具的重复/连续重试
- **THEN** 系统 MUST 将该既有计数导出为可观测指标（计数或事件），供前后对比"省了多少调用"，且 MUST NOT 新建并行的重复检测器

#### Scenario: 优化收益可量化
- **WHEN** 本 change 的其余能力落地前后分别采样
- **THEN** 遥测数据 MUST 足以计算每任务平均工具调用次数与失败重试率的变化，用于验证"又省又好"目标是否达成
