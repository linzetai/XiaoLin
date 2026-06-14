# Tasks

## T1: 修复 Goal 模式终止条件
- 涉及: Goal 完成判定逻辑（可能在 `goal_check` 或 `end_turn` 相关代码中）
- 问题: Agent 构建成功后仍不终止，因为 sandbox 路径限制导致某个子目标不可达
- 修复方向:
  - 当 Agent 连续多次尝试同一操作失败时，应触发 "graceful exit"
  - 或：当 Agent 明确输出"项目完成/构建成功"类文本后，Goal 完成判定应触发
  - 或：添加 "不可达子目标跳过" 机制 — 当 sandbox 限制阻止某操作时标记为 skipped 而非 failed
- 状态: pending

## T2: 修复 Sandbox 文件系统可见性
- 涉及: Sandbox overlay / 文件工具 (glob, list_dir, write_file 的缓存同步)
- 问题: Sub-agent 写入的文件对主 Agent 不可见（glob 返回不包含这些文件）
- 复现: Sub-agent 写入文件后，主 Agent 使用 glob 搜索同目录 → 文件不在结果中
- 修复方向:
  - Sub-agent 完成后刷新文件系统缓存
  - 或：glob/list_dir 每次都从磁盘读取而非缓存
- 状态: pending

## T3: 稳定 Shell/execute_command 环境
- 涉及: `execute_command` 工具实现（与 `terminal_input` 对比）
- 问题: execute_command 报 "zsh 找不到" 但 terminal_open + terminal_input 正常
- 调查方向:
  - 对比 execute_command 和 terminal_input 使用的 shell 路径
  - 确认 execute_command 是否正确继承用户 PATH
  - 检查是否有 sandbox 对 /bin/zsh 的访问限制
- 状态: pending

## T4: 修复 Sub-agent 输出截断
- 涉及: Sub-agent 调度器（custom:code 类型 sub-agent 的 max_tokens 配置）
- 问题: Sub-agent 写入大文件时输出被截断
- 修复方向:
  - 增加 custom:code sub-agent 的 max_output_tokens
  - 或：将大文件拆分为多次写入
  - 或：检测截断并自动 retry（在 sub-agent 调度层）
- 状态: pending

## T5: 优化 terminal_input 超时机制
- 涉及: terminal_input 工具实现
- 问题: 固定 wait_ms=30000，即使命令 100ms 就完成也要等 30s
- 修复方向:
  - 支持 "等待 shell prompt" 模式（检测 $ 或 % 或 ❯ 等提示符出现）
  - 或：支持 "命令完成检测"（exit code 可用时立即返回）
  - 保留 wait_ms 作为最大超时而非固定等待
- 状态: pending

## T6: 修复前端任务进度计数器
- 涉及: 前端 task progress UI 组件
- 问题: 显示 "0/12" 但实际多个子任务已标记 completed
- 调查方向:
  - 检查 task_management 工具的 "mark complete" 是否触发前端更新事件
  - 可能是 WebSocket 事件未推送到 UI 或 UI 未监听正确的 state change
- 状态: pending
