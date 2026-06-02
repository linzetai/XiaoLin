## Why

XiaoLin 定位为"全能桌面端 AI 个人助手"，但当前桌面端只是一个嵌入 gateway 的 WebView shell，缺乏个人助手应有的 OS 深度集成。剪贴板仅支持 Linux 图片读取、全局快捷键仅能显示/隐藏窗口、无语音输入、无快速捕获流程。这些差距导致产品体验更像"浏览器里的 ChatGPT"而非"桌面原生助手"。

## What Changes

- **一等公民的剪贴板工具**：
  - Tauri plugin 实现跨平台（macOS/Windows/Linux）文本+图片读写
  - Agent builtin tool `clipboard_read` / `clipboard_write`
  - 可选：剪贴板历史环形缓冲 + 监控模式
- **全局快捷操作栏**（Spotlight/Raycast 风格）：
  - 新全局快捷键（如 `Ctrl+Shift+L`）唤出迷你浮窗
  - 支持快速输入问题、粘贴内容、发起新对话
  - 不是显示/隐藏主窗口，而是独立的轻量交互入口
- **语音输入循环**：
  - STT 热键（按住说话 / push-to-talk）
  - 集成 Whisper 或系统 STT API
  - 语音输入 → 文本 → 接入现有流式对话管线
- **MCP 精选包 + 一键安装 UI**：
  - 预配置的 MCP server 包（Google Calendar、Gmail、Apple Reminders 等）
  - 设置页面中的 MCP 市场 UI，一键安装/启用
  - 替代手动编辑 JSON 配置的体验
- **截图/OCR 增强**：
  - 跨平台截图区域选择器（当前仅 Linux）
  - 可选 OCR 管线（screenshot → 文本提取 → 上下文注入）

## Capabilities

### New Capabilities

- `clipboard-tool`: 跨平台剪贴板读写工具的接口、权限模型和历史管理
- `quick-action-bar`: 全局浮窗快捷操作栏的 UI 规范、快捷键和交互流程
- `voice-input`: 语音输入的采集、STT 集成和对话管线接入
- `mcp-marketplace`: MCP server 精选包的分发、安装和生命周期管理

### Modified Capabilities

_无现有 spec 需要修改。_

## Impact

- **Tauri 层**：新增 Tauri plugin（clipboard、voice、quick-action window）
- **前端**：新增 QuickActionBar 组件、MCP Marketplace 页面、Voice 控制 UI
- **Agent**：新增 clipboard_read/write builtin tools
- **配置**：新增 clipboard、voice、quickAction 配置节
- **平台特定**：需要 macOS（Accessibility 权限）、Windows（UI Automation）、Linux（wl-copy/xclip）适配
- **依赖**：可能引入 whisper-rs 或系统 STT 绑定
