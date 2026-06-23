## ADDED Requirements

### Requirement: 底部多 Tab 面板容器
系统 SHALL 提供一个底部多 Tab 面板，包含 Agent、Console、Network 三个 Tab。

#### Scenario: Tab 切换
- **WHEN** 用户点击 Console Tab
- **THEN** 面板 SHALL 显示 Console 面板内容
- **THEN** 其他 Tab SHALL 显示为非激活状态

#### Scenario: 默认显示 Agent Tab
- **WHEN** 浏览器首次打开
- **THEN** 底部面板的 Agent Tab SHALL 为默认激活

### Requirement: 面板高度可拖拽调整
底部面板 SHALL 支持通过顶部拖拽条调整高度。

#### Scenario: 拖拽调整高度
- **WHEN** 用户拖拽面板顶部边缘
- **THEN** 面板高度 SHALL 跟随鼠标移动，范围 100px 到视口高度的 50%

#### Scenario: 折叠面板
- **WHEN** 用户双击面板顶部边缘或点击折叠按钮
- **THEN** 面板 SHALL 折叠到仅显示 Tab 栏（28px 高）
- **THEN** 再次双击或点击 SHALL 恢复到之前的高度

### Requirement: Error badge 通知
Console Tab SHALL 在有 console.error 消息时显示红色 badge（total count 模式，非 unread 模式）。

#### Scenario: Console Tab 非激活时显示 error badge
- **WHEN** 当前页面存在 console.error 消息且 Console Tab 非激活
- **THEN** Console Tab 标签 SHALL 显示红色 error 计数 badge

#### Scenario: Console Tab 激活时隐藏 badge
- **WHEN** 用户切换到 Console Tab
- **THEN** error badge SHALL 隐藏（用户已在查看 Console）
- **THEN** 切回 Agent/Network Tab 后若仍有 error，badge SHALL 重新出现

#### Scenario: 清空 Console 后 badge 消失
- **WHEN** 用户清空 Console
- **THEN** error 计数归零，badge SHALL 消失
