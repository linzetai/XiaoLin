## ADDED Requirements

### Requirement: 页面加载进度条可视化
系统 SHALL 在浏览器内容区域上方显示一个 2px 高的进度条，当页面处于 loading 状态时提供视觉反馈。进度条 SHALL 使用 NProgress 风格的模拟进度动画（非真实加载进度）。

#### Scenario: 页面开始加载时进度条出现
- **WHEN** 页面 `loadState` 变为 `"loading"`（通过导航、链接点击、后退/前进触发）
- **THEN** 进度条从 0% 开始动画，200ms 内快速到达 30%，然后 2s 内减速到 60%，再 8s 内缓慢到 85%
- **THEN** 进度条在 85% 处停留等待，直到收到 loading 完成信号

#### Scenario: 页面加载完成时进度条消失
- **WHEN** 页面 `loadState` 从 `"loading"` 变为 `"ready"`
- **THEN** 进度条在 200ms 内从当前位置加速到 100%
- **THEN** 进度条在 150ms 内 fade out（opacity 0）并从 DOM 中移除

#### Scenario: 页面加载失败时进度条消失
- **WHEN** 页面 `loadState` 从 `"loading"` 变为 `"failed"`
- **THEN** 进度条立即 fade out（150ms），不播放到 100% 的动画

#### Scenario: 快速连续导航
- **WHEN** 用户在上一次导航未完成时发起新的导航
- **THEN** 进度条 SHALL 重置到 0% 并重新开始动画，不出现多条进度条

### Requirement: 进度条不干扰用户交互
进度条 SHALL 设置 `pointer-events: none`，不阻挡用户与浏览器 chrome 或 WebView 的交互。

#### Scenario: 用户在加载中点击地址栏
- **WHEN** 进度条正在显示且用户点击进度条区域
- **THEN** 点击事件穿透进度条，触发进度条下方元素的交互

### Requirement: 进度条位于浏览器 chrome 和 WebView 之间
进度条 SHALL 渲染在 React 层（BrowserTabContent 组件内），位于地址栏和 WebView 占位 div 之间，确保不被 OS 级 WebView 遮挡。

#### Scenario: WebView 渲染在进度条下方
- **WHEN** WebView 正在显示页面内容且进度条处于活动状态
- **THEN** 进度条 SHALL 在 WebView 内容上方可见，不被 WebView 遮挡

### Requirement: 长时间加载的持续反馈
进度条 SHALL 在长时间加载时持续提供视觉反馈，告知用户页面仍在加载中。

#### Scenario: 加载超过 15 秒的 trickling 模式
- **WHEN** 页面 `loadState` 保持 `"loading"` 超过 15 秒
- **THEN** 进度条 SHALL 在 85% 附近做微小的来回抖动动画（±2%）
- **THEN** 视觉上表明加载仍在进行中，未卡死
