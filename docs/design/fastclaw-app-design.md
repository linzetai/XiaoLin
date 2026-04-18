# FastClaw App — 跨平台桌面客户端设计文档

> **版本**: v1.1 (Multi-Chat 架构)  
> **日期**: 2026-04-20  
> **状态**: In Progress (Phase 2 完成, Phase 3-4 进行中)  
> **作者**: FastClaw Team  
> **关联**: [PRD F-012 Harness Studio](../prd/product-requirements.md), [架构设计](architecture-design.md)

---

## 1. 背景与动机

### 1.1 问题陈述

当前 FastClaw 提供两种用户入口：

| 入口 | 启动方式 | 体验痛点 |
|------|---------|---------|
| **嵌入式 Web UI** | 先 `fastclaw serve`，再浏览器访问 `http://localhost:18789` | 两步操作，需要保持终端窗口 |
| **CLI TUI** | 先 `fastclaw serve`，再 `fastclaw tui` | 两步操作，TUI 功能受限 |

两者都要求用户**先启动一个独立的后端服务**，然后再用客户端连接。这与 Codex、Cursor 等现代 AI 工具"打开即用"的体验形成了鲜明对比。

### 1.2 设计目标

| 目标 | 度量标准 |
|------|---------|
| **零配置启动** | 双击 App / 运行 `fastclaw app` 即进入可用状态，无需手动启动后端 |
| **全功能覆盖** | 与 Web Studio 功能对齐：Chat、Session、Agent、DAG、Settings 等 |
| **类 Codex 体验** | Agent Mode — 交互式 AI Agent，支持工具调用、流式输出、会话管理 |
| **跨平台** | Desktop (macOS / Windows / Linux)；Mobile (iOS / Android) 为 P2 延伸目标 |
| **CLI 不受影响** | `fastclaw` CLI 保持独立可用，App 与 CLI 共享配置和数据 |
| **轻量** | 安装包 < 30MB，运行内存 < 100MB (含嵌入式网关) |

### 1.3 与现有入口的关系

```
用户视角（修改后）:

  fastclaw app          → 桌面 App（嵌入网关 + GUI）     ⭐ 新增
  fastclaw              → CLI Agent Mode（嵌入网关 + TUI）⭐ 新增
  fastclaw serve        → 纯服务模式（API only，保持不变）
  fastclaw tui --url .. → 连接远程网关的 TUI（保持不变）
```

---

## 2. 技术选型

### 2.1 决策：Tauri 2.0 + React

| 维度 | 选择 | 理由 |
|------|------|------|
| **客户端框架** | Tauri 2.0 | Rust 原生宿主，可直接嵌入 `fastclaw-gateway`；单进程；< 10MB shell 开销 |
| **前端框架** | React 19 + TypeScript | 最大生态，Chat/AI 类组件成熟，shadcn/ui 高质量组件库 |
| **构建工具** | Vite 6 | 极速 HMR，Tauri 官方推荐 |
| **样式方案** | TailwindCSS 4 | 原子化 CSS，与 shadcn/ui 天然配合 |
| **状态管理** | Zustand | 轻量、TypeScript 友好、无 boilerplate |
| **WebSocket** | 原生 WebSocket API | 与现有 `fastclaw-ws/1` 协议直连，无需额外库 |

### 2.2 排除方案及理由

| 方案 | 排除理由 |
|------|---------|
| **Electron** | Chromium 捆绑 ~150MB，内存 300MB+，与 FastClaw 轻量化定位矛盾 |
| **Flutter** | Dart 语言无法直接嵌入 Rust crate，需 FFI 桥接增加复杂度和维护成本 |
| **React Native** | 桌面支持不成熟 (RN Windows/macOS 维护缓慢)，无 Linux 支持 |
| **原生 Swift/Kotlin** | 多套代码库，无法共享 UI 层，维护成本高 |
| **PWA** | 无法嵌入网关、无系统级能力（托盘、全局快捷键、文件系统深度访问）|

### 2.3 Tauri 2.0 关键特性利用

| Tauri 特性 | 在 FastClaw App 中的应用 |
|-----------|------------------------|
| **Rust Backend Commands** | 嵌入式网关启动/停止、配置读写、健康检查 |
| **IPC (invoke)** | 前端通过 IPC 获取网关状态、动态端口、认证 Token |
| **System Tray** | 后台常驻，快速唤起 App，显示 Agent 状态 |
| **Global Shortcut** | 全局快捷键呼出 Agent（类似 Spotlight/Raycast）|
| **Notifications** | Agent 任务完成通知、错误告警 |
| **Auto Updater** | 内置自动更新机制 |
| **Deep Link** | `fastclaw://` 协议处理，支持从浏览器/CLI 唤起 App |
| **Tauri Mobile (2.0)** | P2 阶段延伸至 iOS/Android |

---

## 3. 系统架构

### 3.1 进程模型

FastClaw App 采用**单进程嵌入式网关**架构，与 Codex 的运行模型一致：

```
┌─── fastclaw-app 进程 ──────────────────────────────────────────┐
│                                                                 │
│  Tokio Runtime (shared)                                         │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │              Embedded Gateway (fastclaw-gateway)            │  │
│  │                                                            │  │
│  │  ┌─────────┐ ┌──────────┐ ┌────────────┐ ┌─────────────┐  │  │
│  │  │ REST API│ │WebSocket │ │  Agent RT  │ │  Session DB │  │  │
│  │  │  /api/* │ │   /ws    │ │(LLM+Tools) │ │  (SQLite)   │  │  │
│  │  └─────────┘ └──────────┘ └────────────┘ └─────────────┘  │  │
│  │  ┌─────────┐ ┌──────────┐ ┌────────────┐ ┌─────────────┐  │  │
│  │  │ Memory  │ │   DAG    │ │  Plugins   │ │  Evolution  │  │  │
│  │  │(向量+KG)│ │ (工作流) │ │  (WASM)    │ │  (自进化)   │  │  │
│  │  └─────────┘ └──────────┘ └────────────┘ └─────────────┘  │  │
│  └───────────────────────┬────────────────────────────────────┘  │
│                          │ 127.0.0.1:{dynamic_port}              │
│  ┌───────────────────────▼────────────────────────────────────┐  │
│  │           Tauri IPC Bridge (Rust Commands)                  │  │
│  │                                                            │  │
│  │  · get_gateway_info() → { port, ws_url, health }           │  │
│  │  · get_config()       → FastClawConfig                     │  │
│  │  · restart_gateway()  → Result<()>                         │  │
│  └───────────────────────┬────────────────────────────────────┘  │
│                          │ Tauri IPC (JSON-RPC over IPC)         │
│  ┌───────────────────────▼────────────────────────────────────┐  │
│  │              WebView (React + TypeScript)                    │  │
│  │                                                            │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐  │  │
│  │  │Agent Chat│ │ Sessions │ │ Settings │ │ Flow Editor  │  │  │
│  │  │(WS 直连) │ │  (REST)  │ │  (IPC)   │ │  (REST+WS)   │  │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────┘  │  │
│  └────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 启动流程

```
App 启动
  │
  ├─ 1. 加载配置 (fastclaw_core::config::load_config)
  │     · 读取 ~/.fastclaw/config/default.json
  │     · 合并环境变量覆盖
  │
  ├─ 2. 检测端口冲突
  │     · 尝试绑定 config.gateway.port
  │     · 冲突时自动选择随机可用端口
  │
  ├─ 3. 启动嵌入式网关 (tokio::spawn)
  │     · fastclaw_gateway::run_with_listener(config, listener)
  │     · 网关在后台 tokio task 中运行
  │     · 返回实际绑定的 SocketAddr
  │
  ├─ 4. 等待网关就绪
  │     · 轮询 /health 直到返回 200
  │     · 超时 5s 后报错
  │
  ├─ 5. 初始化 Tauri WebView
  │     · 注册 IPC Commands
  │     · 将网关地址传递给前端
  │     · 创建窗口
  │
  └─ 6. 前端初始化
        · 从 IPC 获取 gateway_url
        · 建立 WebSocket 连接到 ws://127.0.0.1:{port}/ws
        · 渲染 Agent Chat 界面
```

### 3.3 通信架构

App 内的通信分为三个层次：

| 通信路径 | 协议 | 用途 | 数据格式 |
|---------|------|------|---------|
| **WebView → Gateway** | WebSocket | Chat 流式交互、实时事件 | `fastclaw-ws/1` JSON |
| **WebView → Gateway** | HTTP REST | CRUD 操作（Session/Agent/DAG/Config）| JSON |
| **WebView → Tauri** | IPC (invoke) | 网关状态查询、本地配置、系统能力 | JSON-RPC |

```
                     ┌─────────────────┐
                     │    WebView      │
                     │  (React App)    │
                     └──┬─────┬────┬──┘
                        │     │    │
           Tauri IPC ◄──┘     │    └──► HTTP REST
           (系统能力)          │        (CRUD)
                              │
                         WebSocket
                      (实时 Chat 流)
                              │
                     ┌────────▼────────┐
                     │  Embedded       │
                     │  Gateway        │
                     └─────────────────┘
```

**设计决策**：前端通过标准的 HTTP/WS 协议与网关通信，而不是全部走 Tauri IPC。这保证了：
1. 前端代码可同时用于 Web 版 Studio（浏览器直连远程网关）
2. 协议与 CLI TUI 完全一致，任何 bug 只需修一处
3. IPC 仅用于 Tauri 特有的系统级能力（托盘、通知、快捷键等）

---

## 4. Cargo 工作区集成

### 4.1 新增 crate

在现有 workspace 中新增 `crates/fastclaw-app/`：

```
crates/fastclaw-app/
├── src-tauri/
│   ├── Cargo.toml              # Rust 依赖
│   ├── tauri.conf.json         # Tauri 配置
│   ├── build.rs                # Tauri 构建脚本
│   ├── capabilities/           # Tauri 2.0 capability 声明
│   │   └── default.json
│   ├── icons/                  # App 图标（各平台尺寸）
│   └── src/
│       ├── main.rs             # Tauri 入口 + 嵌入式网关启动
│       ├── embedded.rs         # 网关生命周期管理
│       ├── commands.rs         # Tauri IPC Commands 定义
│       └── tray.rs             # 系统托盘
├── src/                        # React 前端
│   ├── main.tsx                # React 入口
│   ├── App.tsx                 # 根组件 + 路由
│   ├── lib/
│   │   ├── gateway.ts          # 网关连接管理（IPC + WS）
│   │   ├── ws-client.ts        # WebSocket 客户端（fastclaw-ws/1 协议）
│   │   ├── api-client.ts       # REST API 客户端
│   │   └── store.ts            # Zustand 全局状态（IM 范式）
│   ├── components/
│   │   ├── agent-list/         # 左栏：Agent 联系人列表
│   │   │   ├── AgentList.tsx
│   │   │   ├── AgentAvatar.tsx
│   │   │   └── AgentSearch.tsx
│   │   ├── message-stream/     # 中栏：跨会话消息流
│   │   │   ├── MessageStream.tsx       # 消息流容器（虚拟滚动）
│   │   │   ├── MessageBubble.tsx       # 单条消息气泡
│   │   │   ├── ToolCallCard.tsx        # 工具调用折叠卡片
│   │   │   ├── SessionDivider.tsx      # 会话分隔线
│   │   │   ├── StreamingIndicator.tsx  # 流式打字指示器
│   │   │   ├── MessageSearch.tsx       # 消息搜索 + 高亮定位
│   │   │   └── InputBar.tsx            # 输入栏（新话题按钮 + 发送）
│   │   ├── agent-detail/       # 右栏：Agent 详情面板（可收起）
│   │   │   ├── AgentDetail.tsx
│   │   │   ├── AgentConfigTab.tsx      # 配置编辑
│   │   │   └── SessionListTab.tsx      # 会话列表 + 跳转定位
│   │   ├── settings/           # 全局设置
│   │   │   ├── SettingsPanel.tsx
│   │   │   ├── ModelConfig.tsx
│   │   │   └── CredentialsForm.tsx
│   │   └── layout/             # 布局组件
│   │       ├── AppLayout.tsx   # 三栏布局容器
│   │       ├── TitleBar.tsx    # 自定义标题栏（无边框窗口）
│   │       └── StatusBar.tsx
│   └── hooks/
│       ├── useGateway.ts       # 网关状态 hook
│       ├── useAgentChat.ts     # Agent 对话流 hook（含 detach 语义）
│       ├── useMessageStream.ts # 消息流分页加载 hook
│       └── useAgentConfig.ts   # Agent 配置读写 hook
├── index.html                  # Vite 入口 HTML
├── package.json                # npm 依赖
├── tsconfig.json               # TypeScript 配置
├── vite.config.ts              # Vite 配置
└── tailwind.config.ts          # TailwindCSS 配置
```

### 4.2 Rust 依赖关系

```toml
# crates/fastclaw-app/src-tauri/Cargo.toml

[package]
name = "fastclaw-app"
version = "0.1.0"
edition = "2021"

[dependencies]
tauri = { version = "2", features = ["tray-icon", "protocol-asset"] }
tauri-plugin-global-shortcut = "2"
tauri-plugin-notification = "2"
tauri-plugin-shell = "2"
tauri-plugin-updater = "2"
tauri-plugin-deep-link = "2"
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }

# 嵌入 FastClaw 核心
fastclaw-core = { path = "../../fastclaw-core" }
fastclaw-gateway = { path = "../../fastclaw-gateway" }
fastclaw-observe = { path = "../../fastclaw-observe" }

[build-dependencies]
tauri-build = "2"
```

### 4.3 Workspace Cargo.toml 修改

```toml
# 在 workspace members 中追加
members = [
    # ... 现有 crates ...
    "crates/fastclaw-app/src-tauri",
]
```

---

## 5. Rust 后端设计

### 5.1 嵌入式网关管理 (`embedded.rs`)

```rust
// 核心结构：管理嵌入式网关的生命周期
pub struct EmbeddedGateway {
    port: u16,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    health_url: String,
}

impl EmbeddedGateway {
    /// 启动嵌入式网关
    /// 1. 加载配置
    /// 2. 绑定端口（冲突时自动选择）
    /// 3. 在 tokio task 中运行网关
    /// 4. 等待 /health 就绪
    pub async fn start(dev: bool, profile: Option<&str>) -> Result<Self>;

    /// 获取网关信息
    pub fn info(&self) -> GatewayInfo;

    /// 优雅关停
    pub async fn shutdown(self) -> Result<()>;
}

pub struct GatewayInfo {
    pub port: u16,
    pub ws_url: String,    // ws://127.0.0.1:{port}/ws
    pub http_url: String,  // http://127.0.0.1:{port}
    pub version: String,
}
```

**关键设计**：

- 网关绑定 `127.0.0.1` 而非 `0.0.0.0`，确保只有本地进程可访问
- 端口冲突时自动 fallback 到 `0`（OS 分配随机端口），避免与已运行的 `fastclaw serve` 冲突
- 保持 `shutdown_tx` 用于优雅关停，App 退出时发送信号

### 5.2 Tauri IPC Commands (`commands.rs`)

```rust
/// 前端通过 invoke("get_gateway_info") 调用
#[tauri::command]
async fn get_gateway_info(
    state: tauri::State<'_, AppData>,
) -> Result<GatewayInfo, String>;

/// 获取当前配置
#[tauri::command]
async fn get_config(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String>;

/// 重启网关（配置变更后）
#[tauri::command]
async fn restart_gateway(
    state: tauri::State<'_, AppData>,
) -> Result<GatewayInfo, String>;

/// 获取网关健康状态
#[tauri::command]
async fn health_check(
    state: tauri::State<'_, AppData>,
) -> Result<bool, String>;

/// 连接远程网关（替代嵌入式网关）
#[tauri::command]
async fn connect_remote(
    url: String,
    token: Option<String>,
    state: tauri::State<'_, AppData>,
) -> Result<GatewayInfo, String>;
```

### 5.3 App 入口 (`main.rs`)

```rust
fn main() {
    // 初始化 Tokio runtime（Tauri 2.0 内置异步支持）
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // 启动嵌入式网关
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let gateway = EmbeddedGateway::start(false, None)
                    .await
                    .expect("failed to start embedded gateway");
                handle.manage(AppData::new(gateway));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_info,
            get_config,
            restart_gateway,
            health_check,
            connect_remote,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
```

---

## 6. 前端设计

### 6.1 交互范式：Agent IM × Multi-Chat

FastClaw App 采用 **IM 联系人列表 + 多会话 Tab** 范式，结合了 IM 的 Agent 管理与浏览器式多标签页体验：

| 概念 | FastClaw 映射 | 说明 |
|------|-------------|------|
| **联系人列表** | Agent 列表 | 侧边栏展示所有可用 Agent，每个 Agent 是一个"联系人" |
| **多标签聊天** | Chat Tabs | 每个 Agent 下可同时打开多个 Chat Tab，每个 Tab 对应一个独立 Session |
| **工作目录绑定** | Chat WorkDir | 每个 Chat 可关联一个工作目录，`@` 引用时索引该目录下的文件 |
| **新会话** | 新建 Tab | 创建新 Tab + 新 Session，独立的 LLM 上下文和消息流 |
| **搜索** | 全文消息搜索 | 在当前 Chat 的消息中搜索，Cmd/Ctrl+F 高亮定位 |
| **联系人详情** | Agent 配置面板 | 右侧面板查看/编辑 Agent 配置、管理所有会话（含已关闭的） |
| **记忆** | 跨会话记忆 | Agent 的 Memory 系统天然跨 Chat，Agent "记得"所有 Chat 中的对话 |
| **Tab 管理** | 开/关/恢复 | Tab 可关闭（移入历史），在右栏历史列表中可恢复 |

**核心体验**：用户打开 App，看到一列 Agent。点击 Agent 进入其多 Tab 工作区，每个 Tab 是一个独立对话会话。可同时打开多个 Chat 处理不同任务，关闭的 Tab 进入历史列表可随时恢复。每个 Chat 可绑定不同工作目录，实现项目隔离。

**选择 Multi-Chat 而非连续流的理由**：
1. **项目隔离**：不同 Chat 绑定不同工作目录，`@` 引用精确索引
2. **上下文清晰**：每个 Tab 独立消息流，避免跨会话信息混淆
3. **并发任务**：可同时与同一 Agent 讨论多个不相关话题
4. **性能**：虚拟滚动在独立 Chat 中更高效，无需加载全量历史

### 6.2 页面结构

```
┌──────────────────────────────────────────────────────────────────────────┐
│  TitleBar  [←][→]  FastClaw                              [🔍] [Model▾] [⚙]│
├─────────┬──────────────────────────────────────────────────┬─────────────┤
│         │  Agent Header: 🤖 FastClaw 助手  ● 在线      [▷]│ (可收起)    │
│  Agent  │  ┌──────────┬───────────┬──────────┬──┐         │  Agent      │
│  List   │  │ 新对话 ✕ │ SQL优化 ✕ │ API设计 ✕│ +│         │  Detail     │
│         │  └──────────┴───────────┴──────────┴──┘         │             │
│ ┌─────┐ │                                                  │ ┌─────────┐│
│ │ M   │ │  [You] 帮我优化这个 SQL 查询                      │ │ Config  ││
│ │Main │ │                                                  │ │         ││
│ │Agent│ │  [Main] 让我分析一下...                            │ │ Model:  ││
│ │ 2m  │ │  ┌─ 🔧 sql_analyze ─────────────────────┐       │ │ qwen3.5 ││
│ │ ago │ │  │ ✅ 分析完成 (120ms)                   │       │ │         ││
│ ├─────┤ │  │  Input: SELECT * FROM ...              │       │ │ Tools:  ││
│ │ C   │ │  │  Output: 优化建议 ...                  │       │ │ 19 个   ││
│ │Code │ │  └───────────────────────────────────────┘       │ │         ││
│ │Review│ │  基于分析结果，有 3 种优化方案...                  │ │ Memory: ││
│ │ 1h  │ │                                                  │ │ enabled ││
│ │ ago │ │                                                  │ └─────────┘│
│ ├─────┤ │                                                  │             │
│ │ D   │ │                                                  │ ┌─────────┐│
│ │Data │ │                                                  │ │Sessions ││
│ │Anal.│ │                                                  │ │(历史)   ││
│ │     │ │                                                  │ │ SQL优化  ││
│ └─────┘ │                                                  │ │ API设计  ││
│         │                                                  │ │ 初始设置 ││
│ [+ New] │  ┌──────────────────────────────────────────────┐│ └─────────┘│
│         │  │ 描述任务，或向 Agent 提问...                   ││             │
│         │  │ [📎][✂][🔧] ~/workspace/my-proj  Enter [发送]││             │
│         │  └──────────────────────────────────────────────┘│             │
├─────────┴──────────────────────────────────────────────────┴─────────────┤
│  StatusBar  ● 已连接 │ 内嵌 46557 │ v0.1.0                              │
└──────────────────────────────────────────────────────────────────────────┘
```

**三栏布局说明**：

| 区域 | 宽度 | 内容 | 交互 |
|------|------|------|------|
| **左栏：Agent List** | 固定 64-72px | Agent 头像/名称/最后活跃时间，可搜索 | 点击切换 Agent，底部 "+ New" 创建 Agent |
| **中栏：Chat Area** | 自适应 | Agent Header → Chat Tabs → 当前 Tab 消息流 → 输入栏 | Tab 切换/拖拽排序/双击重命名/关闭 |
| **右栏：Agent Detail** | 可收起 280px | Agent 配置 + 会话历史列表 | 默认隐藏，点击 Agent 头像或顶栏按钮展开 |

**中栏层次结构**：

| 层 | 组件 | 说明 |
|---|------|------|
| 1 | **Agent Header** | Agent 名称 + 在线状态 + 展开右栏按钮 |
| 2 | **Chat Tabs** | 激活的 Chat 标签页，可拖拽排序、双击重命名、hover 显示关闭按钮，溢出时水平滚动 |
| 3 | **Message Stream** | 当前 Tab 的消息列表（虚拟滚动），markdown 渲染 + 工具调用卡片 |
| 4 | **Input Area** | 消息输入框 + WorkDir 选择器 + 附件/引用/工具快捷操作 |

### 6.3 多 Chat 设计

每个 Agent 支持多个 Chat（会话），每个 Chat 是一个独立的 Tab：

```
Chat Tabs:
┌────────────────────────────────────────────────────────┐
│ [新对话 ✕] [SQL优化 ✕] [API设计 ✕] [+]                  │
└────────────────────────────────────────────────────────┘
         ↓ 选中 "SQL优化"
┌────────────────────────────────────────┐
│                                        │
│  [You] 帮我优化这个 SQL 查询            │
│                                        │
│  [Agent] 让我分析一下...                │
│  🔧 sql_analyze → ✅ (120ms)          │
│  基于分析结果，有 3 种优化方案...        │
│                                        │
│  [You] 用方案 2 实现                    │
│  [Agent] ▊ (正在回复...)                │
│                                        │
└────────────────────────────────────────┘
```

**关键行为**：

| 操作 | 效果 |
|------|------|
| **新建 Chat** (点击 "+" 按钮) | 创建新 Tab + 新 Session，独立的 LLM 上下文和消息流 |
| **关闭 Chat** (点击 Tab 的 "✕") | Tab 从 Tabs 栏移除，进入右栏历史列表（消息保留在后端） |
| **恢复 Chat** (右栏历史列表点击) | Tab 重新出现在 Tabs 栏，加载历史消息 |
| **发送消息** | 追加到当前 Chat 的消息流底部，自动滚动 |
| **切换 Tab** | 切换到另一个 Chat 的消息流，各 Chat 独立滚动位置 |
| **切换 Agent** | 切换到另一个 Agent 的 Chat Tabs，恢复之前的活跃 Tab |
| **拖拽排序** | Chat Tab 支持 HTML5 Drag & Drop 拖拽重排 |
| **重命名 Chat** | 双击 Tab 标题进入编辑模式，Enter 确认 |
| **搜索** | Cmd/Ctrl+F 在当前 Chat 消息中搜索，高亮匹配并上下切换 |
| **WorkDir 绑定** | 每个 Chat 可设置独立的工作目录，`@` 引用时索引该目录 |
| **Agent 记忆** | Memory 系统跨 Chat 共享，Agent 可引用其他 Chat 中的上下文 |

**Chat 生命周期**：

```
创建 ("+") ──→ 激活 (Tab 可见) ──→ 关闭 ("✕") ──→ 归档 (历史列表)
                      ↑                                    │
                      └────────────── 恢复 (点击历史) ──────┘
```

### 6.4 Agent 详情面板（右栏）

右栏默认隐藏，展开后分为两个 Tab：

**Tab 1: Config（配置）**

| 字段 | 可编辑 | 说明 |
|------|--------|------|
| 名称 | ✅ | Agent 显示名 |
| 头像/图标 | ✅ | Emoji 或自定义图标 |
| 模型 | ✅ | Provider + Model 选择 |
| System Prompt | ✅ | 系统提示词编辑 |
| 工具列表 | ✅ | 启用/禁用各工具 |
| 记忆策略 | ✅ | 工作记忆/情景记忆/语义记忆开关 |
| 温度/MaxTokens | ✅ | 生成参数 |

**Tab 2: Sessions（会话历史）**

| 内容 | 说明 |
|------|------|
| 会话列表 | 按时间倒序，显示标题/时间/消息数，含已关闭 Tab 的会话 |
| 恢复 Chat | 点击某个已关闭的会话，将其重新打开为 Tab |
| 删除会话 | 永久删除会话及其消息 |

### 6.5 核心视图路由

由于采用 IM 范式，路由结构简化：

| 视图 | 路由 | 功能 |
|------|------|------|
| **Agent Chat** | `/agent/:agentId` (默认) | Agent 对话流（IM 主界面）|
| **Flow Editor** | `/flows` | FlowDSL 可视化编排 (P2) |
| **Settings** | `/settings` | 全局设置：凭据、网关、外观 |

Agent 的配置不再需要独立路由，集成在右栏面板中。

### 6.3 WebSocket 客户端 (`ws-client.ts`)

复用现有 `fastclaw-ws/1` 协议，封装为 TypeScript SDK：

```typescript
interface WsClient {
  // 连接管理
  connect(url: string, token?: string): Promise<void>;
  disconnect(): void;
  readonly connected: boolean;

  // RPC 调用（带 Promise）
  send(method: string, params?: Record<string, unknown>): Promise<WsResponse>;

  // Chat 流式交互
  chat(params: ChatParams): ChatStream;

  // 事件订阅
  on(event: 'chat.start' | 'chat.delta' | 'chat.tool.start' |
     'chat.tool.done' | 'chat.complete' | 'chat.error' |
     'connected' | 'disconnected', handler: Function): void;
}

interface ChatStream {
  // 当前请求 ID，用于取消
  readonly requestId: string;
  // 取消当前 chat（发送 chat.cancel）
  abort(): void;
  // 断开流监听但不取消后端生成（切换会话时使用）
  detach(): void;
}
```

**关键**：`detach()` 方法对应本次修改的 `detachStream()` 逻辑 — 切换会话时只断开前端监听，不发送 `chat.cancel`，让后端继续生成。

### 6.7 状态管理

FastClaw App 使用两个 Zustand Store 分层管理状态：

**`useGatewayStore` — 网关连接状态 (`store.ts`)**

```typescript
interface GatewayState {
  mode: 'embedded' | 'remote' | 'browser' | 'connecting';
  info: GatewayInfo | null;  // { port, wsUrl, httpUrl, version }
  connected: boolean;
  error: string | null;

  init(): Promise<void>;     // 检测环境 → 获取网关信息 → WS 连接 → 同步 Agent/Session
  setConnected(v: boolean): void;
}
```

**`useAgentStore` — Agent 与 Chat 状态 (`agent-store.ts`)**

```typescript
interface AgentState {
  // Agent 列表
  agents: AgentInfo[];
  activeAgentId: string;

  // 每个 Agent 的 Chat 列表（多 Tab）
  chats: Record<string, Chat[]>;        // agentId → Chat[]
  activeChatId: Record<string, string>; // agentId → 当前激活的 chatId

  // UI
  rightPanelOpen: boolean;
  theme: 'light' | 'dark' | 'system';

  // Chat 管理
  createChat(agentId: string, workDir?: string): void;
  closeChat(agentId: string, chatId: string): void;
  renameChat(agentId: string, chatId: string, name: string): void;
  reorderChats(agentId: string, fromIdx: number, toIdx: number): void;
  setActiveChat(agentId: string, chatId: string): void;

  // 消息管理
  addMessage(agentId: string, msg: Message): void;
  appendStreamDelta(agentId: string, delta: string): void;

  // 后端同步
  syncAgentsFromBackend(agents: BackendAgent[]): void;
  syncSessionsForAgent(agentId: string, sessions: BackendSession[]): void;
  loadChatStream(agentId: string, chatId: string, messages: BackendMessage[]): void;
  updateChatBackendId(agentId: string, localChatId: string, backendSessionId: string): void;
}

interface Chat {
  id: string;              // 对应后端 sessionId
  name: string;            // Tab 标签显示名
  workDir: string | null;  // 绑定的工作目录
  open: boolean;           // true=Tab 可见, false=已关闭（历史列表可恢复）
  messageCount: number;
  stream: StreamItem[];    // 该 Chat 的消息列表
}

type StreamItem = { role: 'user' | 'assistant'; content: string; timestamp: Date };
```

**状态流转要点**：

| 操作 | 状态变化 |
|------|---------|
| 切换 Agent | `activeAgentId` 更新 → 渲染对应 Agent 的 Chat Tabs → 恢复之前的活跃 Tab |
| 新建 Chat | `createChat()` → 新增 Chat 到 `chats[agentId]` → 自动切换到新 Tab |
| 关闭 Chat | `closeChat()` → `chat.open = false` → Tab 移除 → 自动激活相邻 Tab |
| 恢复 Chat | 右栏历史列表点击 → `chat.open = true` → Tab 重新出现 → 按需加载消息 |
| 发送消息 | `addMessage()` → 追加到当前 Chat 的 `stream` |
| 流式回复 | `appendStreamDelta()` → 实时更新消息内容 |
| Chat Tab 拖拽 | `reorderChats()` → 更新 `chats[agentId]` 数组顺序 |
| 后端同步 | `init()` → `syncAgentsFromBackend()` → `syncSessionsForAgent()` → 历史 Chat 就绪 |

### 6.6 UI 设计规范

| 维度 | 规范 |
|------|------|
| **窗口** | 无边框窗口 + 自定义标题栏（macOS 交通灯对齐）|
| **配色** | 深色主题为默认，支持浅色切换，跟随系统主题 |
| **字体** | 系统字体栈 + JetBrains Mono (代码) |
| **动画** | 消息出现 fade-in，流式打字效果，工具调用展开/折叠，会话分隔线动画 |
| **响应式** | 最小宽度 640px，左栏可折叠为图标模式，右栏默认隐藏 |
| **无障碍** | 键盘导航、ARIA 标签、高对比度模式 |
| **虚拟滚动** | 消息流使用虚拟滚动（react-virtuoso），支撑万级消息无卡顿 |
| **离线消息** | Agent 后台完成回复后，切换回来时消息自动补全到流中 |

**IM 特有设计**：

| 元素 | 设计 |
|------|------|
| **Agent 头像** | 左栏显示 Emoji 或自定义图标 + Agent 名称 + 最后活跃时间 |
| **未读标记** | Agent 在后台完成回复时，左栏该 Agent 显示未读圆点 |
| **会话分隔线** | 淡色横线 + "✨ 新会话 · 时间" 标签，视觉上区分但不打断阅读流 |
| **新话题按钮** | 输入栏左侧，点击后插入分隔线并重置上下文 |
| **搜索** | 顶栏搜索框，搜索时消息流进入高亮模式，上下箭头切换匹配项 |
| **@Agent** | 输入 @ 弹出 Agent 选择器，可在当前对话中 @其他 Agent 协作 (P2) |

---

## 7. 协议复用

### 7.1 WebSocket 协议 (`fastclaw-ws/1`)

App 完全复用现有 WebSocket 协议，不引入新协议：

**客户端 → 服务端**：

```json
{
  "id": "req_1",
  "method": "chat",
  "params": {
    "messages": [{ "role": "user", "content": "Hello" }],
    "sessionId": "session_abc",
    "agentId": "main"
  }
}
```

**服务端 → 客户端（流式事件）**：

| 事件类型 | 说明 |
|---------|------|
| `connected` | 连接建立，返回协议版本和可用方法列表 |
| `heartbeat` | 心跳保活 (30s 间隔) |
| `chat.start` | 回复开始 |
| `chat.delta` | 流式文本增量 |
| `chat.tool.start` | 工具调用开始 |
| `chat.tool.done` | 工具调用完成 |
| `chat.complete` | 回复完成，返回 sessionId |
| `chat.error` | 错误 |

**可用 RPC 方法**：

| 方法 | 说明 |
|------|------|
| `ping` | 连通性检查 |
| `auth` | API Key 认证 |
| `chat` | 发送消息并开始流式回复 |
| `chat.cancel` | 取消正在进行的回复 |
| `sessions.list` | 获取会话列表 |
| `sessions.get` | 获取会话详情 |
| `sessions.messages` | 获取会话消息历史 |
| `sessions.new` | 创建新会话 |
| `sessions.delete` | 删除会话 |
| `sessions.update_title` | 更新会话标题 |
| `agents` | 获取 Agent 列表 |
| `models.list` | 获取可用模型列表 |
| `config.get` | 获取配置 |
| `config.set` | 更新配置 |
| `subscribe` / `unsubscribe` | 事件订阅 |

### 7.2 REST API

| 端点 | 用途 |
|------|------|
| `GET /health` | 健康检查 |
| `GET /ready` | 就绪检查 |
| `GET /metrics` | Prometheus 指标 |
| `POST /api/v1/chat` | HTTP Chat (SSE 流式) |
| `GET /api/v1/agents` | Agent 列表 |
| `GET /api/v1/sessions` | Session 列表 |
| `POST /api/v1/dag/validate` | DAG 验证 |
| `POST /api/v1/dag/execute` | DAG 执行 |

---

## 8. 多入口统一

### 8.1 配置共享

所有入口共享同一套配置和数据目录：

```
~/.fastclaw/
├── config/
│   └── default.json        # 主配置文件
├── data/
│   └── sessions.db         # SQLite 数据库
├── logs/
│   └── gateway-daemon.log  # 守护进程日志
├── plugins/                # WASM 插件
└── daemon.pid              # 守护进程 PID 文件
```

### 8.2 端口冲突处理

| 场景 | App 行为 |
|------|---------|
| 端口空闲 | 绑定配置端口，正常启动 |
| 端口被 `fastclaw serve` 占用 | 检测到 FastClaw 健康响应 → 切换为**远程连接模式**，直接连接已运行的网关 |
| 端口被其他进程占用 | 自动选择随机端口，提示用户 |

```rust
async fn resolve_gateway(config: &Config) -> GatewayMode {
    let port = config.gateway.port;
    match check_fastclaw_health(port).await {
        Ok(true) => GatewayMode::Remote { port },
        _ => match try_bind(port).await {
            Ok(listener) => GatewayMode::Embedded { listener },
            Err(_) => {
                let listener = bind_random().await?;
                GatewayMode::Embedded { listener }
            }
        }
    }
}
```

### 8.3 CLI Agent Mode

除 GUI App 外，增加 CLI 入口的 Agent Mode：

```bash
# 新增子命令：嵌入网关 + 增强 TUI
fastclaw agent

# 等价于（但无需先启动网关）
fastclaw serve &
fastclaw tui
```

`fastclaw agent` 将复用 `EmbeddedGateway` 模块，在同一进程中启动网关并进入增强版 TUI。

---

## 9. 安全设计

### 9.1 嵌入式网关安全

| 措施 | 说明 |
|------|------|
| **仅本地绑定** | 网关绑定 `127.0.0.1`，外部网络无法访问 |
| **无需认证** | 嵌入模式下 API Key 认证可选（同进程通信无需额外认证）|
| **进程隔离** | App 退出时网关自动关停，不残留后台进程 |
| **WebView 沙箱** | Tauri WebView 遵守系统 WebView 沙箱策略 |

### 9.2 远程连接安全

| 措施 | 说明 |
|------|------|
| **API Key** | 连接远程网关时必须提供 Token |
| **TLS** | 远程连接强制 `wss://` 和 `https://` |
| **Token 存储** | 使用 OS Keychain 存储远程 Token |

### 9.3 Tauri Capability 声明

```json
{
  "identifier": "default",
  "description": "FastClaw App capabilities",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "global-shortcut:allow-register",
    "notification:default",
    "shell:allow-open",
    "updater:default",
    {
      "identifier": "http:default",
      "allow": [
        { "url": "http://127.0.0.1:*" },
        { "url": "ws://127.0.0.1:*" }
      ]
    }
  ]
}
```

---

## 10. 构建与发布

### 10.1 构建矩阵

| 平台 | 架构 | 输出格式 | CI |
|------|------|---------|-----|
| macOS | aarch64 (Apple Silicon) | `.dmg`, `.app` | GitHub Actions |
| macOS | x86_64 (Intel) | `.dmg`, `.app` | GitHub Actions |
| Windows | x86_64 | `.msi`, `.exe` | GitHub Actions |
| Linux | x86_64 | `.deb`, `.AppImage`, `.rpm` | GitHub Actions |
| Linux | aarch64 | `.deb`, `.AppImage` | GitHub Actions |

### 10.2 CI/CD 流水线

```
push to main
  │
  ├─ lint (clippy + eslint + prettier)
  ├─ test (cargo test + vitest)
  │
  └─ tag vX.Y.Z
       │
       ├─ build (5 platform targets)
       ├─ sign (macOS notarization, Windows Authenticode)
       ├─ package (dmg/msi/deb/AppImage)
       └─ publish (GitHub Releases + auto-updater endpoint)
```

### 10.3 自动更新

利用 Tauri 内置的 `tauri-plugin-updater`：

- 更新清单托管在 GitHub Releases
- App 启动时后台检查更新
- 用户确认后下载并替换，自动重启

---

## 11. 实施计划

### Phase 0: 脚手架 — ✅ 已完成

| 任务 | 描述 | 状态 |
|------|------|------|
| T-001 | Tauri 2.0 项目初始化 | ✅ |
| T-002 | 集成到现有 Cargo workspace | ✅ |
| T-003 | Vite + React + TypeScript + TailwindCSS 配置 | ✅ |
| T-004 | `EmbeddedGateway` — 嵌入式网关启动/关停 + 端口冲突回退 + invokeWithRetry | ✅ |
| T-005 | Tauri IPC Commands (`get_gateway_info`, `health_check`) + `__TAURI_INTERNALS__` 检测 | ✅ |
| T-006 | 前端连接验证 — IPC → WS URL → WebSocket → StatusBar 显示"已连接 · 内嵌" | ✅ |

### Phase 1: Agent Chat 核心 — ✅ 大部分完成

| 任务 | 描述 | 状态 |
|------|------|------|
| T-007 | `ws-client.ts` — 完整的 `fastclaw-ws/1` 协议客户端 | ✅ |
| T-008 | `AgentList` — 左栏 Agent 列表，头像/名称/活跃时间，切换 Agent | ✅ |
| T-009 | `MessageStream` — 虚拟滚动 (react-virtuoso) + Markdown 渲染 | ✅ |
| T-010 | `ChatTabs` — 多 Tab 标签页，拖拽排序、双击重命名、hover 关闭、溢出滚动 | ✅ |
| T-011 | `InputBar` — 输入栏 + `@` 引用 (文件/目录/Skill) + WorkDir 选择 | ✅ |
| T-012 | `ToolCallCard` — 工具调用展示（展开/折叠 + 参数 + 输出截断 + 状态动画 + 更多工具图标）| ✅ |
| T-013 | `detach` 语义 — 切换 Agent/Chat 时断开流监听但不取消后端生成 | ✅ |
| T-014 | 消息分页加载 — 向上滚动时懒加载历史消息（客户端分页，PAGE_SIZE=50） | ✅ |
| T-015 | Agent/Chat 切换时记忆和恢复滚动位置 | ✅ |

### Phase 2: 完整 Multi-Chat 体验 — 部分完成

| 任务 | 描述 | 状态 |
|------|------|------|
| T-016 | `AgentDetail` — 右栏 Agent 配置 + 会话历史列表（含恢复已关闭 Chat）| ✅ |
| T-017 | 会话历史恢复 — 右栏点击已关闭 Chat，重新打开为 Tab | ✅ |
| T-018 | 消息搜索 — Cmd/Ctrl+F 全文搜索 + 高亮 + 上下切换 | ✅ |
| T-019 | Settings 面板 — 模型配置（Provider/Model/API Key）+ Skill 配置 | ✅ |
| T-020 | 端口冲突自动检测 + 远程连接模式 + 浏览器模式 | ✅ |
| T-021 | 自定义标题栏（`decorations: false`，无边框窗口 + 拖拽 + 窗口控制 + 连接状态指示） | ✅ |
| T-022 | 深色/浅色主题 + 跟随系统 | ✅ |
| T-023 | 未读标记 — Agent 后台完成回复时左栏显示未读圆点 | ✅ |
| T-024 | 后端同步 — WS 对接真实 Agent/Session 数据 + 流式回复 + 历史消息加载 | ✅ |
| T-025 | 实时标题同步 — subscribe `sessions.changed` 事件，自动更新 Chat Tab 名称 | ✅ |

### Phase 3: 桌面增强 — 待实施

| 任务 | 描述 | 状态 |
|------|------|------|
| T-026 | 自定义 TitleBar 组件 — 无边框窗口标题栏 + 拖拽区域 + 窗口控制按钮 | ✅（同 T-021） |
| T-027 | 系统托盘 — 右键菜单（显示窗口/退出）+ 左键唤起 | ✅ |
| T-028 | 全局快捷键 — `Ctrl+Shift+Space` 切换窗口可见性，注册失败不阻断启动 | ✅ |
| T-029 | 系统通知 — Agent 任务完成通知 | ❌ |
| T-030 | 自动更新 — `tauri-plugin-updater` 集成 | ❌ |
| T-031 | `fastclaw app` CLI 入口子命令 | ❌ |

### Phase 4: 功能深化 — 待实施

| 任务 | 描述 | 状态 |
|------|------|------|
| T-032 | `ToolCallCard` 增强 — 展开/折叠、参数显示、输出预览、执行时间、状态动画 | ✅（同 T-012） |
| T-033 | `detach` 语义实现 — 切换 Tab/Agent 时保留后端流 | ✅（同 T-013） |
| T-034 | 消息分页加载 — 向上滚动懒加载 + `hasMore` 标记 | ❌ |
| T-035 | `fastclaw agent` CLI 子命令 — 嵌入式网关 + 增强 TUI | ❌ |

### Phase 5: Mobile 延伸 (P2)

| 任务 | 描述 | 状态 |
|------|------|------|
| T-036 | Tauri Mobile 构建配置 (iOS + Android) | ❌ |
| T-037 | 响应式 UI 适配 | ❌ |
| T-038 | 移动端嵌入式网关验证 | ❌ |

---

## 12. 风险与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| Tauri 2.0 Mobile 在部分设备上 WebView 行为不一致 | 中 | 中 | Desktop 优先，Mobile 作为 P2 延伸；针对具体设备做适配测试 |
| 嵌入式网关增加 App 启动时间 | 低 | 低 | 网关冷启动 < 100ms（已验证），异步启动不阻塞 UI 渲染 |
| 前端从原生 JS 迁移到 React 的工作量 | 中 | 中 | 不做逐行迁移，按功能模块重新实现，借此优化 UI/UX |
| 多入口共享同一 SQLite 数据库的并发安全 | 低 | 高 | SQLite WAL 模式天然支持多读单写；App 和 CLI 不会同时写入 |
| Tauri + Cargo workspace 编译时间增长 | 中 | 低 | 开发阶段使用 `cargo build -p fastclaw-app`，CI 并行构建 |

---

## 13. 术语表

| 术语 | 定义 |
|------|------|
| **嵌入式网关** | 在 App 进程内启动的 `fastclaw-gateway` 实例，无需独立进程 |
| **Agent Mode** | 交互式 AI Agent 体验，Agent 可调用工具、流式回复、多轮对话 |
| **Tauri IPC** | Tauri 框架提供的前端 WebView 与 Rust 后端之间的通信机制 |
| **detach** | 切换会话时断开前端流式监听但不取消后端生成的操作语义 |
| **fastclaw-ws/1** | FastClaw WebSocket 通信协议，JSON 文本帧，支持 RPC 和流式事件 |
| **Capability** | Tauri 2.0 的权限声明机制，定义 App 可访问的系统能力 |
