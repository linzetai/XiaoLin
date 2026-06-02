## 1. 剪贴板工具

- [ ] 1.1 在 `xiaolin-app/src-tauri/src/` 添加跨平台 clipboard Tauri commands（read_text, write_text, read_image, write_image）
- [ ] 1.2 macOS 实现：使用 `arboard` crate
- [ ] 1.3 Windows 实现：使用 `arboard` crate
- [ ] 1.4 Linux 实现：使用 `arboard` crate（自动检测 Wayland/X11）
- [ ] 1.5 在 `xiaolin-agent/src/builtin_tools/` 添加 `clipboard.rs` 模块，注册 `clipboard_read` 和 `clipboard_write` tools
- [ ] 1.6 工具通过 WebSocket → Tauri command 路径与系统剪贴板交互
- [ ] 1.7 添加 Tauri capability 权限配置
- [ ] 1.8 验证跨平台编译通过

## 2. Quick Action Bar

- [ ] 2.1 在 `tauri.conf.json` 中配置 `quick-action` window（frameless, transparent, always-on-top, hidden）
- [ ] 2.2 创建 `QuickActionBar.tsx` 前端组件（输入框 + 最近对话摘要）
- [ ] 2.3 实现窗口路由：main window 渲染 App，quick-action window 渲染 QuickActionBar
- [ ] 2.4 注册全局快捷键 `Ctrl+Shift+L`（可配置），toggle quick-action window visibility
- [ ] 2.5 实现输入 → 创建/追加对话 → WebSocket 发送逻辑
- [ ] 2.6 实现 Esc/blur 隐藏、Enter 发送交互
- [ ] 2.7 使用 `tauri-plugin-positioner` 实现居中定位
- [ ] 2.8 添加显示/隐藏动画（CSS transition）

## 3. 语音输入（Phase 1: 系统 STT）

- [ ] 3.1 调研并选择跨平台 STT 方案：系统 API vs whisper-rs
- [ ] 3.2 在 `xiaolin-app/src-tauri/src/` 添加 audio capture Tauri commands（start_recording, stop_recording）
- [ ] 3.3 集成系统 STT API（macOS: SFSpeechRecognizer, Windows: SpeechRecognition, Linux: whisper.cpp fallback）
- [ ] 3.4 前端添加 Voice 控制 UI：录音按钮、波形指示器、录音状态
- [ ] 3.5 实现 push-to-talk 快捷键绑定
- [ ] 3.6 转录文本自动填入当前输入框或 Quick Action Bar
- [ ] 3.7 添加 STT 不可用时的 graceful degradation（禁用按钮 + 提示）

## 4. MCP Marketplace UI

- [ ] 4.1 创建 MCP server 注册表 JSON 文件（curated list：包含 name, description, category, install command, config template）
- [ ] 4.2 在设置页面添加 "MCP 市场" tab
- [ ] 4.3 实现 server 列表 UI：分类筛选、搜索、安装状态
- [ ] 4.4 实现一键安装流程：写入 mcpServers 配置 + 触发 gateway reload
- [ ] 4.5 实现 server 状态显示：running/stopped/error
- [ ] 4.6 实现卸载流程：移除配置 + 可选清理 binary
- [ ] 4.7 与现有 `manage_mcp_server` tool 的 gateway API 对接

## 5. 截图/OCR 增强

- [ ] 5.1 验证现有 screenshot tool 在 macOS/Windows 上的工作状态
- [ ] 5.2 修复跨平台截图区域选择器（如需要）
- [ ] 5.3 添加可选 OCR 管线：screenshot → tesseract/paddleocr → 文本提取

## 6. 集成验证

- [ ] 6.1 `cargo check --workspace` 通过
- [ ] 6.2 `npx tsc --noEmit` 通过
- [ ] 6.3 `cargo tauri dev` 启动并验证剪贴板、Quick Action Bar 功能
