# Tasks

## T1: 修复 Goal 模式终止条件
- 涉及: Goal 完成判定逻辑（可能在 `goal_check` 或 `end_turn` 相关代码中）
- 问题: Agent 构建成功后仍不终止，因为 sandbox 路径限制导致某个子目标不可达
- 修复:
  - `tool_round.rs`: progress 追踪改为只计成功的工具调用（失败的 execute_command 不再重置 stagnation 计数器）
  - `goal_prompts.rs`: continuation prompt 新增 "Unreachable sub-goals" 段落，指导 agent 跳过因系统限制失败的子目标
- 状态: done

## T2: 修复 Sandbox 文件系统可见性
- 涉及: Sandbox overlay / 文件工具 (glob, list_dir, write_file 的缓存同步)
- 问题: Sub-agent 写入的文件对主 Agent 不可见（glob 返回不包含这些文件）
- 分析:
  - `FileStateCache` 未接线（`with_file_state_cache` 从未调用），不存在缓存问题
  - `glob`/`list_dir` 直接读磁盘，文件实际可见
- 修复:
  - `filesystem.rs`: glob 路径相对化改为使用 `workspace_root()` 而非 `std::env::current_dir()`，确保 sub-agent 和主 agent 返回一致的路径
  - 根因可能是 agent 推理误判，T1 的 "unreachable sub-goals" 指导有助于缓解
- 状态: done

## T3: 稳定 Shell/execute_command 环境
- 涉及: `execute_command` 工具实现（与 `terminal_input` 对比）
- 问题: execute_command 报 "zsh 找不到" 但 terminal_open + terminal_input 正常
- 根因: `exec_command.rs` 默认 shell 硬编码为 `"bash"`，而 `terminal_open` 用 `$SHELL`
- 修复:
  - `exec_command.rs`: 新增 `default_shell()` 函数，优先使用 `$SHELL` 环境变量，回退到 `/bin/bash`
  - 与 `xiaolin-pty` 的 `default_shell()` 逻辑对齐
- 状态: done

## T4: 修复 Sub-agent 输出截断
- 涉及: Sub-agent 调度器（custom:code 类型 sub-agent 的 max_tokens 配置）
- 问题: Sub-agent 写入大文件时输出被截断
- 根因: `sidechain.rs` 中 `MAX_RESULT_CHARS = 12288` (12KB) 硬截断
- 修复:
  - `sidechain.rs`: `MAX_RESULT_CHARS` 提升至 32768 (32KB)
  - 截断策略改为保留 head (75%) + tail (25%)，中间省略，保留更多上下文
- 状态: done

## T5: 优化 terminal_input 超时机制
- 涉及: terminal_input 工具实现
- 问题: 固定 wait_ms=30000，即使命令 100ms 就完成也要等 30s
- 修复:
  - default wait_ms=2000（之前已改），wait_for param，500ms idle early-return
  - `terminal.rs`: 新增 `ends_with_shell_prompt()` 函数，检测常见 shell prompt 模式（`$`, `%`, `#`, `❯`, `➜`, `> `）
  - `collect_output()`: 当无 `wait_for` 参数时自动检测 prompt，检测后 50ms grace period 即返回
  - 6 个新单元测试验证 prompt 检测正确性和无误报
- 状态: done

## T6: 修复前端任务进度计数器
- 涉及: 前端 task progress UI 组件
- 问题: 显示 "0/12" 但实际多个子任务已标记 completed
- 调查方向:
  - 检查 task_management 工具的 "mark complete" 是否触发前端更新事件
  - 可能是 WebSocket 事件未推送到 UI 或 UI 未监听正确的 state change
- 状态: done — MessageStream 现在扫描最新 streamSegments 获取 todo 进度
