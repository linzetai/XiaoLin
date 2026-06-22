## Why

Chat 中 agent 通过工具调用（`write_file`/`edit_file`/`create_file` 等）创建和修改文件，但用户无法在 XiaoLin 内直接查看这些文件的内容。必须切换到外部编辑器确认结果，严重打断工作流。同时 `FileChangesCard` 中的文件点击事件（`xiaolin:open-review`）从未有监听方——这是一个功能断裂。

## What Changes

- 新增 **Files** workspace tab：在 WorkspacePanel 中注册新标签页，内含 Session Artifact 列表 + 文件树浏览 + 内置文件查看器
- 新增 **CodeMirror 6 代码查看器**：readonly 模式，支持语法高亮、行号、折叠、搜索、虚拟滚动，预留编辑模式扩展
- 新增 **Markdown 预览器**：复用 `react-markdown` + `remark-gfm`，支持源码/预览切换
- 新增 **图片查看器**：内嵌预览，支持缩放/拖拽
- 新增 **后端 Artifact 追踪**：`tools-fs` 层记录文件操作，gateway 通过 WS event 推送前端，SQLite 持久化
- 新增 **统一文件打开事件**：`xiaolin:open-file` 替代断裂的 `xiaolin:open-review`
- 修改 **FileChangesCard**：文件点击改为触发 `xiaolin:open-file`，在 Files tab 中打开
- 修改 **MarkdownContent**：文件路径 inline code 变为可点击链接
- 修改 **DiffCard**：新增"查看完整文件"操作
- 新增 **Tauri IPC 命令**：`read_file_for_viewer`、`read_binary_for_viewer`、`list_directory`、`file_metadata`（本地文件操作，在 src-tauri 中实现）
- 新增 **WS op**：`artifacts.list`（查询 gateway SQLite 中的 artifact 记录）
- 新增 **DB 表**：`file_artifacts` 持久化 artifact 记录
- 新增 **前端 Store**：`file-viewer-store.ts`（Zustand）管理打开的文件、视图模式、artifact 列表

## Capabilities

### New Capabilities
- `file-viewer-engine`: CodeMirror 6 代码查看器引擎，支持语法高亮、行号、折叠、搜索，readonly 模式预留编辑扩展
- `file-viewer-markdown`: Markdown 文件预览器，支持 GFM 渲染和源码/预览模式切换
- `file-viewer-image`: 图片文件内嵌查看器，支持缩放和拖拽
- `file-viewer-tab`: Workspace Panel 中的 Files 标签页，包含分栏布局（文件列表 + 查看器）、多文件 Tab 切换
- `artifact-tracking`: 后端文件操作追踪系统，记录 agent 在 session 中创建/修改/删除的文件，WS 推送 + SQLite 持久化
- `file-tree-browser`: 文件树浏览器，懒加载用户工作目录下的文件结构
- `chat-file-links`: Chat 消息中的文件路径可点击，跳转到内置文件查看器

### Modified Capabilities
- `file-changes-card`: 文件点击从断裂的 `xiaolin:open-review` 改为 `xiaolin:open-file`，在 Files tab 中打开文件
- `workspace-panel`: 新增 Files tab 注册，auto-open 行为扩展为在 agent 产生文件变更时自动打开 Files tab

## Impact

- **前端新依赖**：`@codemirror/view`、`@codemirror/state`、`@codemirror/language`、`@codemirror/search` + 各语言包（lazy import，~150-250KB gzip）
- **Rust crates 变更**：`xiaolin-agent`（artifact 记录，扩展 SessionFileTracker）、`xiaolin-gateway`（WS event handler + `artifacts.list` WS op）、`xiaolin-protocol`（新增 event 类型 + ClientOp）、`xiaolin-app/src-tauri`（IPC commands: `read_file_for_viewer`/`list_directory`/`file_metadata`）
- **DB schema**：新增 `file_artifacts` 表
- **Tauri capabilities**：当前 `fs:scope` 已覆盖 `$HOME/**`，无需修改；需新增 `fs:allow-stat`、`fs:allow-read-dir` 权限（`read-dir` 已有）
- **现有组件修改**：`FileChangesCard`、`MarkdownContent`、`DiffCard`、`AppShell`（tab 注册）、`workspace-tabs.ts`（auto-open 逻辑）
