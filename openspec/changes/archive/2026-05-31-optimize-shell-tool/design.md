## Context

XiaoLin 的 `shell_exec` 工具是 agent 执行命令的唯一通道。当前实现在 macOS 上因 Seatbelt 沙箱策略错误导致几乎所有命令失败（SIGABRT），同时存在 cwd 不传递、网络被禁、schema 与实现不一致等多个问题。

核心执行链路：`ShellRuntime.run()` → `SandboxManager.transform()` → `sandbox-exec -p "policy" -- sh -c "command"` → 结果捕获。

## Goals / Non-Goals

**Goals:**
- 修复 seatbelt 策略生成，使 shell 命令能正常执行并返回正确 exit code
- 修复 sandbox 模式下 working_dir 传递
- 清理 schema，使声明与实现一致
- 改进输出格式，为 agent 提供有用的执行上下文
- 实现只读命令快速路径（免沙箱免审批）
- 实现 signal-death 自动 escalation

**Non-Goals:**
- 实现后台执行模式（方案 A：移除 is_background，用 nohup 自行处理）
- 实现自定义 shell 选择（统一用 sh，简化实现）
- 改变 approval cache 逻辑（已在之前修复）
- 流式输出/进度反馈（后续优化）

## Decisions

### D1: Seatbelt 策略修复策略

**决策**: 在 `build_filesystem_policy` 中检查 `fs_policy.has_full_disk_read_access()`，若为 true 则生成 `(allow file-read* (subpath "/"))`。

**理由**: 这与 `FileSystemSpecialPath::Root` 的语义完全一致。deny-globs 仍然生效因为 seatbelt 的 deny 规则优先于 allow。

**替代方案**: 只添加 `(literal "/")` 到硬编码路径 — 太 hacky，不解决根本问题。

### D2: working_dir 传递方式

**决策**: 在 `seatbelt::transform` 和 `landlock::transform` 中将 `cwd` 参数设置到 `SandboxedCommand.working_dir`。

**理由**: `cwd` 已经传入 transform 函数但只用于策略生成，直接复用即可。`into_tokio_command` 已有处理 `working_dir` 的逻辑。

### D3: 网络默认策略

**决策**: ShellRuntime 中使用 `NetworkSandboxPolicy::Enabled` 替代 `default()`（即 Restricted）。

**理由**: Shell 命令经常需要网络（git fetch、curl、npm install）。沙箱的主要目的是文件系统隔离，网络限制不在当前产品需求范围内。

**替代方案**: 根据命令内容判断是否需要网络 — 太复杂且不可靠。

### D4: 只读命令免沙箱

**决策**: 当 `validate_readonly_command(command).is_ok()` 时，直接使用 `SandboxBackend::None`。

**理由**: `echo`、`ls`、`cat`、`git status` 等只读命令不会修改系统状态，无需沙箱开销。同时这些命令也应免除审批（但审批逻辑在 orchestrator 的 exec_requirement 中决定，这里只影响 sandbox 选择）。

### D5: Signal-death escalation

**决策**: 在 `ShellRuntime.run()` 中，当 `status.code() == None`（被信号杀死）且当前使用了沙箱时，返回 `ToolRuntimeError::SandboxDenied` 而非 `Ok(result)`。这触发 orchestrator 的 escalation 路径自动重试。

**理由**: 信号终止 + 沙箱环境 = 大概率是沙箱策略问题。自动重试比报错给 agent 更好。

**边界**: 如果 stdout 非空且 exit signal 不是 SIGABRT/SIGKILL/SIGSYS，视为命令本身行为，不触发 escalation。

### D6: 输出格式

**决策**: 输出增加元数据头部：

```
exit_code=0
duration_ms=142
cwd=/path/to/dir
---
<stdout content>
```

当有 stderr 时：
```
exit_code=1
duration_ms=350
cwd=/path/to/dir
---
stdout:
<stdout>
stderr:
<stderr>
```

当被 signal 杀死时：
```
exit_code=SIGNAL(9)
duration_ms=30012
cwd=/path/to/dir
sandbox=seatbelt (escalated to none)
---
<stdout if any>
```

**理由**: Agent 需要 duration 来判断命令性能、cwd 确认执行位置、signal info 理解失败原因。

### D7: Schema 清理

**决策**: 
- 移除 `is_background` 参数
- 移除 `shell` 参数
- 添加 `timeout_ms`（类型 integer，默认 120000，最大 300000）
- 更新 description 与实际行为一致

## Risks / Trade-offs

- **[Risk] `(allow file-read* (subpath "/"))` 过于宽松** → Mitigation: deny-globs（.env, .ssh 等）仍然生效；这本就是 `FileSystemSpecialPath::Root` 的设计意图
- **[Risk] 移除 is_background 影响现有 agent prompts** → Mitigation: 如果 agent 传了 is_background，runtime 忽略即可（不报错），在描述中引导用 nohup
- **[Risk] 只读命令免沙箱可能有安全隐患** → Mitigation: readonly 白名单已经非常保守，且只影响 Plan mode 判断，exec 仍需 approval（除非 session 级已批准）
- **[Risk] Signal escalation 可能在非沙箱原因时误触发** → Mitigation: 只在 SIGABRT/SIGKILL/SIGSYS + stdout 为空时触发
