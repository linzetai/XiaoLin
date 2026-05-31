## Why

Shell 工具（`shell_exec`）当前存在多个关键缺陷：Seatbelt 沙箱策略生成错误导致所有命令被 SIGABRT 杀死（exit_code=-1）、sandbox 模式下工作目录不生效、网络默认被禁止、schema 与实现严重不一致（`is_background`/`shell` 参数声明了但未实现）。这些问题使得 shell 工具在 macOS 上几乎不可用，agent 无法可靠地执行任何命令。

## What Changes

- **修复 Seatbelt 策略生成**：正确处理 `FileSystemSpecialPath::Root`，生成 `(allow file-read* (subpath "/"))` 而非丢弃
- **修复 sandbox 下 cwd 传递**：将用户指定的 `working_dir` 正确传递给 `SandboxedCommand`
- **修复网络策略默认值**：shell 命令默认允许网络访问（`NetworkSandboxPolicy::Enabled`）
- **移除虚假参数**：移除 `is_background` 和 `shell` 参数（未实现且不计划实现）
- **添加 `timeout_ms` 到 schema**：将已存在的隐藏参数正式声明，默认 120 秒
- **Signal 检测与自动 escalation**：进程被信号杀死时，若使用了沙箱则自动 retry 无沙箱执行
- **只读命令免沙箱**：通过已有的 `validate_readonly_command` 跳过沙箱和审批
- **改进输出格式**：增加执行时间、cwd 确认、signal 信息等元数据
- **修正工具描述**：与实际行为一致

## Capabilities

### New Capabilities
- `shell-sandbox-policy`: Shell 沙箱策略的正确生成与 fallback 机制
- `shell-output-format`: Shell 工具输出格式规范与元数据

### Modified Capabilities

（无已有 spec 需要修改）

## Impact

- `crates/fastclaw-sandbox/src/seatbelt.rs` — 策略生成修复
- `crates/fastclaw-sandbox/src/lib.rs` — `SandboxedCommand` working_dir 传递
- `crates/fastclaw-sandbox/src/landlock.rs` — 同上
- `crates/fastclaw-agent/src/runtime/runtimes/shell.rs` — 核心执行逻辑重构
- `crates/fastclaw-agent/src/builtin_tools/shell.rs` — schema 清理
- `crates/fastclaw-agent/src/runtime/orchestrator.rs` — escalation 逻辑增强
- 无 API 变更、无外部依赖变更
