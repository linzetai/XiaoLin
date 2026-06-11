## 1. PTY 后端基础

- [x] 1.1 创建 `xiaolin-pty` crate，添加 `portable-pty` 依赖
- [x] 1.2 实现 `PtySession` 结构体（封装 master fd、child handle、尺寸、活动时间戳）
- [x] 1.3 实现 `PtySessionManager`（会话池：创建/获取/关闭/列表，最大 8 会话限制）
- [x] 1.4 实现 idle timeout 清理逻辑（30 分钟无活动自动关闭 + 僵尸进程回收）

## 2. WebSocket 端点

- [x] 2.1 在 gateway 注册 `/api/v1/pty` WebSocket 路由
- [x] 2.2 实现 WS handler：Text 帧解析为控制消息（resize/ping）
- [x] 2.3 实现 WS handler：Binary 帧 → PTY stdin 写入
- [x] 2.4 实现 PTY stdout → Binary 帧 WebSocket 推送（tokio select loop）
- [x] 2.5 实现会话退出检测 → 发送 `{"type":"session_closed","exit_code":N}` 并关闭连接

## 3. 前端 xterm.js 集成

- [x] 3.1 安装 `@xterm/xterm`、`@xterm/addon-fit`、`@xterm/addon-webgl` 依赖
- [x] 3.2 创建 `InteractiveTerminal.tsx` 组件（xterm 实例化 + WebSocket 连接）
- [x] 3.3 实现键盘输入 → Binary WS 帧发送
- [x] 3.4 实现 WS Binary 帧接收 → xterm.write()
- [x] 3.5 集成 fit addon，面板 resize 时自动调整 cols/rows 并发送 resize 控制消息

## 4. 会话管理 UI

- [x] 4.1 创建 `usePtyStore` Zustand store（会话列表、活跃会话、连接状态）
- [x] 4.2 实现多 session tab 切换（保留各 xterm 实例和滚动位置）
- [x] 4.3 实现 "+" 按钮创建新会话
- [x] 4.4 实现关闭按钮 + 会话退出标记 "[已退出]"

## 5. Terminal Tab 重构

- [x] 5.1 将 Terminal tab 拆分为 "Output" 和 "Shell" 子视图
- [x] 5.2 "Output" 子视图保留现有 TerminalPanel 功能
- [x] 5.3 "Shell" 子视图集成 InteractiveTerminal 组件
- [x] 5.4 首次切换到 "Shell" 时自动创建一个 PTY 会话

## 6. 验证与测试

- [x] 6.1 后端单元测试：PtySession 创建/读写/resize/关闭
- [x] 6.2 E2E 测试：通过 MCP 工具验证终端输入/输出/resize
- [x] 6.3 跨平台验证：Linux 基本功能 + macOS 兼容性检查
