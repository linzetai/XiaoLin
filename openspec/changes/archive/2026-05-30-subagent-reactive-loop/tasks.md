## 1. Reactive Loop 核心 (Harness)

- [x] 1.1 在 `session_bridge.rs` 中新增 `reactive_loop` 函数，包裹 `execute_unified` 调用，实现 outer loop
- [x] 1.2 实现 turn 结束守卫逻辑：`execute_unified` 返回后检查 `subagent_manager.active_runs(session_id)` 是否为空
- [x] 1.3 实现 sub-agent completion 等待机制：使用 `SubAgentManager` 的 broadcast/channel 监听 completion 事件
- [x] 1.4 实现 batch window：等待首个 completion 后额外等待 2s（可配置）收集更多 completions
- [x] 1.5 实现 completion notification system message 构建：格式化 sub-agent 结果为 structured text
- [x] 1.6 将 completion notification 注入 conversation messages 并触发 re-prompt（重新调用 execute_unified）
- [x] 1.7 新增配置项 `SubAgentPolicy`：`batch_window_ms`、`max_reprompts_per_turn`、`max_spawns_per_turn`、`suppress_intermediate_ack`
- [x] 1.8 实现 intermediate ack suppression：如果 LLM re-prompt 后仅输出文本无 tool calls 且仍有 active runs，不 stream 给前端

## 2. SubAgentManager 增强

- [x] 2.1 新增 `SubAgentManager::subscribe_completions(session_id)` 方法，返回 broadcast Receiver 通知 run 完成
- [x] 2.2 `spawn` 完成时自动广播 completion event（包含 run_id、status、result summary）
- [x] 2.3 新增 `active_runs(session_id) -> Vec<SubAgentRun>` 便捷方法
- [x] 2.4 新增 `get_completion_summary(run_id) -> CompletionSummary` 方法，返回结构化摘要

## 3. 工具集调整与 Prompt 增强

- [x] 3.1 在 `prompt_builder.rs` 中重写 sub-agent delegation guidance，强化 delegation trigger signals
- [x] 3.2 移除 `wait_agent` 从推荐工具列表（保留注册但从 prompt guidance 中移除推荐）
- [x] 3.3 新增 re-prompt instruction 模板：引导 LLM 在收到 completion notification 后高效决策
- [x] 3.4 增强 `SubAgentPromptContext`：注入当前活跃 runs 状态摘要到每轮 prompt

## 4. Protocol 扩展

- [x] 4.1 在 `event.rs` 中新增 `AgentEvent::SubAgentNotification` variant（用于前端展示 harness re-prompt 触发）
- [x] 4.2 定义 `CompletionSummary` struct：run_id、type、task、status、elapsed_ms、tool_call_count、result_preview
- [x] 4.3 确保 WebSocket stream 正确 emit `SubAgentNotification` 事件

## 5. 前端监控面板

- [x] 5.1 创建 `SubAgentMonitor` 组件：显示当前 session 所有 sub-agent runs 的实时状态
- [x] 5.2 实现 auto show/hide 逻辑：有活跃 runs 时 slide-in，全部完成后延迟 3s slide-out
- [x] 5.3 每个 run item 展示：type icon、task、status badge、elapsed timer、current tool name
- [x] 5.4 实现 cancel 按钮交互
- [x] 5.5 实现点击展开详情（result preview、tool call list）
- [x] 5.6 修改 `AppLayout.tsx` 将 `SubAgentMonitor` 集成到聊天区右侧
- [x] 5.7 响应式布局：小屏 < 1024px 时改为 overlay drawer 模式

## 6. 集成测试与配置

- [x] 6.1 新增 reactive loop 集成测试：spawn → wait → completion notification → re-prompt → turn end
- [x] 6.2 新增 batch window 测试：多个 sub-agent 短时间完成合并为一次 re-prompt
- [x] 6.3 新增 turn 守卫测试：LLM 停止但 sub-agent 仍活跃时 turn 不结束
- [x] 6.4 新增 `reactive_loop.enabled` 配置 flag，默认开启，支持回退到旧模式
- [x] 6.5 更新 agent config schema，添加 SubAgentPolicy 相关字段
