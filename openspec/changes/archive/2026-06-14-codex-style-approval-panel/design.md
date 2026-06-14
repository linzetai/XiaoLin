## Context

XiaoLin 的权限审批机制已实现基础功能：后端 orchestrator 在执行需审批的工具调用前 emit `ApprovalRequired` 事件，前端渲染 `ApprovalCard` 供用户决策。

当前状态：
- `ApprovalRequired` 事件只包含 `PendingAction`（工具类型+基本参数）、`reason`、`available_decisions`
- `PendingAction::FileWrite` 只有 `path`，无文件内容
- `PendingAction::ApplyPatch` 只有 `paths`，无 diff
- 不携带风险等级，前端尝试读取 `d.risk_level` 但后端从未填充，始终 fallback 为 "caution"
- 前端 `ApprovalCard` 使用彩色边框+Shield图标+隐藏式预览面板，与已完成的 Codex 风格极简 UI 不一致

已完成的前置工作：ReasoningBlock、PhaseIndicator、StepIndicator、ExploringBlock 均已重构为 Codex 风格。

## Goals / Non-Goals

**Goals:**
- 后端 `ApprovalRequired` 事件提供足够上下文：risk_level + 文件内容/diff 预览
- 前端审批面板视觉风格与 Codex 保持一致：极简、无边框、文本优先
- 添加键盘快捷键，提升审批效率
- 向后兼容：新字段均为 Option，旧客户端不受影响

**Non-Goals:**
- 不改变 `ApprovalCache` 的缓存粒度（当前按 tool-type 级别缓存）
- 不引入 Guardian LLM 对每次审批做 AI 风险评估（当前只用规则推断）
- 不修改 `InteractionHandle` 通道机制
- 不修改 `xiaolin-execpolicy` 的 `amend.rs` 核心实现（已满足需求）

## Decisions

### 1. 风险等级用规则推断而非 LLM

**选择**: 在 orchestrator 中根据 `PendingAction` 内容做基于规则的风险分类

**备选方案**:
- (A) 每次审批前调用 Guardian LLM 评估风险 → 延迟高（2-5s），成本高
- (B) 使用已有的 `RiskLevel` 枚举（Low/Medium/High/Critical 4级）→ Critical 对用户无意义

**理由**: 审批是同步阻塞的，增加 LLM 调用会让用户等待更久。规则推断足以覆盖常见场景（rm、sudo、workspace 外写入 = High），且零额外延迟。

### 2. 新建 `ActionRiskLevel` 而非复用 `RiskLevel`

**选择**: 新增 3 级枚举 `ActionRiskLevel { Low, Medium, High }`

**理由**: Guardian 的 `RiskLevel` 是 LLM 评估的 4 级标度（含 Critical），语义不同。审批场景只需 3 级：Low（只读/安全已知）、Medium（workspace 内写入）、High（危险操作）。解耦后两套系统可独立演化。

### 3. content/diff 截断策略

**选择**: 截断到 2000 chars，使用 `floor_char_boundary` 确保 UTF-8 安全

**理由**: WebSocket 单帧不宜过大（64KB 通常上限），审批预览只需让用户判断意图，不需要完整内容。2000 chars 约覆盖 50-80 行代码，足够判断。

### 4. 键盘快捷键方案

**选择**: 组件内 `useEffect` 注册 window keydown listener，挂载时注册/卸载时清理

**备选方案**:
- (A) 全局 Tauri 快捷键 → 与系统快捷键冲突风险高
- (B) 第三方库（react-hotkeys-hook）→ 增加依赖

**理由**: 审批面板显示时间短且独占交互焦点，简单的 window listener 足够，无需引入额外依赖。

### 5. 前端 risk_level 字段映射

**选择**: 直接使用后端 `ActionRiskLevel` 的蛇形命名 (`low`/`medium`/`high`)，前端用 switch 映射到竖线颜色

**理由**: 消除前端当前的 `"danger"/"caution"/"safe"` 三值系统，统一使用后端定义。ApprovalData 接口中 `riskLevel` 类型改为 `"low" | "medium" | "high"`。

### 6. ExecPolicy 持久化（"记住命令前缀"）

**选择**: 复用已有的 `xiaolin-execpolicy::amend::blocking_append_allow_prefix_rule`，在 orchestrator 处理 `ApprovedWithPolicyAmend` 决策时调用

**已有基础设施**:
- `PolicyEngine::add_session_rule()` — 会话级规则立即生效
- `amend::blocking_append_allow_prefix_rule(policy_path, prefix)` — 持久化到项目 `.xiaolin/exec_policy.toml`
- Orchestrator 在审批前已调用 `policy.evaluate()` 检查规则

**实现路径**:
1. `ApprovalDecision` 新增 `ApprovedWithPolicyAmend { prefix: Vec<String> }` 变体
2. Orchestrator 构建 `available_decisions` 时，直接将提取的 prefix 填入该变体（prefix 随选项一起传递，无需额外事件字段）
3. Orchestrator 收到此决策后:
   - 调用 `policy.add_session_rule()` 立即生效（本次会话不再询问）
   - 调用 `spawn_blocking(|| blocking_append_allow_prefix_rule(...))` 持久化到磁盘
4. 前端从 `available_decisions` 中识别该变体，提取 prefix 展示为按钮标签；用户点击后将整个变体（含 prefix）回传后端

**备选方案**:
- (A) 前端手动编辑前缀再提交 → 增加交互复杂度，暂不实现
- (B) 后端自动推断前缀 → 更简单，但用户无法控制粒度

**理由**: 对 shell 命令使用第一个 token（如 `npm`、`cargo`、`git`）作为前缀是合理默认值。Codex 也是自动提取命令名作为 prefix。后续可扩展为用户可编辑。

## Risks / Trade-offs

- **[Risk] 规则推断覆盖不全** → Mitigation: 默认为 Medium，未匹配的命令不会被低估；后续可扩展规则或接入 Guardian
- **[Risk] content 截断丢失关键信息** → Mitigation: 显示 "(truncated)" 提示，用户可查看完整 StepIndicator 展开内容
- **[Risk] 键盘快捷键与输入框冲突** → Mitigation: 审批面板显示时无活跃输入框（发送按钮被审批替代），且 listener 检查 `event.target` 是否为 input/textarea
- **[Risk] `PendingAction` 序列化体积增大** → Mitigation: content/diff 为 Option + skip_serializing_if，无内容时零开销
- **[Risk] ExecPolicy 持久化文件冲突** → Mitigation: `amend.rs` 已使用 advisory file lock (`fs2::FileExt::lock_exclusive`)，支持并发写入
- **[Risk] 用户误记住危险前缀（如 `rm`）** → Mitigation: High 风险命令不展示"记住前缀"选项；只对 Medium 风险的 ShellCommand 提供此决策
