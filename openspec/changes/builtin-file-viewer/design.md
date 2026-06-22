## Context

XiaoLin 是一个基于 Tauri 2 的 AI 助手桌面应用，前端 React 19 + Zustand 5 + Tailwind v4，后端 Rust。Chat 中 agent 通过 `xiaolin-tools-fs` crate 的工具（`write_file`/`edit_file`/`create_file` 等）在用户磁盘上创建和修改文件。

当前状态：
- `FileChangesCard` 聚合展示文件变更统计，点击触发 `xiaolin:open-review` 事件但**零监听方**
- `MarkdownContent` 中文件路径 inline code 有特殊样式（`.md-file-path`）但不可点击
- `WorkspacePanel` 有 tab 注册系统（`workspace-tabs.ts`），当前 5 个 tab（Plan/Review/Goal/Terminal/SubAgents）
- `rehype-highlight-lite.ts` 用 `lowlight` 注册了 17 种语言的语法高亮
- Tauri `fs:scope` 已覆盖 `$HOME/**`，`readFile` 已在 migration/avatar 等场景使用
- `ChatMeta.workDir` 记录每个 session 的工作目录
- 后端 `SessionFileTracker`（`file_persistence.rs`）已追踪 Created/Modified/Deleted 操作但仅用于 session 摘要，未暴露给前端

约束：
- 面板宽度 260-700px（默认 360px），分栏布局需考虑窄屏场景
- 文件可能很大，需分级处理策略
- 预留编辑模式扩展但 v1 为 readonly
- 后端已有 tools-fs 的文件操作追踪基础设施可复用

## Goals / Non-Goals

**Goals:**
- Agent 创建/修改的文件在 workspace panel 中自动出现并可查看
- 代码文件有语法高亮、行号、折叠、搜索能力
- Markdown 文件支持渲染预览和源码查看切换
- 图片文件可内嵌预览（缩放/拖拽）
- Chat 消息中的文件路径可点击，跳转到内置查看器
- 文件树可浏览工作目录下任意文件
- Artifact 记录持久化，页面刷新/重连后可恢复
- 选型预留编辑模式扩展能力

**Non-Goals:**
- v1 不提供文件编辑能力（CodeMirror 6 选型已预留）
- 不做跨 session 文件修改历史聚合
- 不追踪 `read_file` 操作（仅追踪写入/修改/删除）
- 不实现 diff 对比视图（Review tab 已有 git diff）
- 不实现全文件搜索/grep
- 不追踪终端命令产生的文件操作

## Decisions

### D1: 代码查看器引擎 — CodeMirror 6 (readonly)

**选择**: `@codemirror/view` + `@codemirror/state` + `@codemirror/language` + 各语言包

**替代方案**:
- **lowlight/highlight.js (`<pre>` + CSS)**：零新依赖，但无虚拟滚动、折叠、搜索，大文件性能差，无法扩展为编辑器
- **Monaco Editor**：功能最强，但 bundle ~2MB gzip，过重且难以嵌入窄面板
- **Shiki**：纯渲染高亮（更精确的 TextMate 语法），但无交互能力，不支持编辑

**理由**: CM6 是唯一同时满足"轻量（~150-250KB gzip lazy）+ 虚拟滚动 + readonly↔readwrite 切换"的方案。通过 `EditorState.readOnly.of(true)` 即可锁定只读，移除即启用编辑。语言包按需 `import()` 避免首屏负担。

**风险缓解**: 所有 CM6 代码通过 `React.lazy()` + `Suspense` 加载，首次打开 Files tab 才下载，不影响 chat 流的启动速度。

### D2: Artifact 追踪层 — 后端 SessionFileTracker 扩展 + WS event + SQLite

**选择**: 扩展已有的 `SessionFileTracker`（`crates/xiaolin-agent/src/runtime/file_persistence.rs`），在文件 I/O 成功后记录 `FileArtifact`，通过 gateway WS event `file_artifact` 推送前端，gateway SQLite 持久化。前端通过 WS op `artifacts.list` 查询（非 IPC，因为 SQLite 在 gateway 进程中）。

**替代方案**:
- **纯前端推导**：从 `stream-store` 的 tool results 提取 artifact 列表。零后端改动，但依赖消息数据完整性（compact/删除后丢失），且无法追踪非 tool 路径的操作。
- **独立新 crate**：新建 `xiaolin-artifacts` crate。过度设计，且 `SessionFileTracker` 已有 80% 基础。

**理由**: `SessionFileTracker` 已按 session 追踪 Created/Modified/Deleted，只缺 WS 推送和前端 store 对接。扩展它比新建更安全。SQLite 持久化保证刷新后恢复。注意：artifact 数据在 gateway SQLite 中，前端查询走 WS op，不走 Tauri IPC。

### D3: 文件打开事件 — 统一 CustomEvent

**选择**: `xiaolin:open-file` CustomEvent，payload `{ path: string; line?: number; workDir?: string; source?: string }`

**理由**: CustomEvent 是现有代码（`FileChangesCard`）已使用的模式，松耦合，任何组件都可 dispatch。Files tab 在 `useEffect` 中注册全局 listener。

### D4: 面板分栏布局 — 自适应折叠

**选择**: Files tab 内部左右分栏，文件列表默认 180px，点击文件后列表可折叠为 36px 图标条，查看器占满剩余空间。

**理由**: 面板宽度限制（260-700px），双栏在 360px 默认宽度下会过于拥挤。折叠模式让查看器获得 324px 有效宽度（360 - 36），接近可用。用户也可拖宽面板至 700px 获得更好体验。

### D5: 文件读取方式 — Tauri IPC readTextFile

**选择**: 自定义 Tauri IPC command `read_file_for_viewer`，在 Rust 后端做路径安全校验后读取文件。前端调用 `invoke("read_file_for_viewer", { path })` 而非直接使用 `@tauri-apps/plugin-fs`。

**替代方案**:
- **直接用 `@tauri-apps/plugin-fs` 的 `readTextFile`**：前端直调 plugin，但安全校验在前端做不可靠（规则 #29）
- **WS op `file.read`**：走 gateway 网络层，增加延迟和后端改动
- **直接用 agent 的 `read_file` 工具**：会被计入 agent 上下文，污染对话

**理由**: 自定义 Tauri command 在 Rust 侧做 `canonicalize` + `starts_with(allowed_dir)` + 大小检查，fail-closed（规则 #28/#29）。文件已在本地磁盘，IPC 最快最简。同理 `list_directory`、`file_metadata` 也是自定义 Tauri command。

### D6: 大文件分级策略

| 大小 | 策略 |
|------|------|
| < 500KB | 整体加载到 CM6 |
| 500KB - 5MB | 加载到 CM6（CM6 虚拟滚动自动处理渲染性能） |
| > 5MB | 提示文件过大，提供"在外部编辑器打开"按钮（`shell:allow-open`） |

**理由**: CM6 的虚拟滚动可处理百万行级文件的渲染，瓶颈在 IPC 传输。5MB 是 Tauri IPC 序列化的合理上限。

### D7: 文件类型路由

扩展名 → 渲染器映射：

```
.md / .mdx          → MarkdownViewer（可切换源码）
.png/.jpg/.gif/.webp/.svg → ImageViewer
.rs/.ts/.js/.py/... → CodeViewer (CM6 + 对应语言包)
其他                 → CodeViewer (纯文本模式)
```

语言包按需加载：`() => import("@codemirror/lang-rust")` 等。

### D8: 文件树浏览 — Tauri IPC list_directory

**选择**: 新增 `list_directory` IPC 命令，返回目录内容（名称 + 类型 + 大小）。前端懒加载：只在用户展开目录时请求子目录内容。

**安全约束**: 路径 canonicalize 后必须在 `workDir` 范围内（规则 #29），防止目录穿越。

### D9: 多文件 Tab — LRU 限制

文件查看器内部支持多 tab，LRU 策略：最多同时打开 10 个文件 tab，超出时自动关闭最久未访问的 tab。CM6 EditorView 实例在 tab 切换时保留状态（滚动位置、折叠状态）。

## Risks / Trade-offs

**[CodeMirror 6 bundle size]** → lazy import + code splitting，首次打开 Files tab 才加载。Vite 自动 chunk 分离。监控：构建后检查 chunk 大小不超过 300KB gzip。

**[IPC 大文件传输阻塞 UI]** → `readTextFile` 在 Rust 侧异步执行，Tauri IPC 传输不阻塞主线程。超大文件（>5MB）直接拒绝加载。前端显示 loading skeleton 避免白屏。

**[路径安全]** → 所有 IPC 命令（`read_file_for_viewer`/`list_directory`/`file_metadata`）统一安全策略：`canonicalize(path)` + `canonicalize(workDir)` → `starts_with(canonical_workDir)` → 大小检查（仅 read）。**不使用扩展名白名单**（会误伤无扩展名配置文件），改用 8KB NUL 字节检测区分二进制。Fail-closed：任何校验失败返回错误，不降级（规则 #28）。workDir 本身也必须 canonicalize 以防 symlink 逃逸。

**[CM6 语言包覆盖不全]** → 未注册语言的文件回退到纯文本模式。不尝试自动检测语言（误检比无高亮更差）。

**[面板宽度下查看体验]** → 默认 360px 偏窄，但 CM6 支持横向滚动和自动换行切换。文件列表可折叠为 36px 图标条释放空间。建议首次打开 Files tab 时自动将面板宽度扩展至 500px（如屏幕空间允许）。

**[与 Review tab 功能重叠]** → 职责明确划分：Review = git diff/commit 工作流，Files = 文件内容查看/预览。两者可互相跳转但不替代。

## Migration Plan

无需迁移。纯新增功能，不修改现有数据结构。DB 表 `file_artifacts` 是新表，对现有 schema 无影响。

## Resolved Questions

1. **CM6 主题跟随**：使用 `@codemirror/theme-one-dark` 为 dark 模式 + 自定义 light 主题（基于 CSS 变量映射 XiaoLin tokens），不做 accent 色映射（复杂度不值得）。
2. **文件树默认展开深度**：默认展开第一层目录。Session Artifacts 列表始终展开。
3. **artifact 通知策略**：Files tab 非活跃时显示 badge（计数）。已打开文件被修改时显示"文件已被修改"横幅 + "重新加载"按钮（不自动刷新，避免用户丢失阅读位置）。
