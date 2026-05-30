# Sub-Agent Reactive Loop

## Overview
Harness 级响应式编排循环，使主 Agent 的 LLM 只需 spawn sub-agents，由系统自动完成等待和结果回注。

## Requirements

### R1: Supervised Wait
- 当主 agent turn 中存在活跃 sub-agent runs 时，harness 自动进入等待状态
- Turn 不得在有活跃 sub-agent 时结束

### R2: Completion-Driven Re-prompt
- 任意 sub-agent 完成时，harness 将结果作为 system message 注入 context 并 re-prompt 主 LLM
- 主 LLM 可在 re-prompt 中 spawn 新任务、推理、或无操作继续等待

### R3: Batch Window
- 短时间窗口内（默认 2s）完成的多个 sub-agent 合并为一次 re-prompt
- 避免高频 re-prompt 浪费 tokens

### R4: Turn 结束守卫
- Turn 结束条件 = LLM 无新 tool calls AND active_sub_agents == 0
- 如果 LLM 停止输出但仍有 active runs，自动进入等待

### R5: 工具集调整
- `spawn_subagent` 保留，语义不变
- `wait_agent` soft-deprecate（保留注册但移出推荐 prompt）
- `subagent_get` 保留用于手动查询
- `cancel_subagent` 保留

### R6: Delegation Prompt 增强
- 强化 delegation trigger signals
- 引导 LLM 在识别并行机会时积极使用 sub-agent
- Re-prompt 时的 instruction 引导 LLM 高效决策

### R7: 前端状态监控面板
- 聊天区域旁实时显示所有活跃 sub-agent 状态
- 仅有活跃 sub-agent 时自动出现
- 显示: type、task、elapsed time、tool call count、当前动作
- 支持 cancel 操作
