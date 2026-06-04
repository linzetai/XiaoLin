## MODIFIED Requirements

### Requirement: Approval card with operation details
审批卡片 SHALL 在现有 approve/deny 按钮基础上，增加操作详情展示区域，包含操作类型、目标路径/命令、风险等级标记。

#### Scenario: Shell command approval
- **WHEN** Agent 请求执行 shell 命令需要审批
- **THEN** 审批卡片显示：
  - 操作类型标签："Shell 命令"（带终端图标）
  - 命令内容：monospace 预览（如 `rm -rf node_modules`）
  - 风险等级：安全（绿色）/ 注意（黄色）/ 危险（红色）
  - 按钮组："批准" / "本次全部批准" / "拒绝"

#### Scenario: File write approval
- **WHEN** Agent 请求写入文件需要审批
- **THEN** 审批卡片显示：
  - 操作类型标签："文件写入"（带文件图标）
  - 目标路径：如 `src/components/App.tsx`
  - 变更预览：新增/修改行数统计
  - 按钮组："批准" / "本次全部批准" / "拒绝"

#### Scenario: Risk level visual encoding
- **WHEN** 审批请求的 risk_level = "danger"
- **THEN** 审批卡片左侧边框为红色
- **AND** 风险标签显示红色 "⚠ 危险"
- **WHEN** risk_level = "caution"
- **THEN** 左侧边框为黄色，标签显示 "⚡ 注意"
- **WHEN** risk_level = "safe"
- **THEN** 左侧边框为绿色，标签显示 "✓ 安全"

### Requirement: Batch approval option
审批卡片 SHALL 提供"本次全部批准"选项，自动批准当前 turn 中同类型的后续操作。

#### Scenario: Approve all for session
- **WHEN** 用户点击 "本次全部批准"
- **THEN** 当前 turn 中同一工具的后续调用自动批准
- **AND** 审批卡片显示 "已设置本次自动批准 {tool_name}"
- **AND** 下一个 turn 恢复正常审批流程
