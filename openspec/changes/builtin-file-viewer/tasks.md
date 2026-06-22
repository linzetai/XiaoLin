## 1. 后端 Artifact 追踪基础设施

- [x] 1.1 定义 `FileArtifact` 数据模型（在 `xiaolin-agent/src/runtime/` 中，扩展已有 `SessionFileTracker`）：`session_id`, `path`, `operation` (created/modified/deleted), `timestamp`, `tool_call_id`, `bytes`
- [x] 1.2 创建 `file_artifacts` SQLite 表（migration），字段对应 `FileArtifact`，含索引 `idx_session_id`
- [x] 1.3 实现 `ArtifactStore` trait：`record_artifact()`, `get_session_artifacts()`, `delete_session_artifacts()`
- [x] 1.4 在 `write_file`/`edit_file`/`create_file`/`multi_edit`/`apply_patch`/`str_replace_editor` 工具成功回调中注入 `record_artifact()` 调用
- [x] 1.5 在 `xiaolin-protocol` 中新增：`FileArtifactEvent` 事件类型 + `artifacts.list` ClientOp variant + parse_request
- [x] 1.6 在 `xiaolin-protocol/lib.rs` 中 pub use 导出新类型
- [x] 1.7 在 `xiaolin-gateway/ws/mod.rs` 中注册 `artifacts.list` dispatch 分支
- [x] 1.8 实现 `xiaolin-gateway/ws/artifact.rs` handler：查询 SQLite 返回 artifact 列表
- [x] 1.9 实现 `file_artifact` WS event 推送逻辑：agent runtime 记录 artifact → channel/callback → gateway WS dispatcher 推送到前端
- [x] 1.10 session 删除时级联删除对应的 artifact 记录
- [x] 1.11 在 `transport.ts` 中添加 `listArtifacts(sessionId)` 函数 + `FileArtifact` interface
- [x] 1.12 在 `api.ts` 中添加 `listSessionArtifacts()` 高层包装

## 2. Tauri IPC 命令

- [x] 2.1 实现 `read_file_for_viewer` IPC 命令：`canonicalize(path)` + `canonicalize(workDir)` → `starts_with(canonical_workDir)` + 大小检查（>5MB 拒绝）+ 8KB NUL 字节检测（二进制拒绝）+ `std::fs::read_to_string`
- [x] 2.2 实现 `list_directory` IPC 命令：路径安全校验 + readDir + 过滤隐藏文件和大型目录（node_modules/target/.git 等）+ 排序（目录优先）
- [x] 2.3 实现 `file_metadata` IPC 命令：路径安全校验 + stat（size, modifiedAt, isDirectory）
- [x] 2.4 实现 `artifacts.list` WS op（gateway 侧）：查询 gateway SQLite 返回 artifact 列表（注：artifact 在 gateway 进程中，不走 IPC）
- [x] 2.5 实现 `read_binary_for_viewer` IPC 命令：与 `read_file_for_viewer` 共享路径安全校验，返回 base64 编码的二进制内容（用于图片预览）
- [x] 2.6 更新 `capabilities/default.json`：确认权限覆盖，注册新 IPC commands（自定义 command 无需 capabilities 注册；现有 fs:scope 已覆盖 $HOME/**）

## 3. 前端 Store 和事件系统

- [x] 3.1 创建 `file-viewer-store.ts`（Zustand）：`openFiles: Map<string, OpenFile>`, `activeFilePath`, `artifacts: FileArtifact[]`, `viewMode: "code" | "preview"`, `fileListCollapsed: boolean`
- [x] 3.2 实现 `openFile(path, line?)` action：解析路径 → `invoke("read_file_for_viewer")` → 添加到 openFiles → 设为 active
- [x] 3.3 实现 `closeFile(path)` action：移除 tab + LRU 选择下一个 active
- [x] 3.4 注册 `file_artifact` WS event 监听，更新 artifacts 列表
- [x] 3.5 实现 `xiaolin:open-file` 全局 CustomEvent 监听，调用 `openFile` + `setActiveTab("files")`
- [x] 3.6 实现路径解析工具函数 `resolveFilePath(path, workDir)`：处理绝对/相对路径
- [x] 3.7 实现 session 切换时 store 隔离：`switchSession` 时保存当前 openFiles/artifacts，加载新 session 的 artifacts（调用 `artifacts.list` WS op）
- [x] 3.8 定义 `filesClosedByUser` 状态 + setter（类似 `planClosedByUser`），session 切换时重置。行为接入（调用 setter 的 UI 逻辑）在 Phase 7/9 实现

## 4. CodeMirror 6 代码查看器

- [x] 4.1 安装 CM6 依赖：`@codemirror/view`, `@codemirror/state`, `@codemirror/language`, `@codemirror/search`, `@codemirror/commands`, `@codemirror/language-data`
- [x] 4.2 创建 `CodeViewer.tsx`：React 封装 CM6 EditorView，readonly 模式，接收 `content`/`language`/`line` props
- [x] 4.3 实现文件扩展名 → CM6 语言包映射表 + 按需 `import()` 加载
- [x] 4.4 实现 light/dark 主题跟随：监听 XiaoLin theme 变化，切换 CM6 theme extension
- [x] 4.5 实现行高亮跳转：收到 `line` prop 时滚动到指定行并添加临时高亮效果
- [x] 4.6 实现搜索面板：CM6 `@codemirror/search` 的 `searchKeymap` 绑定
- [x] 4.7 实现代码折叠：CM6 `foldGutter()` + `foldKeymap` 绑定
- [x] 4.8 用 `React.lazy()` + `Suspense` 包装 CodeViewer，实现 lazy import

## 5. Markdown 预览器

- [x] 5.1 创建 `MarkdownViewer.tsx`：复用 `MarkdownContent` 组件 + 源码/预览切换
- [x] 5.2 预览模式：`react-markdown` + `remarkGfm` + `rehypeHighlightLite`
- [x] 5.3 源码模式：CodeMirror 6 + Markdown 语言包
- [x] 5.4 处理 Markdown 内相对路径链接：点击在 Files tab 中打开
- [x] 5.5 处理 Markdown 内本地图片引用：通过 `read_binary_for_viewer` IPC 命令加载（统一安全校验）

## 6. 图片查看器

- [x] 6.1 创建 `ImageViewer.tsx`：通过新增 `read_binary_for_viewer` IPC 命令加载图片为 base64 → blob URL（复用 `read_file_for_viewer` 的路径安全校验，但返回二进制而非文本）
- [x] 6.2 实现缩放功能：鼠标滚轮缩放（以鼠标位置为锚点），10%-500% 范围
- [x] 6.3 实现拖拽平移：放大后鼠标拖拽移动
- [x] 6.4 实现工具栏控制：适应窗口、原始大小、当前缩放比例显示
- [x] 6.5 SVG 文件支持：图片预览/源码查看切换

## 7. Files Tab 和分栏布局

- [x] 7.1 创建 `FileViewerTab.tsx`：Files tab 主组件，分栏布局容器
- [x] 7.2 在 `AppShell.tsx` 中注册 Files tab（`registerTab({ id: "files", ... })`），完整 order 表：Plan=0, Review=1, Files=2, Goal=3, Terminal=4, SubAgents=5
- [x] 7.3 实现分栏布局：左侧文件列表（180px / 可折叠为 36px）+ 右侧查看器
- [x] 7.4 实现自适应折叠：面板宽度 < 400px 时自动折叠文件列表为 36px 图标条
- [x] 7.4b 实现折叠后 overlay 展开：点击图标条弹出 overlay 文件列表（不挤压查看器），点击外部关闭
- [x] 7.5 创建 `FileTabBar.tsx`：查看器区域上方的多文件 tab 栏（切换/关闭/LRU）
- [x] 7.6 创建 `FileViewer.tsx`：按文件类型路由到 CodeViewer/MarkdownViewer/ImageViewer
- [x] 7.7 创建 `FileToolbar.tsx`：文件名 + 复制内容 + 在外部打开 + 自动换行切换 + 预览/源码切换
- [x] 7.8 实现空状态 UI：无文件时显示引导提示
- [x] 7.9 实现首次打开自动扩展面板宽度至 500px

## 8. 文件树浏览器

- [x] 8.1 创建 `FileTree.tsx`：递归树组件，懒加载子目录
- [x] 8.2 实现 Session Artifacts 区域：在文件树上方显示 agent 操作过的文件列表，带操作类型标识（C/M）
- [x] 8.3 实现隐藏文件/大型目录过滤逻辑
- [x] 8.4 实现文件类型图标映射（扩展名 → Phosphor 图标 + 颜色）
- [x] 8.5 点击文件时调用 `fileViewerStore.openFile()`

## 9. Chat 集成

- [x] 9.1 修改 `FileChangesCard.tsx`：`xiaolin:open-review` → `xiaolin:open-file`
- [x] 9.2 修改 `MarkdownContent.tsx` 中的 `CodeBlock`：`.md-file-path` 添加 `onClick` handler，dispatch `xiaolin:open-file`
- [x] 9.3 修改 `DiffCard.tsx`：文件名行添加"查看完整文件"图标按钮
- [x] 9.4 实现 `file_artifact` WS event 触发 Files tab auto-open：`AppShell.tsx` 中监听事件 → 自动 `setActiveTab("files")`
- [x] 9.5 Files tab badge：agent 操作文件时非活跃状态显示 badge 计数

## 10. 文件变更通知和刷新

- [x] 10.1 实现"文件已被修改"横幅通知：当 `file_artifact` event 的 path 匹配已打开文件时，在查看器上方显示横幅
- [x] 10.2 实现"重新加载"按钮：重新读取文件内容并更新 CM6 EditorView（保留滚动位置）
- [x] 10.3 实现二进制文件检测：读取前 8KB 检测 NUL 字节，二进制文件显示"无法预览"提示

## 11. 集成测试和优化

- [ ] 11.1 验证 CM6 lazy import 不影响应用启动速度（检查 main chunk 大小）
- [ ] 11.2 验证大文件处理：500KB / 1MB / 5MB / >5MB 各级别文件的加载行为
- [ ] 11.3 验证路径安全：尝试读取 workDir 外的文件被拒绝
- [ ] 11.4 验证 tab 切换时 CM6 EditorView 状态保持（滚动位置、折叠、搜索）
- [ ] 11.5 验证页面刷新后 artifact 列表从 SQLite 恢复
- [ ] 11.6 验证 light/dark 主题切换时 CM6 主题跟随
- [ ] 11.7 内存泄漏检查：反复打开/关闭文件 tab，检查 EditorView 是否正确 destroy
- [ ] 11.8 验证二进制文件检测正确（不把二进制内容加载到 CM6）
