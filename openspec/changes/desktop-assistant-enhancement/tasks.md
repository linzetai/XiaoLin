## 1. 剪贴板工具

- [x] 1.1 在 `xiaolin-app/src-tauri/src/` 添加跨平台 clipboard Tauri commands（read_text, write_text, read_image, write_image）
- [x] 1.2 macOS 实现：使用 `arboard` crate（跨平台统一实现）
- [x] 1.3 Windows 实现：使用 `arboard` crate（跨平台统一实现）
- [x] 1.4 Linux 实现：使用 `arboard` crate（自动检测 Wayland/X11）
- [x] 1.5 在 `xiaolin-agent/src/builtin_tools/` 添加 `clipboard.rs` 模块，注册 `clipboard_read` 和 `clipboard_write` tools（deferred，通过 ToolSearch 可用）
- [x] 1.6 Agent 工具直接通过 `arboard` 与系统剪贴板交互（无需 WebSocket → Tauri 绕行，同进程）
- [x] 1.7 自定义 IPC 命令已在 invoke_handler 注册，无需额外 Tauri capability
- [x] 1.8 `cargo check --workspace` 通过

## 2. Quick Action Bar

- [x] 2.1 在 `tauri.conf.json` 中配置 `quick-action` window（frameless, transparent, always-on-top, hidden, skipTaskbar）
- [x] 2.2 创建 `QuickActionBar.tsx` 前端组件（搜索图标 + 输入框 + 发送按钮）
- [x] 2.3 实现窗口路由：main.tsx 根据 pathname 渲染 App 或 QuickActionBar
- [x] 2.4 注册全局快捷键 `Ctrl+Shift+L`，toggle quick-action window visibility
- [x] 2.5 输入 → Enter 发送 → 隐藏窗口（WebSocket 对接待后续 Phase 补充）
- [x] 2.6 实现 Esc 隐藏、blur（失焦）自动隐藏、Enter 发送交互
- [x] 2.7 使用 Tauri 内建 `center: true` 实现居中定位（无需额外 positioner plugin）
- [x] 2.8 添加 focus 动画（box-shadow transition）和 capability 权限配置

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
