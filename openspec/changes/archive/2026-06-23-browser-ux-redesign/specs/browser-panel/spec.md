## MODIFIED Requirements

### Requirement: 浏览器 Chrome 结构
浏览器 Chrome 区域 SHALL 按以下顺序从上到下排列：PageTabs → AddressBar → ProgressBar → WebView Placeholder。标签栏 SHALL 在地址栏上方。

#### Scenario: 浏览器 chrome 渲染顺序
- **WHEN** 浏览器面板或全宽面板渲染时
- **THEN** 标签栏 SHALL 在地址栏上方显示
- **THEN** 进度条 SHALL 在地址栏和 WebView 之间显示

### Requirement: 地址栏 Stop/Reload 按钮切换
地址栏的刷新按钮 SHALL 在页面加载中时切换为停止按钮。

#### Scenario: 加载中显示 Stop 按钮
- **WHEN** 当前页面 `loadState` 为 `"loading"`
- **THEN** 地址栏 SHALL 显示 ✕ (X) 图标代替 ↻ (ArrowClockwise) 图标
- **THEN** 点击该按钮 SHALL 执行 `window.stop()` 停止页面加载

#### Scenario: 空闲时显示 Reload 按钮
- **WHEN** 当前页面 `loadState` 不为 `"loading"`
- **THEN** 地址栏 SHALL 显示 ↻ (ArrowClockwise) 图标
- **THEN** 点击该按钮 SHALL 执行 `location.reload()` 刷新页面

## ADDED Requirements

### Requirement: 后退/前进操作即时反馈
用户点击后退或前进按钮时，系统 SHALL 立即（乐观地）将页面 `loadState` 设为 `"loading"`，触发进度条和 spinner 的即时响应。

#### Scenario: 后退按钮即时反馈
- **WHEN** 用户点击后退按钮
- **THEN** 页面 `loadState` SHALL 立即设为 `"loading"`
- **THEN** 进度条 SHALL 在点击瞬间开始动画
- **THEN** 若 5 秒内未收到后端 `browser-loading` 事件，`loadState` SHALL 自动恢复为 `"ready"`

#### Scenario: 前进按钮即时反馈
- **WHEN** 用户点击前进按钮
- **THEN** 行为 SHALL 与后退按钮一致——立即设 loading 状态并设 5 秒超时

#### Scenario: 后端事件取消超时
- **WHEN** 乐观设置 loading 后，5 秒内收到后端 `browser-loading` 事件
- **THEN** 超时定时器 SHALL 被取消
- **THEN** 后续状态由后端事件驱动

#### Scenario: 慢速网络下 Started 延迟
- **WHEN** 后端 `on_page_load Started` 延迟超过 2 秒但在 5 秒内到达
- **THEN** 进度条和 spinner SHALL 持续显示（不因超时被中断）

### Requirement: Stop 按钮的状态兜底
`browserStopLoading` 执行后 SHALL 乐观设置 `loadState: "ready"`，防止 WebKit 不 emit Finished 事件时前端卡在 loading。

#### Scenario: Stop 后 WebKit 未 emit Finished
- **WHEN** 用户点击 Stop 按钮且 `window.stop()` 执行成功
- **AND** WebKit 未在 500ms 内发出 `on_page_load Finished` 事件
- **THEN** 前端 SHALL 保持 `loadState: "ready"`（由 Stop 乐观设置）

#### Scenario: Stop 后 WebKit 又 emit Started
- **WHEN** 用户点击 Stop 按钮后 500ms 内又收到 `browser-loading Started` 事件
- **THEN** `loadState` SHALL 切回 `"loading"`（后端状态优先）

### Requirement: 地址栏 Chat Toggle 按钮
全宽模式下，地址栏右端 SHALL 显示 Chat toggle 按钮（💬 图标），用于控制 Chat 面板的展开/折叠。

#### Scenario: 全宽模式显示 Chat toggle
- **WHEN** 布局模式为全宽
- **THEN** 地址栏右端 SHALL 显示 Chat toggle 按钮

#### Scenario: Panel 模式不显示 Chat toggle
- **WHEN** 布局模式为 Panel
- **THEN** 地址栏 SHALL 不显示 Chat toggle 按钮（Chat 在主内容区域，无需 toggle）

### Requirement: 标签页 Favicon 显示
页面加载完成后，系统 SHALL 提取页面的 favicon URL 并在标签页中显示，替代默认的 Globe 图标。

#### Scenario: 页面加载完成后显示 favicon
- **WHEN** 页面 `loadState` 变为 `"ready"`
- **THEN** 系统 SHALL 在 browser WebView 中 eval JS 提取 `<link rel="icon">` 的 href（fallback 到 `origin/favicon.ico`），优先通过 canvas 转为 data URL 回传
- **THEN** 标签页 SHALL 将 Globe 图标替换为 `<img>` 显示 favicon（data URL 或 fallback URL）

#### Scenario: Favicon 加载失败回退到 Globe
- **WHEN** favicon URL 对应的图片加载失败（404、跨域等）
- **THEN** 标签页 SHALL 回退显示 Globe 图标

#### Scenario: 导航到新页面时重置 favicon
- **WHEN** 当前标签页导航到新 URL
- **THEN** favicon SHALL 在新页面加载完成前回退到 Globe 图标
- **THEN** 新页面加载完成后 SHALL 显示新页面的 favicon

### Requirement: 浏览器标准键盘快捷键
系统 SHALL 支持以下浏览器标准键盘快捷键。快捷键仅在浏览器可见（全宽模式或 Panel 模式 browser tab 激活）且非编辑区域聚焦（INPUT/TEXTAREA/SELECT/contentEditable）时生效：

#### Scenario: Ctrl+Tab 切换到下一个标签页
- **WHEN** 用户按下 Ctrl+Tab 且有多个标签页
- **THEN** 系统 SHALL 激活当前标签页之后的下一个标签页
- **THEN** 若当前是最后一个标签页 SHALL 循环到第一个

#### Scenario: Ctrl+Shift+Tab 切换到上一个标签页
- **WHEN** 用户按下 Ctrl+Shift+Tab 且有多个标签页
- **THEN** 系统 SHALL 激活当前标签页之前的上一个标签页
- **THEN** 若当前是第一个标签页 SHALL 循环到最后一个

#### Scenario: Ctrl+数字跳转到指定标签页
- **WHEN** 用户按下 Ctrl+1 到 Ctrl+8
- **THEN** 系统 SHALL 激活对应索引位置的标签页（1-indexed）
- **THEN** 若索引超出标签页数量 SHALL 忽略该快捷键

#### Scenario: Ctrl+9 跳转到最后一个标签页
- **WHEN** 用户按下 Ctrl+9
- **THEN** 系统 SHALL 激活最后一个标签页

#### Scenario: F5 或 Ctrl+R 刷新当前页面
- **WHEN** 用户按下 F5 或 Ctrl+R 且有活动标签页
- **THEN** 系统 SHALL 调用 `browserReload` 刷新当前页面

#### Scenario: Escape 停止加载
- **WHEN** 用户按下 Escape 且当前页面处于 loading 状态
- **THEN** 系统 SHALL 调用 `browserStopLoading` 停止加载
