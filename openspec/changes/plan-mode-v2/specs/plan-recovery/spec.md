## ADDED Requirements

### Requirement: Session 激活时 Plan 元数据 Hydrate
当前端激活一个 session（切换、刷新后首次加载、WS 重连后同步）时，SHALL 主动查询该 session 的 plan 元数据并恢复 UI 状态。

#### Scenario: Session 切换时 hydrate plan 元数据
- **WHEN** 前端通过 activateSession(sessionId) 切换到一个 session
- **THEN** SHALL 调用 `execution.get_plan_meta` RPC 获取 { planFilePath, planFileExists, executionMode }
- **THEN** SHALL 更新 ChatMeta 中的 planFilePath、planFileExists、executionMode
- **THEN** 如果 planFileExists && executionMode === "plan" → PlanPanel 自动展开 + Plan Banner 显示

#### Scenario: 页面刷新后恢复
- **WHEN** 用户刷新页面且有活跃 session
- **THEN** 在 syncBackendData 后 SHALL 对当前活跃 session 调用 execution.get_plan_meta
- **THEN** SHALL 恢复 executionMode 和 planFileExists 状态
- **THEN** 不再硬编码 `executionMode: "agent"` for 所有 session

#### Scenario: WS 重连后恢复
- **WHEN** WS 断线重连成功（emit "reconnected"）
- **THEN** SHALL 对当前活跃 session 调用 execution.get_plan_meta
- **THEN** SHALL 恢复因断线丢失的模式状态

### Requirement: execution.get_plan_meta RPC
后端 SHALL 提供 `execution.get_plan_meta` RPC 端点，允许前端查询任意 session 的 plan 和 mode 状态。

#### Scenario: 正常查询
- **WHEN** 前端调用 `execution.get_plan_meta` with session_id
- **THEN** SHALL 返回:
  - `plan_file_path`: plan 文件的完整路径（如果 session 有 slug）
  - `plan_file_exists`: plan 文件是否存在且非空
  - `execution_mode`: 当前执行模式（"plan" / "agent" / "goal"）
- **THEN** execution_mode 推断逻辑:
  1. SessionModeRegistry 有记录 → 使用注册值
  2. Registry 无记录但 plan 文件存在 + 最后一条 assistant 消息含 plan 相关工具调用 → "plan"
  3. 否则 → "agent"

#### Scenario: Session 无 plan
- **WHEN** session_id 在 plan-index.json 中无映射
- **THEN** SHALL 返回 { plan_file_path: null, plan_file_exists: false, execution_mode: "agent" }

#### Scenario: Plan 文件被外部删除
- **WHEN** session_id 有 slug 但文件不存在
- **THEN** SHALL 返回 { plan_file_path: path, plan_file_exists: false, execution_mode: ... }
- **THEN** execution_mode 按推断逻辑确定（文件不存在时不推断为 plan）

### Requirement: PlanPanel 自动恢复与提示
PlanPanel 在 session 激活后 SHALL 根据 plan 存在状态和 executionMode 自动恢复或显示提示。

#### Scenario: Plan 模式恢复 → PlanPanel 自动展开
- **WHEN** hydrate 结果为 planFileExists === true && executionMode === "plan"
- **THEN** PlanPanel SHALL 自动展开（slideFromRight 动画）
- **THEN** Plan Banner SHALL 显示

#### Scenario: Agent 模式 + 有未完成方案 → 显示提示
- **WHEN** hydrate 结果为 planFileExists === true && executionMode === "agent"
- **THEN** PlanPanel 不自动展开
- **THEN** SHALL 在消息流顶部显示一个小提示卡片: "📋 此会话有一个未完成的规划方案" + [查看方案] 按钮
- **THEN** 点击按钮展开 PlanPanel

#### Scenario: 无 plan → 不显示任何提示
- **WHEN** hydrate 结果为 planFileExists === false
- **THEN** PlanPanel 不展开，不显示提示

#### Scenario: 用户手动关闭 PlanPanel 后不自动重开
- **WHEN** 用户手动关闭了 PlanPanel
- **AND** 后续同一 session 内未产生新的 plan_file_update
- **THEN** SHALL 不自动重开 PlanPanel（尊重用户意图）

### Requirement: Plan Mode Reentry Attachment
当 session 恢复到 Plan 模式且之前已有 plan 内容时，SHALL 在下一次 LLM 调用注入 `plan_mode_reentry` attachment。

#### Scenario: 恢复后首次 LLM 调用注入 reentry
- **WHEN** session 进入 Plan 模式（通过恢复推断或 enter_plan_mode 工具）
- **AND** plan 文件已存在且非空
- **AND** 本次 Plan 模式进入是「重入」而非首次进入
- **THEN** mode_attachments SHALL 注入 plan_mode_reentry attachment:
  ```
  ## 重新进入规划模式

  你之前为此任务创建了一个规划方案。方案文件位于: {plan_path}

  请注意:
  - 如果用户是继续之前的讨论，请基于已有方案进行修改，不要从头规划
  - 如果用户有新的需求，请更新方案中的相关章节
  - 方案中已有的正确内容不需要重写

  当前方案内容:
  {plan_content}
  ```

#### Scenario: 首次进入不注入 reentry
- **WHEN** session 进入 Plan 模式
- **AND** plan 文件不存在或为空（首次规划）
- **THEN** SHALL 不注入 plan_mode_reentry，使用标准的 plan_full attachment

#### Scenario: 仅首次 LLM 调用注入
- **WHEN** reentry attachment 已注入一次
- **THEN** 后续调用 SHALL 使用标准的 plan_full / plan_sparse 循环，不再注入 reentry

### Requirement: Session 删除时 Plan 文件清理
当 session 被删除时，SHALL 清理关联的 plan 文件和索引记录。

#### Scenario: 删除 session 清理 plan
- **WHEN** 前端调用 `sessions.delete` 且 session 有关联的 plan slug
- **THEN** SHALL 删除 `~/.xiaolin/plans/{slug}.md` 文件
- **THEN** SHALL 从 `.plan-index.json` 中移除该 session_id 的映射
- **THEN** SHALL 从 SessionModeRegistry 中移除该 session

#### Scenario: Plan 文件已被外部删除
- **WHEN** session 删除时 plan 文件不存在
- **THEN** SHALL 静默跳过文件删除，仅清理索引和 registry

#### Scenario: 批量删除
- **WHEN** 多个 session 被批量删除
- **THEN** SHALL 对每个 session 执行 plan 清理（不因单个失败阻塞其他）

### Requirement: syncSessionsForAgent 不再硬编码 executionMode
`syncSessionsForAgent` 在恢复 session 列表时 SHALL 不再将所有 session 的 executionMode 硬编码为 "agent"。

#### Scenario: 保留已知 mode
- **WHEN** ChatMeta 中已有某 session 的 executionMode（通过 hydrate 或 WS 事件设置）
- **THEN** syncSessionsForAgent SHALL 保留该值，不覆盖

#### Scenario: 新 session 默认 agent
- **WHEN** session 是新加入列表（之前不在 ChatMeta 中）
- **THEN** SHALL 初始化 executionMode 为 "agent"（直到 hydrate 覆盖）

## Implementation Reference

### 竞品对标

| 能力 | Codex | Claude Code | XiaoLin (目标) |
|------|-------|-------------|----------------|
| Plan 文件磁盘持久化 | ❌ (XML 在对话中) | ✅ slug file | ✅ 已有 |
| Session 恢复时 mode 恢复 | ❌ | ✅ recoverPlanFromMessages | ✅ get_plan_meta RPC |
| Plan ↔ Session 绑定 | ❌ | ✅ slug 系统 | ✅ plan-index.json |
| 崩溃后多层 fallback | ❌ | ✅ file → snapshot → messages | ✅ file → message inference |
| Reentry attachment | ❌ | ✅ plan_mode_reentry | ✅ |
| PlanPanel 自动恢复 | N/A (TUI) | N/A (CLI) | ✅ GUI 优势 |
| 未完成方案提示 | ❌ | ❌ | ✅ 差异化 |
| Session 删除清理 | ❌ (无文件) | ❌ (orphan files) | ✅ |

### 技术实现要点

**1. execution.get_plan_meta RPC (backend)**

```rust
// gateway/ws/execution.rs
async fn handle_execution_get_plan_meta(
    state: &AppState,
    session_id: &str,
) -> Result<PlanMeta> {
    let plan_store = &state.rt.plan_file_store;
    let mode_registry = &state.rt.session_mode_registry;

    let (plan_file_path, plan_file_exists) = if plan_store.has_slug(session_id) {
        let path = plan_store.plan_path(session_id);
        let exists = path.exists() && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false);
        (Some(path.to_string_lossy().to_string()), exists)
    } else {
        (None, false)
    };

    let execution_mode = match mode_registry.get(session_id) {
        Some(mode) => mode.to_string(),
        None => {
            if plan_file_exists {
                infer_mode_from_last_message(state, session_id).await
            } else {
                "agent".to_string()
            }
        }
    };

    Ok(PlanMeta { plan_file_path, plan_file_exists, execution_mode })
}
```

**2. 前端 hydrate 集成**

```typescript
// store.ts 的 syncBackendData 或 activateSession 中
async function hydratePlanMeta(sessionId: string) {
  const meta = await transport.call("execution.get_plan_meta", { session_id: sessionId });
  if (meta) {
    useChatMetaStore.getState().setChatPlanFile(sessionId, meta.plan_file_path, meta.plan_file_exists);
    if (meta.execution_mode !== "agent") {
      useChatMetaStore.getState().setChatExecutionMode(sessionId, meta.execution_mode);
    }
  }
}
```

**3. Session 删除清理**

```rust
// gateway/ws/session.rs handle_sessions_delete 追加
if let Some(slug) = plan_file_store.get_slug(session_id) {
    let path = plan_file_store.plan_path(session_id);
    let _ = std::fs::remove_file(&path);
    plan_file_store.remove_slug(session_id);
    tracing::debug!(session_id, slug, "cleaned up plan file on session delete");
}
mode_registry.remove(session_id);
```

**4. Mode 推断逻辑**

```rust
async fn infer_mode_from_last_message(state: &AppState, session_id: &str) -> String {
    let messages = state.store.session_store
        .get_recent_messages(session_id, 3).await;
    
    for msg in messages.iter().rev() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    match tc.name.as_str() {
                        "enter_plan_mode" => return "plan".to_string(),
                        "exit_plan_mode" => return "agent".to_string(),
                        _ => {}
                    }
                }
            }
        }
    }
    "agent".to_string()
}
```
