## ADDED Requirements

### Requirement: Seatbelt policy handles full-disk read access

当 `FileSystemSandboxPolicy` 包含 `FileSystemSpecialPath::Root` 且 access 为 Read 时，seatbelt 策略生成器 SHALL 生成 `(allow file-read* (subpath "/"))` 规则。

#### Scenario: Policy with Root read generates correct seatbelt rule
- **WHEN** `build_filesystem_policy` 接收到 `has_full_disk_read_access() == true` 的策略
- **THEN** 生成的 seatbelt policy 包含 `(allow file-read* (subpath "/"))`
- **THEN** 不再单独列出 `/usr/lib`、`/bin` 等硬编码路径（已被 subpath "/" 覆盖）

#### Scenario: Policy without Root read still uses specific paths
- **WHEN** `build_filesystem_policy` 接收到没有 Root read 的 restricted 策略
- **THEN** 生成的 seatbelt policy 只包含具体路径的 `(allow file-read* (subpath "..."))` 规则
- **THEN** 保留硬编码系统路径列表作为 fallback

### Requirement: Sandbox command preserves working directory

`SandboxedCommand` SHALL 将调用者指定的 working directory 传递给子进程。

#### Scenario: Seatbelt transform sets working_dir
- **WHEN** `seatbelt::transform` 被调用时 `cwd` 参数为 `/Users/foo/project`
- **THEN** 返回的 `SandboxedCommand.working_dir` 为 `Some(PathBuf::from("/Users/foo/project"))`
- **THEN** `into_tokio_command()` 生成的 Command 设置了 `current_dir("/Users/foo/project")`

#### Scenario: Landlock transform sets working_dir
- **WHEN** `landlock::transform` 被调用时 `cwd` 参数为 `/home/user/project`
- **THEN** 返回的 `SandboxedCommand.working_dir` 为 `Some(PathBuf::from("/home/user/project"))`

### Requirement: Shell commands have network access by default

ShellRuntime SHALL 使用 `NetworkSandboxPolicy::Enabled` 作为默认网络策略。

#### Scenario: Shell command can access network
- **WHEN** ShellRuntime 构建沙箱策略
- **THEN** 使用 `NetworkSandboxPolicy::Enabled`
- **THEN** 生成的 seatbelt policy 包含 `(allow network-outbound)` 和 `(allow network-inbound)`

### Requirement: Signal-terminated process triggers sandbox escalation

当进程被 SIGABRT、SIGKILL 或 SIGSYS 信号终止，且 stdout 为空，且使用了沙箱时，ShellRuntime SHALL 返回 `ToolRuntimeError::SandboxDenied` 以触发自动 escalation。

#### Scenario: SIGABRT with empty stdout triggers escalation
- **WHEN** sandbox-exec 下的命令被 SIGABRT 杀死
- **THEN** ShellRuntime 返回 `Err(ToolRuntimeError::SandboxDenied { reason: "process killed by signal 6 (sandbox policy violation suspected)" })`
- **THEN** Orchestrator 自动用 `SandboxBackend::None` 重试

#### Scenario: Signal with non-empty stdout does not escalate
- **WHEN** 命令被信号终止但 stdout 已有内容
- **THEN** ShellRuntime 返回 `Ok(result)` 并在输出中报告信号信息
- **THEN** 不触发 escalation

#### Scenario: Signal without sandbox does not escalate
- **WHEN** 命令在 `SandboxBackend::None` 下被信号终止
- **THEN** ShellRuntime 返回 `Ok(result)` 并报告信号
- **THEN** 不触发 escalation

### Requirement: Readonly commands bypass sandbox

当命令通过 `validate_readonly_command` 检查时，ShellRuntime SHALL 跳过沙箱直接执行。

#### Scenario: Echo command runs without sandbox
- **WHEN** agent 调用 `shell_exec` 执行 `echo hello`
- **THEN** `validate_readonly_command("echo hello")` 返回 Ok
- **THEN** 使用 `build_plain_command` 直接执行，不经过 sandbox-exec

#### Scenario: Git status runs without sandbox
- **WHEN** agent 调用 `shell_exec` 执行 `git status`
- **THEN** 跳过沙箱直接执行

#### Scenario: Write command still uses sandbox
- **WHEN** agent 调用 `shell_exec` 执行 `rm -rf temp/`
- **THEN** `validate_readonly_command` 返回 Err
- **THEN** 正常走沙箱流程
