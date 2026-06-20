## MODIFIED Requirements

### Requirement: PlanFileStore 实例一致性
所有 Plan 模式相关代码 SHALL 使用同一个 PlanFileStore 实例（或等价配置），确保 plan 文件路径在 mode_attachment、dispatcher、end_turn、PlanArgInterceptor 之间一致。

#### Scenario: mode_attachment 和 dispatcher 使用相同路径
- **WHEN** mode_attachment 注入 plan 文件路径供模型参考
- **THEN** 该路径 SHALL 与 dispatcher 中 `is_plan_file_write` 检查使用的路径完全一致

#### Scenario: 自定义 plans 目录配置
- **WHEN** gateway 配置了自定义 plans 目录
- **THEN** 所有 PlanFileStore 调用 SHALL 使用该自定义目录，而非 `PlanFileStore::new(None)` 的默认目录

### Requirement: Plan 文件写入时工具结果简化
当 write_file 或 edit_file 工具写入 plan 文件时，chat 消息流中的工具结果展示 SHALL 简化为轻量提示。

#### Scenario: write_file plan 的工具结果展示
- **WHEN** write_file 工具成功写入 plan 文件路径
- **THEN** chat 消息流中 SHALL 显示简化的工具结果：「方案已更新」（或等效文案），而非完整文件内容
- **THEN** 简化结果 SHALL 包含可点击的「查看」链接，点击后打开/激活 PlanPanel

#### Scenario: 识别 plan 文件写入
- **WHEN** 前端渲染 write_file 或 edit_file 的工具结果
- **THEN** SHALL 检查 `file_path` 参数是否匹配当前 session 的 plan 文件路径（与 PlanFileStore 一致）
- **THEN** 匹配时使用简化展示，不匹配时使用默认展示

### Requirement: Plan 文件写入时自动显示 PlanPanel
当 plan 文件首次被写入且 PlanPanel 未打开时，SHALL 自动打开 PlanPanel。

#### Scenario: 首次 plan 写入自动打开面板
- **WHEN** 收到 plan_file_update 事件且 `exists: true`
- **AND** PlanPanel 当前未打开
- **AND** 此 session 中 PlanPanel 未被用户手动关闭过
- **THEN** SHALL 自动打开 PlanPanel（带 slideFromRight 动画）

#### Scenario: 用户手动关闭后不再自动打开
- **WHEN** 用户手动关闭了 PlanPanel（点击 X）
- **THEN** 后续该 session 中的 plan_file_update SHALL 不再自动打开 PlanPanel
