## ADDED Requirements

### Requirement: 位置类返回必须附带代码片段

统一 `lsp` 工具（`UnifiedLspTool`，及其内部委托的 go_to_definition / find_references / workspace_symbols 实现）返回的每一个代码位置，SHALL 附带该位置处的代码片段（默认前后各 5 行，行数上限可配置），使 agent 无需对结果立即追加 `read_file` 即可理解代码。

#### Scenario: LSP 路径返回定义并附带片段
- **WHEN** `lsp` 工具以 `goToDefinition` 经 LSP 引擎成功解析出定义位置 `{path, line, column}`
- **THEN** 返回的每个 symbol 对象 MUST 含 `snippet` 字段，内容为该 `path` 在 `line` 附近（±配置行数）的源码，且标注引擎为 `lsp`

#### Scenario: 入参文件内结果零额外 IO 切片
- **WHEN** 定义/引用位于**入参文件内**，且该文件内容已存于 `full_file.output`
- **THEN** 片段 MUST 从该已加载内容切出，不得为生成片段触发额外的文件 IO

#### Scenario: 跨文件结果按需读取且受读次数上限约束
- **WHEN** 定义/引用位于**入参文件以外**的文件
- **THEN** 系统 MAY 按需读取目标文件以生成片段，但单次工具调用的额外文件读取次数 MUST 有上限；超过上限的结果片段以空字符串降级，path/line 仍保留

#### Scenario: 引用列表逐条带片段
- **WHEN** `lsp` 以 `findReferences` 返回 N 条引用
- **THEN** 每条引用对象 MUST 含 `snippet`（该引用所在行，可含上下文），使 agent 能在不 read_file 的情况下判断每条引用的相关性

### Requirement: 三条引擎路径返回结构对齐

symbol_index、ripgrep、lsp 三条引擎路径 SHALL 返回结构一致的对象，至少共同包含 `path`、`line`、`snippet` 字段，使 agent 无需根据 `engine` 值做条件分支即可消费结果。

#### Scenario: ripgrep 回退字段对齐为 snippet
- **WHEN** `findReferences` 经 ripgrep 回退路径返回结果（当前字段为 `text`）
- **THEN** 该字段 MUST 重命名/对齐为 `snippet`，与其他路径一致

#### Scenario: symbol_index 路径片段降级策略
- **WHEN** symbol_index 路径命中（无现成文件内容）
- **THEN** 系统 MUST 以 `signature` 作为片段降级，或在读次数上限内按需读取文件补 `snippet`；二选一行为需在实现中确定且可测

#### Scenario: 片段缺失时显式标记而非静默省略
- **WHEN** 某条结果因文件不可读或超出读次数上限而无法生成片段
- **THEN** 该对象的 `snippet` 字段 MUST 显式为空字符串或 `null`，而非整字段缺省，以保持结构稳定

#### Scenario: 片段按字符边界安全切片
- **WHEN** 片段所在源码含多字节字符（中文/emoji）
- **THEN** 切片 MUST 按字符边界进行（遵循质量守则 #1），不得在非 char 边界 panic

### Requirement: 片段体积受预算约束

附带片段后的返回 SHALL 仍受 `DEFAULT_MAX_RESULT_SIZE_CHARS` 预算约束，片段行数与单条字符数 MUST 有上限，避免大量结果叠加片段后超限被落盘。

#### Scenario: find_references 大量结果的片段策略
- **WHEN** `findReferences` 返回接近上限（最多 2000 条）的结果
- **THEN** 系统 MUST 采用受控策略（仅前 K 条带片段 / 按文件聚合 / 单条片段限单行），保证整体不超预算，且 path/line 对所有结果均保留

#### Scenario: 大量结果时片段不撑爆预算
- **WHEN** 返回结果条数较多且每条都附带片段
- **THEN** 系统 MUST 在不超过结果大小预算的前提下保留片段（必要时按上限截断单条片段或减少上下文行数），并保证关键字段（path/line）不被截断
