# 工具暴露与提示词组装架构探索

> 状态：探索中 | 日期：2026-06-01

## 背景

在实现 `plan-mode-v2` 过程中发现两个 Bug：
1. `exit_plan_mode` 工具注册为 deferred，Plan 模式下 agent 不可见
2. `PlanFileStore` executor 和 gateway 使用了两个独立实例

这暴露了更深层的架构问题：**工具暴露逻辑是静态的**，缺少根据运行时上下文（模式、agent 类型、session 状态）动态组装工具集和提示词的能力。

## 竞品分析

### 三个项目的工具暴露策略对比

| 维度 | FastClaw（当前） | Claude Code | Codex CLI |
|------|----------------|-------------|-----------|
| 工具分层 | eager/deferred 二分 | 全量 → 管线过滤 | feature flag + exposure enum |
| Plan 模式工具 | 从 schema 移除写工具 | **保留 schema，运行时拦截** | **保留 schema，sandbox + prompt 约束** |
| 模式执行 | dispatcher 阻塞 | 运行时权限 + prompt attachment | sandbox + orchestrator approval |
| 子 agent 工具 | 硬编码 match 枚举 | 声明式 allow/deny Set | config 层角色 TOML |
| 模式转换 | 分散在各 tool 中 | 集中 `transitionPermissionMode` | `build_settings_update_items` 增量 |
| 提示词缓存 | 无分层 | 静态前缀 + 动态 section + per-turn | base_instructions + developer fragments |
| 工具发现 | tool_search (deferred → eager) | tool_search + shouldDefer | tool_search + ToolExposure::Deferred |

### 核心共识

**三个项目都在 Plan 模式下保留写工具的 schema**——只是通过运行时约束阻止执行。FastClaw 的 dispatcher 阻塞方式本身是对的，但没有同步提升 `exit_plan_mode` 到可见状态。

---

## Claude Code 架构详解

### 工具组装管线

```
getAllBaseTools()           ← 全量工具（feature flag 过滤）
       ↓
getTools(permCtx)          ← deny rules + isEnabled
       ↓
assembleToolPool()         ← + MCP 工具，去重排序
       ↓
mergeAndFilterTools()      ← coordinator 模式硬过滤
       ↓
resolveAgentTools()        ← 子 agent allow/deny 列表
       ↓
运行时权限检查              ← plan/auto/bypass per call
       ↓
动态 attachments           ← plan_mode / auto_mode 指令注入
```

### Plan 模式执行策略（多层软约束）

| 层级 | 机制 | 说明 |
|------|------|------|
| Prompt | plan_mode attachment | 每 5 轮完整提醒，间隔简短提醒 |
| 文件例外 | checkEditableInternalPath | 只允许写 session plan/scratchpad |
| 写工具 | checkWritePermissionForTool | 仍然走 ask 审批 |
| Bash | BashTool.isReadOnly() | 只读命令自动通过 |
| 子 agent | disallowedTools | Explore/Plan agent 移除 Edit/Write |
| 工具门控 | ExitPlanMode validates mode | 确保只在 plan 模式下可用 |

### 子 agent 工具 Profile

```typescript
// 声明式定义，位于 built-in agent 配置中
const EXPLORE_AGENT = {
  disallowedTools: [Agent, ExitPlanMode, FileEdit, FileWrite, NotebookEdit],
  // ...
}

// 解析管线
function resolveAgentTools(agentDef, availableTools, isAsync) {
  // Main thread: skip filter
  // Wildcard ['*']: all allowed (after global blocks)
  // Explicit allowlist: intersect
  // disallowedTools: per-agent denylist
}
```

### 提示词分层

| 层级 | 缓存策略 | 内容 |
|------|----------|------|
| 静态前缀 | 跨 turn 不变 | 身份、工具使用指南、输出格式 |
| 动态 section | session 级缓存 | 环境信息、MCP 指令、记忆 |
| per-turn attachment | 每轮重算 | plan_mode/auto_mode 提醒 |

关键 API：
- `systemPromptSection(name, compute)` — 缓存到 `/clear` 或 `/compact`
- `DANGEROUS_uncachedSystemPromptSection(name, compute, reason)` — 每轮重算

---

## Codex CLI 架构详解

### 三层分离

1. **Tool registry** — 模型能调什么（feature/session 门控）
2. **Permission profile + orchestrator** — 实际执行什么（sandbox + approval）
3. **Prompt fragments** — 告诉模型什么（模式指令、权限文本）

### ToolExposure 枚举

```rust
pub enum ToolExposure {
    Direct,          // 始终在工具列表中
    Deferred,        // 通过 tool_search 发现
    DirectModelOnly, // 模型可见但不嵌套（code mode）
}
```

### 审批模型（纵深防御）

| 层级 | 机制 |
|------|------|
| Approval preset | read-only / auto / full-access |
| AskForApproval | UnlessTrusted / OnRequest / Granular / Never |
| PermissionProfile | Managed{fs, network} / Disabled / External |
| Orchestrator | approval check → sandbox → execute → retry |
| Execpolicy | 前缀规则自动批准已知安全命令 |
| Guardian | auto_review 子 agent 审批 |

### 子 agent 配置层

```toml
# codex-rs/core/src/agent/roles/awaiter.toml
[config]
developer_instructions = "..."
reasoning_effort = "low"
# 继承父 agent 的工具集，通过 config 层覆盖行为
```

**没有声明式 tool allow/deny 列表**（与 Claude Code 不同）。通过 config 层角色（TOML + feature flags + 指令替换）实现差异化。

---

## FastClaw 改进方向

### 方向 A：最小修复（当前已应用）

在 `execute_unified` 中加 `tool_registry.activate_deferred("exit_plan_mode")`。

**优点**：改动小，立即生效
**缺点**：治标不治本，每次加新模式都需要手动处理

### 方向 B：引入 ToolProfile + 管线化组装

```rust
struct ToolProfile {
    promote: Vec<String>,     // deferred → eager
    demote: Vec<String>,      // eager → hidden
    runtime_block: Vec<BlockRule>,  // 保留 schema 但运行时拦截
}

// 用法
let profile = mode.tool_profile();
let defs = registry.definitions_with_profile(&profile);
```

**优点**：
- 声明式，易于理解和扩展
- 支持 agent 级别的自定义 profile
- 与现有 dispatcher 阻塞逻辑兼容

**缺点**：
- 需要重构工具收集逻辑
- Profile 组合可能复杂化

### 方向 C：全面管线化（Claude Code 风格）

```rust
fn assemble_tool_pool(
    registry: &ToolRegistry,
    mode: ExecutionMode,
    agent_def: Option<&AgentDefinition>,
    mcp_tools: &[ToolDefinition],
    deny_rules: &[DenyRule],
) -> Vec<ToolDefinition>
```

配合提示词分层：
```rust
enum PromptLayer {
    Static(Vec<PromptSection>),    // 跨 turn 可缓存
    Dynamic(Vec<PromptSection>),   // session 级别
    PerTurn(Vec<Attachment>),      // 每轮注入
}
```

**优点**：最灵活，架构清晰
**缺点**：重构量最大

### 方向 D：Codex 风格三层分离

将当前的 dispatcher 阻塞 + 工具过滤拆分为独立的三层：
1. **Registry**（决定 schema）→ feature flag + mode 驱动
2. **Orchestrator**（决定执行）→ sandbox + approval + mode enforcement
3. **Fragments**（决定提示词）→ 模板化 + 动态注入

**优点**：关注点分离最清晰
**缺点**：与 FastClaw 当前的 dispatcher 一体化设计差距大

---

---

## 值得吸纳的具体设计模式

### 从 Claude Code 吸纳

#### 1. Per-Turn Mode Attachments（最高优先级）

**问题**：FastClaw 当前将 Plan 模式指令写在 `session_guidance_section()` 中（系统提示词的一部分），这有两个缺点：
- 系统提示词每轮都包含完整 Plan 指令（浪费 token）
- 无法做频率控制（如每 5 轮完整提醒，中间简短提醒）

**Claude Code 做法**：Plan 模式指令作为 **per-turn attachment** 注入到对话中，而非系统提示词：

```typescript
// 节流逻辑
TURNS_BETWEEN_ATTACHMENTS: 5,
FULL_REMINDER_EVERY_N_ATTACHMENTS: 5,

// 注入为 <system-reminder> 包裹的 user message（isMeta: true，UI 不可见）
// 完整提醒 ~2-4K tokens，简短提醒 ~200 tokens
// 第 1、6、11 轮完整提醒；2-5、7-10 简短提醒
```

**FastClaw 适配方案**：
```rust
struct ModeAttachment {
    mode: ExecutionMode,
    full_template: String,      // 完整指令模板
    sparse_template: String,    // 简短提醒模板
    turns_between: u32,         // 注入间隔（默认 5）
    full_every_n: u32,          // 完整提醒周期（默认 5）
}

// 在 execute_unified 的消息准备阶段注入
fn inject_mode_attachment(messages: &mut Vec<ChatMessage>, turn_count: u32, attachment: &ModeAttachment) {
    // ...
}
```

#### 2. 模式转换状态机

**Claude Code 做法**：集中的 `handlePlanModeTransition()` + 一次性标记：

```typescript
// 状态标记
needsPlanModeExitAttachment: bool   // plan → 非 plan 时设置
hasExitedPlanModeInSession: bool    // 用于 reentry 检测

// 转换处理
fn transition_mode(from, to) {
    if to == Plan && from != Plan: clear exit flag
    if from == Plan && to != Plan: set exit flag, set hasExited
}
```

**FastClaw 适配方案**：在 `ExecutionModeState` 中增加转换历史跟踪。

#### 3. Tool-Aware Prompt Sections

**Claude Code 做法**：`getUsingYourToolsSection(enabledTools)` 只提及当前可用的工具。如果 `Agent` 工具不在集合中，agent 相关的指引就不会出现在提示词里。

**FastClaw 适配**：`session_guidance_section()` 应该接收当前工具列表，条件性地包含或排除指导。

### 从 Codex CLI 吸纳

#### 4. ToolExposure 三级枚举（高优先级）

**当前 FastClaw**：二分法（eager / deferred）
**Codex 做法**：三级枚举 + 每个工具自描述

```rust
pub enum ToolExposure {
    Direct,          // 始终在模型工具列表中
    Deferred,        // 通过 tool_search 发现
    DirectModelOnly, // 模型可见但不嵌套（未来 code mode 需要）
}

// 每个工具自声明暴露级别
trait ToolExecutor {
    fn exposure(&self) -> ToolExposure { ToolExposure::Direct }
}

// 支持运行时覆盖（不需要复制工具）
fn override_tool_exposure(handler, exposure) -> WrappedHandler
```

**FastClaw 适配方案**：
1. 在 `Tool` trait 上增加 `fn exposure(&self) -> ToolExposure` 方法
2. 在 `ToolRegistry` 中用 exposure 替代 deferred HashSet
3. 支持运行时 override（如 Plan 模式下 exit_plan_mode → Direct）

#### 5. ContextualUserFragment Trait（中优先级）

**Codex 做法**：每个提示词片段是一个带标记的类型化 fragment：

```rust
trait ContextualUserFragment {
    const ROLE: &'static str;           // "developer" 或 "user"
    const START_MARKER: &'static str;   // "<permissions instructions>"
    const END_MARKER: &'static str;     // "</permissions instructions>"
    fn body(&self) -> String;
    fn render(&self) -> String;         // START + body + END
    fn into(self) -> ResponseItem;      // 包装为消息
}
```

**好处**：
- 历史过滤：可以识别哪些消息是注入的上下文（vs 真实用户消息）
- 增量更新：可以精确替换/更新特定 fragment
- 压缩感知：compact 时知道哪些消息可以丢弃

**FastClaw 适配**：现有的 `PromptSection` 已部分覆盖，但缺少 marker 系统和角色区分。

#### 6. Reference Context + Diff Pipeline（中优先级）

**Codex 做法**：

```
第 1 轮：build_initial_context() → 完整注入所有 fragment
第 N 轮：build_settings_update_items() → 仅注入变化的 fragment
         - 模式变化 → 注入 collaboration_mode 更新
         - 权限变化 → 注入 permissions 更新
         - CWD 变化 → 注入 environment 更新
```

对比的基准是 `reference_context_item`（上一轮的快照），确保 fork/resume 也能正确 diff。

**FastClaw 适配**：将模式切换时的提示词变更从"重新生成整个 section"改为"注入 diff 消息"。

---

## 综合推荐方案

### Phase 1：ToolExposure + Mode-Aware Promotion（紧急修复 + 短期）

**改动范围**：`fastclaw-core/src/tool.rs`, `fastclaw-agent/src/runtime/mod.rs`

1. 为 `Tool` trait 增加 `fn exposure(&self) -> ToolExposure` 方法
2. `ExitPlanModeTool` 自声明 `Deferred`，Plan 模式下自动提升为 `Direct`
3. 在 `execute_unified` 中增加 mode-aware 的工具提升逻辑
4. 移除 `deferred: HashSet<String>`，改用 `Tool::exposure()` 驱动

### Phase 2：Mode Attachments（中期）

**改动范围**：`fastclaw-agent/src/runtime/mod.rs`, 新增 `mode_attachments.rs`

1. 定义 `ModeAttachment` struct（模板 + 节流参数）
2. Plan 模式指令从 `session_guidance_section()` 移出
3. 每轮注入 attachment（完整/简短交替）
4. 增加模式转换状态跟踪（entry/exit/reentry 标记）

### Phase 3：ContextualFragment + Diff Pipeline（长期）

**改动范围**：新增 `fastclaw-agent/src/context/fragment.rs`

1. 定义 `ContextualFragment` trait（带 marker + role）
2. 重构现有 `PromptSection` 为 fragment
3. 实现 `reference_context` 快照 + diff 生成
4. 支持 fragment 级别的增量更新
