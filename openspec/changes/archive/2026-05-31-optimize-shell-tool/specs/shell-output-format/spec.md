## ADDED Requirements

### Requirement: Shell output includes execution metadata

shell_exec 的输出 SHALL 包含元数据头部，提供执行时间、工作目录和退出状态信息。

#### Scenario: Successful command output format
- **WHEN** 命令 `ls -la` 成功执行（exit code 0），耗时 45ms，在 `/Users/foo/project` 目录
- **THEN** 输出格式为:
  ```
  exit_code=0
  duration_ms=45
  cwd=/Users/foo/project
  ---
  <ls output>
  ```

#### Scenario: Failed command with stderr
- **WHEN** 命令 `cat nonexist.txt` 失败（exit code 1），stderr 非空
- **THEN** 输出格式为:
  ```
  exit_code=1
  duration_ms=12
  cwd=/Users/foo/project
  ---
  stdout:
  
  stderr:
  cat: nonexist.txt: No such file or directory
  ```

#### Scenario: Signal-terminated command output
- **WHEN** 命令被 signal 9 杀死，sandbox 自动 escalate 后重试成功
- **THEN** 最终输出为重试后的正常输出（escalation 对 agent 透明）

#### Scenario: Signal without escalation
- **WHEN** 命令被信号终止且不满足 escalation 条件（非沙箱或 stdout 非空）
- **THEN** 输出格式为:
  ```
  exit_code=SIGNAL(15)
  duration_ms=30002
  cwd=/Users/foo/project
  ---
  <partial stdout if any>
  ```

### Requirement: Schema matches implementation

shell_exec 的 parameter schema SHALL 只声明实际生效的参数。

#### Scenario: Valid parameters
- **WHEN** agent 查看 shell_exec 的 parameters_schema
- **THEN** 包含 `command`（required string）、`working_dir`（optional string）、`timeout_ms`（optional integer，默认 120000）、`description`（optional string）
- **THEN** 不包含 `is_background`、`shell` 参数

#### Scenario: Timeout parameter works
- **WHEN** agent 传入 `timeout_ms: 5000` 且命令执行超过 5 秒
- **THEN** 命令被终止，返回 timeout 错误

#### Scenario: Unknown parameters ignored gracefully
- **WHEN** agent 传入已移除的 `is_background: true` 参数
- **THEN** 参数被忽略，命令正常以 foreground 模式执行

### Requirement: Tool description reflects actual behavior

shell_exec 的 description SHALL 准确描述工具的实际行为和限制。

#### Scenario: Description content
- **WHEN** agent 读取 shell_exec 的 description
- **THEN** 描述包含：默认 timeout（120s）、使用 sh 执行、会在沙箱中运行（写操作）、只读命令直接执行
- **THEN** 不包含对 background mode 或 shell 选择的提及
