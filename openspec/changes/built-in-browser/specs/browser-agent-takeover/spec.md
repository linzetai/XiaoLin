## ADDED Requirements

### Requirement: 操作模式管理
系统 SHALL 维护 per-page 的操作模式状态，支持 Free Mode、Agent Control Mode、User Takeover 三种模式。

#### Scenario: 默认 Free Mode
- **WHEN** 没有 Agent 正在执行 browser 操作
- **THEN** 页面处于 Free Mode，用户完全控制

#### Scenario: 进入 Agent Control Mode
- **WHEN** Agent 调用 browser tool action（navigate、click、fill 等）
- **THEN** 目标页面进入 Agent Control Mode
- **AND** 页面标签显示 🤖 前缀
- **AND** 地址栏显示 "Agent 操作中" 状态条
- **AND** 页面叠加半透明蓝色遮罩（pointer-events:none，不阻止滚动）

#### Scenario: Agent 操作完成退出
- **WHEN** Agent 的 browser action 执行完成
- **AND** 500ms 内没有新的 Agent action
- **THEN** 退出 Agent Control Mode，恢复 Free Mode

#### Scenario: 连续 Agent 操作防闪烁
- **WHEN** Agent 连续执行多个 action（如 fill → fill → click）
- **THEN** 两个 action 之间不退出 Agent Control Mode（500ms 防闪烁延迟）

### Requirement: Agent Control 期间用户操作控制
系统 SHALL 在 Agent Control Mode 下限制用户的破坏性操作，但允许非破坏性操作。

#### Scenario: 允许滚动
- **WHEN** Agent Control Mode 下用户滚动页面
- **THEN** 滚动正常执行

#### Scenario: 允许选中文本
- **WHEN** Agent Control Mode 下用户选中页面文本
- **THEN** 选中正常执行

#### Scenario: 允许切换 browser tab
- **WHEN** Agent Control Mode 下用户切换到其他 browser tab
- **THEN** 切换正常执行，Agent 操作在原 tab 继续

#### Scenario: 拦截点击
- **WHEN** Agent Control Mode 下用户点击页面中的链接或按钮
- **THEN** 点击被拦截（capture phase preventDefault）
- **AND** 弹出 toast 提示 "Agent 正在操作此页面" + [中止 Agent] 按钮

#### Scenario: 拦截输入
- **WHEN** Agent Control Mode 下用户在页面输入框中输入
- **THEN** 输入被拦截
- **AND** 弹出 toast 提示

### Requirement: 用户接管
系统 SHALL 允许用户随时中止 Agent 的 browser 操作并取回控制权。

#### Scenario: 点击"取回控制"按钮
- **WHEN** 用户点击地址栏的 [取回控制] 按钮
- **THEN** Agent 当前 action 返回错误 `{ error: "user_takeover", message: "user interrupted operation" }`
- **AND** 页面退出 Agent Control Mode
- **AND** 用户获得完整控制权

#### Scenario: 确认中止 Agent
- **WHEN** 用户在 toast 中点击 [中止 Agent] 按钮
- **THEN** 效果与"取回控制"相同

#### Scenario: per-page 隔离
- **WHEN** Page A 在 Agent Control Mode，Page B 在 Free Mode
- **THEN** 用户可以自由操作 Page B，不影响 Page A 的 Agent 操作

### Requirement: Agent 操作可视化
系统 SHALL 在 Agent 执行交互操作前可视化高亮目标元素。

#### Scenario: click 操作高亮
- **WHEN** Agent 执行 click 操作
- **THEN** 目标元素被橙色脉冲边框高亮 300ms
- **THEN** 执行点击后元素绿色闪烁表示完成

#### Scenario: fill 操作高亮
- **WHEN** Agent 执行 fill 操作
- **THEN** 目标 input 被橙色边框高亮
- **THEN** 输入过程中高亮保持
- **THEN** 完成后绿色闪烁

#### Scenario: 操作日志实时更新
- **WHEN** Agent 执行任何 browser action
- **THEN** Browser Panel 底部的操作日志面板实时显示：时间戳 + 操作类型 + 目标描述
