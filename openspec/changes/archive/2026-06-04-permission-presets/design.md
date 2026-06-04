## Context

当前权限系统的层次：

1. **AgentConfig.behavior** (`BehaviorConfig`)：全局默认，从 `config.toml` 加载
2. **PermissionRuleEngine**：Session/Global 两级规则链，支持 Exact/Prefix/Wildcard 匹配
3. **DenialTracker**：记录用户拒绝历史，避免重复询问
4. **ExecApprovalRequirement**：工具级审批决策（Skip/NeedsApproval/Forbidden）
5. **SecurityTab**（前端）：4 个执行模式（plan/default/auto-edit/yolo），映射到 tools_ask/deny 组合

前端 `SecurityTab` 已有模式→配置映射的概念（`inferMode` / `applyMode`），但需要 Settings 页面操作且修改全局配置。原型图需要的是在 InputBar 中快速切换的能力。

依赖关系：
- 依赖 `layout-overhaul` 的 InputBar 新布局
- 可与 `project-model` 结合：per-project 默认权限预设

## Goals / Non-Goals

**Goals:**
- 用户可通过 InputBar 一键切换权限策略，无需离开对话页面
- 预设覆盖粒度为 session 级（同一用户不同 session 可用不同策略）
- 内置 3-4 个标准预设，覆盖常见场景
- 支持用户自定义预设（高级）
- 权限变更实时生效，无需重启 session

**Non-Goals:**
- 不做细粒度的单工具权限 UI（如勾选列表）——保持预设级别的简洁
- 不做 per-turn 权限（如"这一轮自动批准"）——预设在 session 粒度生效
- 不修改后端 `PermissionRuleEngine` 核心逻辑——只在上层映射

## Decisions

### D1: 预设定义与 BehaviorConfig 映射

**决策**：定义 `PermissionPreset` 结构体，每个预设映射到 `BehaviorConfig` 的一组参数。

| 预设 | approval_strategy | file_access | tools_ask | tools_deny | 说明 |
|------|------------------|-------------|-----------|------------|------|
| `suggest` | interactive | workspace | ["write_file", "edit_file", "shell_exec", "mcp_*"] | [] | 所有写操作需确认（默认） |
| `auto-edit` | interactive | workspace | ["shell_exec", "mcp_*"] | [] | 文件编辑自动批准，shell 仍需确认 |
| `full-auto` | auto_approve | full | [] | [] | 完全自动（YOLO 模式） |
| `plan-only` | interactive | workspace | [] | ["write_file", "edit_file", "shell_exec"] | 只读+规划，禁止所有写操作 |

**理由**：与现有 `SecurityTab` 的 4 个模式对齐，但使用更友好的名称。
**替代方案**：让用户手动组合参数。否决，因为 BehaviorConfig 参数过多，组合爆炸不利于快速切换。

### D2: Per-session 覆盖机制

**决策**：在 `SessionHandle` 层面增加 `permission_override: Option<PermissionPreset>`。

- 如果 session 有 override → 用 override 的 BehaviorConfig
- 如果无 override → 用全局 AgentConfig.behavior
- override 存储在内存中（不持久化到 SQLite），session 关闭后重置

**理由**：session 级别最直观（用户在不同对话中可能有不同安全需求）。不持久化是因为权限策略应该有意识地选择，默认安全。
**替代方案**：per-project 持久化。可在 project-model 集成后作为增强，但初始版本用 session 级更安全。

### D3: 运行时 BehaviorConfig 动态切换

**决策**：在 `ShellRuntime`/`ToolOrchestrator` 的审批路径中，不直接读取 `AgentConfig.behavior`，改为通过 `PermissionResolver` trait 获取当前 session 的有效 BehaviorConfig。

```
PermissionResolver.resolve(session_id) → BehaviorConfig
  = session_override.unwrap_or(global_config.behavior)
```

**理由**：最小改动。现有审批路径只需替换 BehaviorConfig 的来源，不需要改变 `ExecApprovalRequirement` / `PermissionRuleEngine` 的评估逻辑。

### D4: 权限选择器 UI 设计

**决策**：InputBar 工具栏中增加 `PermissionSelector` 组件。

外观：`🔒 Suggest edits ▾`
点击弹出下拉：
```
  ✓ Suggest edits      — 所有写操作需确认
    Auto edit           — 文件自动编辑，shell 需确认
    Full auto           — 完全自动
    Plan only           — 只读规划模式
    ─────────────────
    自定义...           → 打开 SecurityTab
```

每个选项有名称 + 一句描述。当前选中项有 ✓ 标记。

**理由**：与原型图 "Default permissions ▾" 一致。简洁的下拉比弹窗配置更快。

### D5: WS API 设计

**决策**：复用现有 WS 通道，新增两个方法：

- `permissions.get { session_id }` → 返回当前 session 有效权限预设 + 可用预设列表
- `permissions.set { session_id, preset_id }` → 设置 session 权限预设，立即生效

**理由**：轻量，与现有 WS 协议一致。

## Risks / Trade-offs

**[R1] 运行中切换权限可能导致 Agent 行为不一致** → 切换时如果有正在执行的 turn，当前 turn 继续使用旧权限，下一个 turn 才用新权限。前端提示 "权限将在下一轮对话生效"。

**[R2] Full-auto 模式的安全风险** → 选择 "Full auto" 时弹确认对话框："此模式将跳过所有安全确认，确定继续？"。前端在 Full-auto 模式下 InputBar 显示橙色警告边框。

**[R3] 预设不覆盖所有 BehaviorConfig 参数** → 预设只映射安全相关参数（approval_strategy、file_access、tools_ask、tools_deny）。其他参数（max_tool_calls_per_turn、streaming_tool_execution 等）不受预设影响，保持全局配置。

**[R4] 与 SecurityTab 的一致性** → PermissionSelector 和 SecurityTab 操作同一个概念但不同粒度。SecurityTab 修改全局默认，PermissionSelector 修改 session 覆盖。两者需要保持同步：SecurityTab 显示当前全局预设，PermissionSelector 显示 session 有效预设（可能是覆盖或全局默认）。
