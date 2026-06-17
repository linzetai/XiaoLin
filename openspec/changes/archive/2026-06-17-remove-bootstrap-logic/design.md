## Context

当前系统有一套 "bootstrap ritual" 机制：
1. `AgentWorkspace::ensure_bootstrap()` 创建初始文件（SOUL.md, IDENTITY.md, USER.md, AGENTS.md, TOOLS.md, BOOTSTRAP.md）
2. `ContextEngine` 在每次会话开始时检查 BOOTSTRAP.md 是否存在，若存在则注入 "Bootstrap Pending" 消息
3. BOOTSTRAP.md 期望 agent 在首次对话中完成身份发现仪式后自行删除该文件

问题：
- BOOTSTRAP.md 经常不被删除，导致后续所有新会话都带有无关上下文
- 机制依赖 LLM 执行"删除文件"操作，可靠性低
- identity 文件本身（SOUL.md 等）已足够定义 agent 行为，无需额外的"仪式"注入

## Goals / Non-Goals

**Goals:**
- 完全移除 BOOTSTRAP.md 文件及其相关逻辑
- 保持 workspace 初始化能力（创建 identity 模板文件）
- 新 workspace 仍然有合理的默认 identity 文件
- 消除 "Bootstrap Pending" 上下文注入

**Non-Goals:**
- 不改变 identity 文件的注入机制（SOUL.md、IDENTITY.md 等仍正常注入 prompt）
- 不修改 `get_identity` / `set_identity` 工具逻辑
- 不引入新的"初次启动向导"UI 流程

## Decisions

### D1: 删除而非条件跳过

**选择**: 彻底删除 bootstrap 相关代码，而非添加条件判断（如 `identity_already_configured`）

**理由**: bootstrap 概念本身多余——identity 模板文件直接告诉 agent "Fill this in"，agent 看到空模板自然会引导用户填写。不需要额外的仪式文件。

**替代方案**: 保留 BOOTSTRAP.md 但加自动清理 → 增加复杂度，仍然依赖文件状态判断

### D2: `ensure_bootstrap()` → `ensure_workspace()`

**选择**: 重命名方法为 `ensure_workspace()`，保留创建 identity 模板文件的逻辑，仅移除 BOOTSTRAP.md 的创建

**理由**: 方法的核心功能（确保 workspace 目录和 identity 模板文件存在）仍然有价值，只是名字不再准确

### D3: 对现有 BOOTSTRAP.md 不做迁移

**选择**: 代码不再读取 BOOTSTRAP.md，现有文件留在磁盘上不主动删除

**理由**: 不产生副作用，文件只是变成一个无用的 Markdown。用户可自行清理。

## Risks / Trade-offs

- [极低风险] 用户若依赖"首次启动仪式"行为 → 模板文件的占位符文本足以引导 agent，无功能损失
- [零风险] 现有 identity 文件内容不受影响 → 只移除 bootstrap 注入，不触碰 soul/identity/user/agents/tools 注入逻辑
