## ADDED Requirements

### Requirement: 失败返回必须携带恢复指导

任何工具的失败返回 SHALL 至少包含 ① 结构化错误分类（`ToolErrorType`）或明确的软失败标记，以及 ② 面向 agent 的"下一步怎么做"恢复提示。禁止返回不含任何恢复信息的泛化错误字符串（`error_type=Unknown` 且无 hint）。

#### Scenario: shell 执行失败返回结构化错误与恢复提示
- **WHEN** `shell_exec` 因命令不存在、非零退出或权限问题失败
- **THEN** 返回的 `ToolResult` MUST 携带具体 `ToolErrorType`（而非默认 `Unknown`），且 `output` MUST 含可操作的下一步建议（如"检查命令是否安装/修正路径/换用 X 工具"）

#### Scenario: lsp/network/task 失败对齐约定
- **WHEN** `lsp`、`web_fetch`、`web_search`、`task_*` 任一工具失败
- **THEN** 返回 MUST 满足"结构化错误分类 + 恢复提示"约定，不得使用裸 `ToolResult::err(message)` 且 `error_type` 保持 `Unknown`

### Requirement: 可能死循环的场景必须含反循环指令

当失败原因属于"重试无益"类（后端不可用、配置缺失、连接故障、解析持续失败）时，恢复提示 SHALL 明确告知 agent 停止重试并上报，而非诱导其反复调用同一工具。

#### Scenario: 后端不可用时引导停止而非重试
- **WHEN** 某工具因依赖后端（数据库、LSP server、外部 API）不可用而失败
- **THEN** 恢复提示 MUST 包含"停止循环并上报/改用替代方案"的明确措辞，而非仅"请重试"

#### Scenario: 与 memory 软失败范式一致
- **WHEN** 工具选择以软失败（成功结果内嵌 `*_error` 字段）方式返回部分失败
- **THEN** 该 `*_error` 字段 MUST 同样包含"下一步怎么做"与适用的反循环指令，与 `memory_search` 现有范式一致

### Requirement: 提供统一的错误恢复构造入口

系统 SHALL 提供统一的辅助构造（如 `ToolResult::err_with_recovery(error_type, message, hint)` 或等价物），使两套既有范式（filesystem 的 `typed_err`+`recovery_hint` 与 memory 的软失败内嵌）收敛为一致的 API，降低各工具实现成本与不一致风险。

#### Scenario: 新工具用统一入口构造失败
- **WHEN** 开发者为新工具或既有工具的失败路径构造返回
- **THEN** 存在一个文档化的统一构造入口可用，且其产出在 `error_type` 与恢复提示上结构一致、可被前端与遥测统一消费
