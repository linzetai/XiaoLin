## ADDED Requirements

### Requirement: Console 消息展示
系统 SHALL 在 DevTools Console Tab 中展示当前活动页面的 console 输出（log/warn/error/info/debug）。

#### Scenario: 页面 console.log 消息显示
- **WHEN** 浏览器页面内执行 `console.log("hello", "world")`
- **THEN** Console Tab SHALL 显示 `[时间] ℹ hello world`

#### Scenario: 页面 console.error 消息显示
- **WHEN** 浏览器页面内执行 `console.error("something failed")`
- **THEN** Console Tab SHALL 显示该消息并使用红色背景高亮
- **THEN** 若 Console Tab 当前非激活，其标签 SHALL 显示红色 error 计数 badge（详见 `browser-devtools-panel/spec.md` Error badge 需求）

#### Scenario: 切换标签页时过滤消息
- **WHEN** 用户从标签页 A 切换到标签页 B
- **THEN** Console Tab SHALL 仅显示标签页 B 的 console 消息

### Requirement: Console 级别过滤
用户 SHALL 能够按 console 级别过滤消息。

#### Scenario: 仅显示 Error 消息
- **WHEN** 用户点击 Errors 过滤按钮
- **THEN** Console Tab SHALL 仅显示 level 为 error 的消息

#### Scenario: 显示所有消息
- **WHEN** 用户点击 All 过滤按钮
- **THEN** Console Tab SHALL 显示所有级别的消息

### Requirement: Console 清空
用户 SHALL 能够清空当前页面的 console 消息。

#### Scenario: 清空 console
- **WHEN** 用户点击清空按钮
- **THEN** 当前页面的所有 console 消息 SHALL 被移除
- **THEN** 其他页面的 console 消息 SHALL 不受影响

### Requirement: Console 消息上限
系统 SHALL 限制每个页面最多保留 500 条 console 消息，超出时 FIFO 淘汰最旧消息。

#### Scenario: 超过 500 条消息
- **WHEN** 当前页面的 console 消息达到 500 条且有新消息到达
- **THEN** 最旧的消息 SHALL 被移除
- **THEN** 新消息 SHALL 被添加到列表末尾
