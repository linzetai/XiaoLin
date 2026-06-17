# Proposal: 修复 Agent 长程任务中发现的关键问题

## 背景

通过 TaskFlow 项目管理看板的开发任务（Goal 模式 + Plan 模式）进行了长程任务能力测试。
测试发现多个影响 Agent 自主完成能力的 Bug。

## 测试条件

- 模型: deepseek-v4-flash
- 模式: Goal（完全自动）+ Plan（只读探索）
- 任务: 从零创建 React + Vite + localStorage 的项目管理看板
- 耗时: Goal 35+ 分钟（手动终止）/ Plan 4m36s（自然完成）

---

## 问题 1: Goal 模式无法自然终止（🔴 严重）

### 现象

Agent 在完成代码编写并构建成功后，陷入无限验证循环：
1. 构建通过 → "所有 23 个文件验证通过"
2. 尝试复制到 /tmp（sandbox 禁止）→ 失败
3. 回到步骤 1 重新验证
4. 35 分钟后仍未终止

### 根因假设

1. Goal 完成判定逻辑未将 "构建成功" 视为目标达成信号
2. Sandbox 路径限制导致 Agent 认为 "目标未完成"（指定了 /tmp 但只能写 workspace）
3. Agent 缺乏 "尽力而为" 的退出策略 — 当某个子目标不可达时应跳过而非循环

### 影响

- 用户必须手动停止，否则无限消耗 token
- 实际工作早已完成但无法感知

---

## 问题 2: Sandbox 文件可见性不一致（🔴 严重）

### 现象

Agent 使用 glob/list_dir 工具报告文件 "不存在"，但文件实际存在于磁盘：
- `Register.jsx` (2259 bytes) — Agent 报告缺失
- `Projects.jsx` (5795 bytes) — Agent 报告缺失
- `Layout.jsx` (257 bytes) — Agent 报告缺失

### 根因假设

1. Sandbox 文件系统 overlay 的缓存与实际文件系统不同步
2. Sub-agent 写入的文件未被主 Agent 的文件系统视图刷新
3. 可能是 glob 工具在特定时机返回了过期的目录快照

### 影响

- Agent 反复 "修复" 不存在的问题
- 50+ 次冗余文件读取（验证已存在的文件）
- 整体效率降低 5-10x

---

## 问题 3: Shell 环境间歇性不可用（🟡 中等）

### 现象

Agent 的 Shell 执行环境在任务执行过程中多次报告 "Shell 环境不可用（zsh 找不到）"：
- `execute_command` 工具返回 Terminal 错误
- 但 `terminal_open` + `terminal_input` 路径可以工作
- 同一会话内 Shell 状态不稳定

### 根因假设

1. `execute_command` 使用的 shell 路径与 `terminal_input` 不同
2. 可能 execute_command 尝试用 `/bin/sh` 或默认 shell，而用户环境只有 zsh
3. 终端 session 超时后未正确重建

### 影响

- npm install 多次失败后才通过 terminal_input 绕过
- 增加了约 2-3 分钟的无效重试时间

---

## 问题 4: Sub-agent 输出截断（🟡 中等）

### 现象

- Agent 报告 "sub-agent task was truncated"
- CSS 文件写入被截断需要重试
- 大文件（>3KB）写入可能不完整

### 根因假设

1. Sub-agent 的输出 token 限制过低
2. custom:code 类型 sub-agent 的 response 被 max_tokens 截断
3. 写入文件的 content 参数可能有长度限制

### 影响

- 需要额外的读取+重写循环修复截断
- 增加 1-2 分钟的修复时间

---

## 问题 5: terminal_input 固定超时（🟢 轻微）

### 现象

每次 `terminal_input` 都等待完整的 `wait_ms`（30 秒），即使命令在 1 秒内完成。

### 建议

- terminal_input 应支持 "等待 prompt 出现" 的模式而非固定超时
- 或者自动检测命令完成（shell prompt 重新出现）

---

## 问题 6: 任务进度计数器不更新（🟢 轻微）

### 现象

- UI 显示 "0/12" 但实际 10+ 个子任务已完成
- Plan 模式完成后仍显示 "0/6"

### 根因假设

- 前端的进度计数器可能只在特定事件时更新
- 或者 task_management 工具的 "完成" 标记未同步到前端状态

---

## 优先级排序

| 优先级 | 问题 | 预计影响 |
|--------|------|----------|
| P0 | Goal 模式无法终止 | 所有长程任务都会无限循环 |
| P0 | Sandbox 文件可见性 | 导致 Agent 在循环中做无用功 |
| P1 | Shell 环境不稳定 | 阻塞 npm/build 类命令 |
| P1 | Sub-agent 输出截断 | 大文件创建可靠性 |
| P2 | terminal_input 超时 | 效率优化 |
| P2 | 进度计数器 | UI 体验 |
