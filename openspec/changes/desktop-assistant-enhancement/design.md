## Context

XiaoLin 桌面端当前是一个 Tauri 2 应用，前端 React 通过 WebSocket 与嵌入的 gateway 通信。Tauri IPC 仅用于文件上传/导出/剪贴板图片读取等少量本地操作。系统托盘有显示/退出两个选项，全局快捷键 `Ctrl+Shift+Space` 仅切换窗口可见性。

作为"全能桌面端 AI 个人助手"，需要从"嵌入 gateway 的浏览器 shell"进化为"深度集成 OS 的助手入口"。

## Goals / Non-Goals

**Goals:**
- 用户可以通过全局快捷键在任何应用上下文中快速与小林交互
- 跨平台的剪贴板读写作为 agent tool 可用
- 语音输入作为对话的替代输入方式
- MCP server 的发现、安装、管理有 GUI 体验

**Non-Goals:**
- 不做移动端（iOS/Android）
- 不做 always-on 语音助手（wake word）
- 不做系统级 accessibility integration（如读屏）
- 不替代 OS 原生快捷方式管理器（如 Raycast/Alfred 的全部功能）

## Decisions

### D1: 剪贴板工具架构

**决定**：Tauri command 层实现跨平台剪贴板 read/write（文本 + 图片），agent 侧注册 `clipboard_read` 和 `clipboard_write` builtin tools，通过 WebSocket 调用 Tauri command。

**平台实现**：
- macOS: `NSPasteboard` via `objc2` 或 `arboard` crate
- Windows: `arboard` crate
- Linux: `wl-copy`/`wl-paste` (Wayland) 或 `xclip` (X11)

**替代方案**：使用 `tauri-plugin-clipboard`。但该插件的 API 不够灵活（无图片格式控制），自行实现更可控。

### D2: Quick Action Bar 窗口

**决定**：创建独立的 Tauri WebView window（`quick-action`），使用 `tauri-plugin-positioner` 居中显示。窗口特性：frameless、always-on-top、transparent background、blur backdrop。

**交互流程**：
1. `Ctrl+Shift+L` 唤出浮窗
2. 输入框自动聚焦
3. 输入后 Enter 发送 → 创建新对话或追加到当前对话
4. Esc 或失去焦点 → 隐藏
5. 可选：显示最近 3 条对话摘要

**理由**：独立 window 避免影响主窗口状态，positioner 处理多显示器场景。

### D3: 语音输入

**决定**：Phase 1 使用系统 STT API（macOS: `NSSpeechRecognizer`/`SFSpeechRecognizer`, Windows: `Windows.Media.SpeechRecognition`, Linux: PipeWire + Whisper.cpp）。Tauri command 暴露 `stt_start`/`stt_stop`/`stt_result`。

**交互模式**：Push-to-talk（长按快捷键录音，松开转文字并发送）。

**替代方案**：嵌入 `whisper-rs` 做本地 STT。优点是跨平台统一，缺点是模型文件大（~75MB base 模型）且需要 GPU 加速。作为 Phase 2。

### D4: MCP Marketplace

**决定**：在设置页面新增 "MCP 市场" tab。数据源：本地 JSON registry 文件（随应用分发），列出预配置的 MCP server 包。安装 = 写入 `config/agents/main.json` 的 `mcpServers` 数组 + 下载/配置 server binary。

**不做**：在线 marketplace server。Phase 1 是本地 curated list。

## Risks / Trade-offs

- **跨平台剪贴板** → Linux Wayland/X11 分裂，需要运行时检测。`arboard` crate 处理了大部分，但图片格式兼容性可能有 edge case。
- **Quick Action Bar 性能** → 独立 WebView window 有启动开销。缓解：预创建 hidden window，快捷键只 toggle visibility。
- **语音输入隐私** → 系统 STT API 可能将音频发送到云端。缓解：在 UI 中明确标注，提供本地 Whisper 选项。
- **MCP 安装安全** → 安装 MCP server 等于运行第三方代码。缓解：signature verification + sandbox（利用现有 sandbox 栈）。
- **范围膨胀** → 4 个独立功能组合在一个 change 中。缓解：tasks 按功能独立，可以分批实施。
