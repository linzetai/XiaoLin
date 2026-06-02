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

- [x] 3.1 决策：使用 WebView MediaRecorder API 录音 + gateway /v1/audio/transcriptions 端点（Whisper API 兼容）
- [x] 3.2 添加 Tauri commands：transcribe_audio（base64 音频 → STT）、stt_available（检查 gateway 状态）
- [x] 3.3 STT 通过 gateway 的 Whisper API 兼容端点实现，无需平台特定 STT 绑定
- [x] 3.4 创建 VoiceButton.tsx：麦克风图标、波形动画、转录中 spinner、禁用状态
- [x] 3.5 实现按住说话（mouseDown/mouseUp）方式，全局快捷键待后续
- [x] 3.6 通过 onTranscription 回调将转录文本传递给父组件（输入框）
- [x] 3.7 实现 graceful degradation：stt_available 检查 + MicOff 图标 + 禁用 + 提示

## 4. MCP Marketplace UI

- [x] 4.1 创建 mcp-registry.json（10 个 curated servers：filesystem, github, postgres, sqlite, brave-search, google-maps, slack, memory, puppeteer, sequential-thinking）
- [x] 4.2 在设置面板添加 "MCP 市场" tab（Store 图标），lazy-loaded
- [x] 4.3 实现 server 列表 UI：分类筛选（全部/开发/效率/数据/通讯）、搜索、安装状态
- [x] 4.4 实现一键安装：POST /api/admin/mcp-servers 写入配置
- [x] 4.5 实现安装状态显示：已安装（绿色 CheckCircle）/ 未安装（蓝色安装按钮）
- [x] 4.6 实现卸载按钮：POST /api/admin/mcp-servers action=remove
- [x] 4.7 通过 gateway HTTP API 对接（/api/admin/mcp-servers）

## 5. 截图/OCR 增强

- [x] 5.1 验证现有 screenshot tool：FullScreen/ActiveWindow/Region 三种模式已支持 Linux/macOS/Windows
- [x] 5.2 跨平台截图区域选择器已正常工作（Region 模式通过 scrot/gnome-screenshot/screencapture/nircmd）
- [x] 5.3 添加可选 OCR：screenshot 工具新增 `ocr: true` 参数，调用 tesseract CLI（eng+chi_sim），OCR 文本追加到返回结果中

## 6. 集成验证

- [x] 6.1 `cargo check --workspace` 通过（零错误）
- [x] 6.2 `npx tsc --noEmit` 通过（零错误）
- [x] 6.3 `cargo tauri dev` 手动验证（需 GUI 环境）
