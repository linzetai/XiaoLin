## ADDED Requirements

### Requirement: Chat 链接在内置浏览器打开
系统 SHALL 将 Chat 消息中的 http/https 链接的点击行为从外部浏览器改为内置浏览器。

#### Scenario: 普通点击
- **WHEN** 用户点击 Chat 消息中的 https 链接
- **THEN** 在内置浏览器 Browser Panel 中打开该 URL，自动切换到 Browser Tab

#### Scenario: Shift+Click 外部打开
- **WHEN** 用户按住 Shift 键点击 Chat 消息中的链接
- **THEN** 在系统默认浏览器中打开该 URL（保留 escape hatch）

#### Scenario: 非 HTTP 链接
- **WHEN** 用户点击 mailto: 或 file: 等非 HTTP 链接
- **THEN** 使用系统默认处理方式（不在内置浏览器打开）

### Requirement: 链接打开行为可配置
系统 SHALL 支持用户配置 Chat 链接的默认打开方式。

#### Scenario: 配置为"总是内置浏览器"（默认）
- **WHEN** 用户未修改配置（或选择"内置浏览器"）
- **THEN** Chat 中的 http/https 链接点击后在内置浏览器打开

#### Scenario: 配置为"总是外部浏览器"
- **WHEN** 用户在设置中选择"总是在外部浏览器打开"
- **THEN** Chat 中的链接点击行为与当前一致（target="_blank"）

#### Scenario: 修饰键反转默认行为
- **WHEN** 默认为内置浏览器，用户 Shift+Click
- **THEN** 在外部浏览器打开
- **WHEN** 默认为外部浏览器，用户 Shift+Click
- **THEN** 在内置浏览器打开
