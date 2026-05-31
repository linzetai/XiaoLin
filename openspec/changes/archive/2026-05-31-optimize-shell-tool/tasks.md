## 1. 修复 Seatbelt 策略生成

- [x] 1.1 在 `seatbelt.rs` 的 `build_filesystem_policy` 中，当 `fs_policy.has_full_disk_read_access()` 为 true 时生成 `(allow file-read* (subpath "/"))`，跳过硬编码路径列表
- [x] 1.2 在 `seatbelt::transform` 和 `transform_with_proxy` 中将 `cwd` 设置到 `SandboxedCommand.working_dir`
- [x] 1.3 在 `landlock::transform` 中同样将 `cwd` 设置到 `SandboxedCommand.working_dir`
- [x] 1.4 添加单元测试：`has_full_disk_read_access` 策略生成正确的 seatbelt policy
- [x] 1.5 添加单元测试：`SandboxedCommand.working_dir` 非 None

## 2. Shell Runtime 核心修复

- [x] 2.1 将 `build_fs_policy` 中的 `NetworkSandboxPolicy::default()` 改为 `NetworkSandboxPolicy::Enabled`
- [x] 2.2 添加只读命令快速路径：`validate_readonly_command(command).is_ok()` 时使用 `build_plain_command` 绕过沙箱
- [x] 2.3 实现 signal 检测：用 `ExitStatusExt::signal()` 获取信号号，当 signal ∈ {SIGABRT, SIGKILL, SIGSYS} 且 stdout 为空且 sandbox != None 时返回 `ToolRuntimeError::SandboxDenied`
- [x] 2.4 改进输出格式：添加 `duration_ms`、`cwd` 元数据头，signal 时输出 `exit_code=SIGNAL(N)`
- [x] 2.5 将默认 timeout 从 30s 改为 120s

## 3. Schema 清理

- [x] 3.1 从 `shell_parameter_schema` 移除 `is_background` 参数
- [x] 3.2 从 `shell_parameter_schema` 移除 `shell` 参数
- [x] 3.3 添加 `timeout_ms` 参数到 schema（type: integer, description 包含默认值 120000）
- [x] 3.4 更新 `ShellDefinitionStub::description()` 文本，去掉 background 和 shell 相关描述，增加只读命令免沙箱、默认 120s timeout 等信息

## 4. 测试验证

- [x] 4.1 添加集成测试：`echo hello` 在 seatbelt sandbox 下返回 exit_code=0
- [x] 4.2 添加测试：signal 终止 + sandbox → SandboxDenied error
- [x] 4.3 添加测试：signal 终止 + 有 stdout → Ok(result) with SIGNAL info
- [x] 4.4 添加测试：只读命令 bypass sandbox
- [x] 4.5 添加测试：输出格式包含 duration_ms 和 cwd
- [x] 4.6 运行 `cargo check` 和 `cargo test` 确保无编译错误和测试失败
