# Sub-Agent Monitor Panel

## Overview
前端实时 sub-agent 状态监控面板，位于聊天区域右侧，自动根据 sub-agent 活跃状态 show/hide。

## Requirements

### R1: 自动显隐
- 当前 session 有活跃 sub-agent runs 时自动 slide-in 显示
- 所有 sub-agent 完成后延迟 3s 自动 slide-out 隐藏
- 用户可手动 toggle

### R2: 状态展示
- 每个 sub-agent run 显示: type icon、task 描述、状态(running/completed/failed)、elapsed time、tool call count
- Running 状态: 显示当前正在执行的 tool 名称
- Completed 状态: 显示结果摘要（前 200 字符）
- Failed 状态: 显示错误信息

### R3: 交互操作
- Cancel 按钮: 取消正在运行的 sub-agent
- 点击展开查看详细信息
- Result 可复制

### R4: 布局与响应式
- 固定宽度 280px，slide-in 动画
- 小屏（< 1024px）时改为 overlay drawer 模式
- 不挤压聊天输入区域的可用宽度

### R5: 数据源
- 复用 WebSocket 的 SubAgent* 事件
- 复用 useAgentStore 中的 SubAgentRunUI 状态
