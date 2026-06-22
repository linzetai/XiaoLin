## ADDED Requirements

### Requirement: Markdown 文件预览

Files tab 中的 `.md` / `.mdx` 文件 SHALL 默认以渲染预览模式显示，支持切换到源码模式。

#### Scenario: 打开 Markdown 文件
- **WHEN** 用户打开一个 `.md` 或 `.mdx` 文件
- **THEN** 默认以渲染预览模式显示
- **AND** 支持 GFM（GitHub Flavored Markdown）：表格、任务列表、删除线、自动链接
- **AND** 代码块内有语法高亮

#### Scenario: 切换到源码模式
- **WHEN** 用户点击工具栏中的"源码"切换按钮
- **THEN** 切换为 CodeMirror 6 渲染的 Markdown 源码视图
- **AND** 有 Markdown 语法高亮
- **AND** 切换回"预览"恢复渲染模式

#### Scenario: 预览中的链接处理
- **WHEN** Markdown 预览中包含链接
- **THEN** 外部链接（http/https）通过 Tauri shell 在默认浏览器中打开
- **AND** 相对路径链接（如 `./other.md`）在 Files tab 中打开对应文件
- **AND** 锚点链接（`#section`）在预览内滚动到对应标题

#### Scenario: 预览中的图片
- **WHEN** Markdown 预览中包含本地图片引用
- **THEN** 通过 `read_binary_for_viewer` IPC 命令加载图片并显示
- **AND** 图片 SHALL 限制最大宽度为查看器宽度
- **AND** 点击图片调用 `openLightbox` 放大查看

### Requirement: Markdown 渲染技术

Markdown 预览 SHALL 复用现有的 `react-markdown` + `remark-gfm` 基础设施。

#### Scenario: 与 MarkdownContent 共享组件
- **WHEN** 渲染 Markdown 预览
- **THEN** 复用 `MarkdownContent.tsx` 中的 `components` 映射（`CodeBlock`、`PreBlock`、`Link`、`MarkdownImage`）
- **AND** 预览样式与 chat 消息中的 Markdown 保持一致
