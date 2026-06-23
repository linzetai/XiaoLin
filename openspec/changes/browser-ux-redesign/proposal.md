## Why

内置浏览器的全宽模式交互不符合用户对浏览器的心智模型：Chat 面板在左侧将浏览器挤到中间位置，浏览器不是布局主角；同时页面内链接跳转时缺乏加载反馈（无进度条、无内容区 loading 状态），用户无法判断点击是否生效、页面是否在加载，体验与主流浏览器差距明显。此外，所有标签页都使用相同的 Globe 图标缺乏辨识度，且缺少浏览器常用的键盘快捷键（Ctrl+Tab 切换标签、F5 刷新等），降低了日常使用的效率。

## What Changes

- **全宽模式布局反转**: Chat 面板从左侧移到右侧，浏览器成为 flex:1 主体占据左侧；Chat 折叠时完全隐藏（0px），通过地址栏 toggle 按钮唤出
- **浏览器 chrome 顺序调整**: 标签栏移到地址栏上方，与 Chrome/Firefox 一致
- **新增页面加载进度条**: 在 WebView 内容区域顶部添加 NProgress 风格的模拟进度条，提供导航过程中的视觉反馈
- **地址栏 Stop/Reload 切换**: 加载中时 reload 按钮替换为 stop 按钮（✕），支持停止加载
- **后退/前进即时反馈**: 调用 history.back/forward 时乐观设置 loading 状态，进度条和 spinner 立即响应
- **Chat toggle 整合到地址栏**: 全宽模式的 Chat 展开/收起按钮放入地址栏右端，取代独立的全宽切换按钮
- **全宽模式不显示 WorkspacePanel**: 全宽模式下移除 WorkspacePanel 以最大化浏览器面积，用户可通过快捷键切回 Panel 模式使用（overlay 形式作为后续迭代）
- **标签页 Favicon**: 页面加载完成后提取 favicon URL 并显示在标签页中，替代统一的 Globe 图标
- **键盘快捷键补充**: 新增 Ctrl+Tab/Ctrl+Shift+Tab（切换标签）、Ctrl+1~8（跳转标签）、F5/Ctrl+R（刷新）、Escape（停止加载）等浏览器常用快捷键

## Capabilities

### New Capabilities
- `browser-progress-bar`: 页面加载进度条——NProgress 风格的模拟进度动画，loading 状态驱动，支持 start/done/reset
- `browser-fullwidth-layout`: 全宽模式布局重设计——浏览器优先的布局模型，Chat 右侧面板（可展开/隐藏），地址栏 toggle 按钮

### Modified Capabilities
- `browser-panel`: 浏览器 chrome 结构调整（标签栏/地址栏顺序、Stop 按钮、Chat toggle 整合）、后退/前进即时反馈、Favicon 显示、键盘快捷键补充

## Impact

- **前端 `crates/xiaolin-app/src/`**:
  - `components/shell/ContentBlock.tsx`: 全宽模式布局顺序翻转（Browser → Chat），WorkspacePanel 不显示
  - `components/browser/ChatSidePanel.tsx`: Chat 从左侧移到右侧，折叠态从 48px 改为 0px
  - `components/browser/BrowserTabContent.tsx`: 标签栏移到地址栏上方
  - `components/browser/BrowserAddressBar.tsx`: Stop/Reload 切换、Chat toggle 按钮整合
  - 新增 `components/browser/BrowserProgressBar.tsx`: 进度条组件，渲染在 BrowserTabContent 内（地址栏与占位 div 之间）
  - `components/browser/BrowserPageTabs.tsx`: Favicon 图标替代 Globe 图标
  - `lib/stores/browser-store.ts`: 后退/前进乐观更新 loading 状态、Chat 折叠态逻辑调整、Favicon URL 存储
- **后端微调**: `on_page_load Finished` 后 eval JS 在 browser WebView 内提取 favicon 并通过 canvas 转为 data URL，经 `__XIAOLIN__.notify('favicon', ...)` 回传，后端 emit `browser-favicon-changed` 扁平事件
- **无依赖变更**: 使用纯 CSS 动画实现进度条，Favicon 使用 `<img>` 原生加载，不引入新依赖
