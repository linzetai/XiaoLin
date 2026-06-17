## ADDED Requirements

### Requirement: shell_exec prompt method
`ShellExecTool`（或等效 shell 工具）SHALL 实现 `prompt()` 方法，返回 ~2000 字的详细行为引导文本，涵盖工具路由、git 操作、并行命令、反模式四个核心领域。

#### Scenario: Tool routing rules present
- **WHEN** LLM 收到 shell_exec 的 prompt
- **THEN** prompt 中 SHALL 包含工具路由规则，明确列出：搜索用 search_in_files 不用 grep/rg、读文件用 read_file 不用 cat/head/tail、编辑用 edit_file 不用 sed/awk

#### Scenario: Git operation guidance present
- **WHEN** LLM 使用 shell_exec 执行 git 命令
- **THEN** prompt 中 SHALL 包含 git 操作规范，包括：commit message 格式、禁止 force push main、amend 规则

#### Scenario: Anti-pattern warnings present
- **WHEN** LLM 考虑使用 shell_exec
- **THEN** prompt 中 SHALL 列出反模式清单：禁止 sleep 轮询、禁止 echo 输出通信、禁止无限循环、禁止 heredoc 创建文件

#### Scenario: Plan mode demotion
- **WHEN** agent 处于 Plan mode
- **THEN** shell_exec 的 ToolProfile SHALL 支持 demote，减少 prompt token 开销
