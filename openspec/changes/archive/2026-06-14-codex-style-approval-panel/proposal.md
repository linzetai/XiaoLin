## Why

当前审批面板（ApprovalCard）存在两个核心问题：

1. **信息不足**：后端 `ApprovalRequired` 事件不携带 `risk_level`，`PendingAction` 不包含文件内容/diff，用户做决策时缺乏关键上下文。
2. **UI 笨重**：大面积彩色背景 + Shield 图标 + 隐藏式命令预览，与已完成的 Codex 风格极简 UI 格格不入。

参考 Codex app 的审批设计（意图优先、命令始终可见、键盘快捷键、结构化决策列表），需要前后端协同改造。

## What Changes

- 后端 `PendingAction` 枚举扩展：`FileWrite` 增加 `content`，`ApplyPatch` 增加 `diff`（均为 Option，截断到 2000 chars）
- 新增 `ActionRiskLevel` 枚举（Low/Medium/High），后端规则推断并填入 `ApprovalRequired` 事件
- `ApprovalRequired` 事件增加 `risk_level: Option<ActionRiskLevel>` 字段
- 前端 `ApprovalCard` 完全重写：去掉彩色边框/背景/Shield图标，改为左竖线风险色 + 意图标题 + 始终展示命令 + 列表式决策按钮
- 前端添加键盘快捷键支持（y/s/n/a/p 直接触发决策）
- 新增 `ApprovalDecision::ApprovedWithPolicyAmend` 变体，用户选择"记住此命令前缀"时持久化到 ExecPolicy 文件
- 后端处理 `ApprovedWithPolicyAmend` 时调用已有的 `xiaolin-execpolicy::amend::blocking_append_allow_prefix_rule` 持久化规则

## Capabilities

### New Capabilities
- `action-risk-assessment`: 后端根据 PendingAction 内容规则推断风险等级（Low/Medium/High），填充到 ApprovalRequired 事件
- `approval-content-preview`: 后端在 PendingAction 中携带文件内容/diff 预览数据，供前端展示
- `policy-amend-on-approval`: 用户审批时可选择"记住此命令前缀"，后端将命令前缀持久化到 ExecPolicy 文件，后续相同前缀命令自动放行

### Modified Capabilities
- `approval-ui`: 前端审批面板重写为 Codex 风格极简布局，添加键盘导航，增加"记住前缀"选项

## Impact

- **协议层** (`xiaolin-protocol`): `PendingAction` 枚举和 `AgentEvent::ApprovalRequired` 结构变更；`ApprovalDecision` 新增 `ApprovedWithPolicyAmend` 变体
- **Agent 层** (`xiaolin-agent`): orchestrator 计算 risk_level、file runtime 填充 content/diff；处理 `ApprovedWithPolicyAmend` 时调用 `xiaolin-execpolicy::amend`
- **ExecPolicy 层** (`xiaolin-execpolicy`): 已有 `blocking_append_allow_prefix_rule`，无需修改，仅被调用
- **前端** (`xiaolin-app`): ApprovalCard 组件完全重写，useMessageStreamChat 适配新字段，新增"记住前缀"决策选项
- **序列化**: 新字段使用 `skip_serializing_if = "Option::is_none"`，不影响旧客户端
