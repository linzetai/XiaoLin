## Context

FastClaw 当前使用 `ToolRegistry` 管理工具，采用二分法（eager / deferred + channel_scoped）。Plan 模式通过 `ToolDispatcher` 在运行时阻塞 Edit/Execute 类工具，同时允许计划文件写入作为例外。提示词通过 `PromptEngine` 的 `PromptSection` 动态组装。

竞品分析发现：
- **Claude Code**：保持完整工具 schema，通过 per-turn attachment + 运行时权限 + prompt 引导实现模式约束。子 agent 使用声明式 allow/deny 列表
- **Codex CLI**：`ToolExposure` 三级枚举（Direct/Deferred/DirectModelOnly），每个工具自描述暴露级别。三层分离：Registry（schema）/ Orchestrator（执行）/ Fragments（提示词）

FastClaw 已有部分基础设施：`SubAgentToolFilter`（声明式 allow/deny）、`AgentToolsConfig.profile`（字段存在但未实现）、`PromptSection`（动态提示词 section）。

## Goals / Non-Goals

**Goals:**
- 工具暴露根据运行时上下文（模式、agent 类型）动态调整，解决 exit_plan_mode 不可见问题
- Plan 模式指令从系统提示词迁移到节流式 attachment，减少 token 消耗
- 模式转换逻辑集中化，避免 entry/exit/reentry 状态错乱
- `AgentToolsConfig.profile` 字段实际生效

**Non-Goals:**
- 不重构 `ToolDispatcher` 的运行时阻塞机制（当前设计合理）
- 不实现 Codex 的 Code Mode / DirectModelOnly（暂无此需求）
- 不实现完整的 ContextualFragment trait + diff pipeline（留作 Phase 3）
- 不改变前端组件架构

## Decisions

### Decision 1：在 Tool trait 上增加 `exposure()` 方法

**选择**：每个工具通过 trait 方法自声明暴露级别

```rust
pub enum ToolExposure {
    Direct,    // 始终在 LLM 工具列表中
    Deferred,  // 通过 tool_search 发现
}

trait Tool {
    fn exposure(&self) -> ToolExposure { ToolExposure::Direct }  // 默认 Direct
}
```

**替代方案**：保持 ToolRegistry 的 `deferred: HashSet<String>` 集中管理

**理由**：自描述方式让工具的暴露意图在定义处可见，不依赖注册调用方的顺序。与 Codex 的设计一致。当前暂不引入 `DirectModelOnly`（FastClaw 无 Code Mode），保持二级枚举简洁性。

### Decision 2：Mode-Aware 工具提升通过 ToolProfile 实现

**选择**：引入 `ToolProfile` struct，为每种模式预定义 promote/demote 规则

```rust
pub struct ToolProfile {
    pub promote: Vec<String>,  // Deferred → Direct
    pub demote: Vec<String>,   // Direct → Deferred
}

impl ToolProfile {
    pub fn for_mode(mode: ExecutionMode) -> Self {
        match mode {
            ExecutionMode::Plan => Self {
                promote: vec!["exit_plan_mode".into()],
                demote: vec!["enter_plan_mode".into()],
            },
            ExecutionMode::Agent => Self::default(),
        }
    }
}
```

**替代方案 A**：在 `execute_unified` 中硬编码 `activate_deferred("exit_plan_mode")`
**替代方案 B**：让 ToolRegistry 感知 ExecutionMode，内部自动调整

**理由**：ToolProfile 是声明式的，可扩展到 agent-level profile（通过 `AgentToolsConfig.profile`），也可以组合使用。方案 A 太脆弱（每次加新模式工具都要改），方案 B 让 Registry 承担太多责任。

### Decision 3：Plan 模式指令迁移到 per-turn attachment

**选择**：创建 `ModeAttachment` 机制，在消息准备阶段注入模式指令

```rust
struct ModeAttachment {
    mode: ExecutionMode,
    full_template: String,
    sparse_template: String,
    turns_between: u32,       // 默认 5
    full_every_n: u32,        // 默认 5
}
```

注入位置：在 `execute_unified` 的每轮迭代中，根据节流策略决定是否注入、注入完整还是简短版本。

**替代方案**：保留在 `session_guidance_section()` 系统提示词中

**理由**：
- 系统提示词中的 Plan 指令每轮都占用 ~800 token（中文版），30 轮对话浪费 ~24K token
- 附件方式支持频率控制：首轮完整（~800 token）、后续简短（~100 token）
- 与 Claude Code / Codex 的做法一致
- 系统提示词更稳定，有利于 prompt cache（如果未来接入支持的 LLM）

### Decision 4：模式转换状态集中管理

**选择**：在 `ExecutionModeState` 中增加转换历史跟踪

```rust
pub struct ExecutionModeState {
    current: AtomicU8,
    // 新增
    last_transition: AtomicU64,       // timestamp
    has_exited_plan: AtomicBool,      // 用于 reentry 检测
    plan_turn_counter: AtomicU32,     // attachment 节流计数
}
```

**理由**：当前模式转换逻辑分散在 `EnterPlanModeTool::execute()`、`ExitPlanModeTool::execute()`、`dispatcher`、`dynamic.rs` 四处。集中跟踪状态可以：
- 防止重复 attachment 注入
- 支持 reentry 检测（plan → agent → plan）
- 为前端提供转换事件（已有 `AgentEvent::ModeChange`）

### Decision 5：ToolRegistry 兼容性

**选择**：保留 `register_deferred()` API 作为便捷方法，内部改为设置 `exposure` 属性

```rust
impl ToolRegistry {
    pub fn register_deferred(&self, tool: Arc<dyn Tool>) {
        // 内部仍然使用 deferred set，但在 definitions_with_profile 中统一处理
        self.register(tool);
        // ...
    }

    pub fn definitions_with_profile(&self, profile: &ToolProfile) -> Vec<ToolDefinition> {
        // 基于 Tool::exposure() + profile 的 promote/demote 规则过滤
    }
}
```

**理由**：保持向后兼容。现有的 `register_deferred()` 调用不需要修改，同时新代码可以直接使用 `Tool::exposure()` 方式。

## Risks / Trade-offs

- **[附件 token 节省 vs 指令遗忘]** → 简短提醒仍包含核心约束；完整提醒每 5 轮重复一次，确保 LLM 不会遗忘
- **[ToolProfile 增加复杂度]** → Profile 是 opt-in 的，默认 profile 为空（不修改 exposure），只在需要时使用
- **[向后兼容]** → `register_deferred()` API 保留，现有代码无需改动。新机制在不使用 profile 时行为完全等价
- **[节流参数调优]** → 初始值参考 Claude Code（5 轮 / 5 次），后续可通过配置调整
