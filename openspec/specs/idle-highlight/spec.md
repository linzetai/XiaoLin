## ADDED Requirements

### Requirement: Non-streaming MarkdownContent SHALL defer code highlighting
当 `streaming` 为 `false`（或未指定）时，`MarkdownContent` SHALL 首次渲染时不执行 `rehype-highlight`，而是使用 `requestIdleCallback` 在浏览器空闲时设置 `highlighted` 状态为 `true` 后再重渲染以补全高亮。

#### Scenario: Initial mount without highlight
- **WHEN** 一条已完成的 AI 消息的 `MarkdownContent` 首次 mount
- **THEN** 首次渲染不包含 `rehype-highlight` 插件
- **THEN** 代码块以等宽字体显示纯文本内容
- **THEN** 文本内容完整、可读、可选中复制

#### Scenario: Idle callback triggers highlight
- **WHEN** 浏览器进入空闲期且 `MarkdownContent` 仍然 mounted
- **THEN** `requestIdleCallback` 触发，设置 `highlighted = true`
- **THEN** 组件重渲染，此次包含 `rehype-highlight` 插件
- **THEN** 代码块显示语法高亮

#### Scenario: Component unmounts before idle callback
- **WHEN** 用户快速滚动，`MarkdownContent` 在 idle callback 触发前 unmount
- **THEN** 通过 `cancelIdleCallback` 取消待执行的回调
- **THEN** 无内存泄漏，无 setState on unmounted 警告

### Requirement: Streaming content SHALL always skip idle deferral
当 `streaming` 为 `true` 时，`MarkdownContent` SHALL 直接按现有逻辑决定是否使用 `rehype-highlight`（基于 `hasUnclosedCodeBlock` 检测），不使用 idle 延迟。

#### Scenario: Streaming mode bypass
- **WHEN** `<MarkdownContent content="..." streaming />` 渲染
- **THEN** 不注册 `requestIdleCallback`
- **THEN** `rehypePlugins` 由 `unclosed` 状态直接决定（闭合 → 含 highlight，未闭合 → 不含 highlight）

### Requirement: MessageRendererRow SHALL be wrapped with React.memo
`MessageRendererRow` SHALL 使用 `React.memo` 包裹，附带自定义 comparator。对于已完成的消息（非 streaming），当以下条件全部满足时跳过重渲染：
- `item` 引用相同
- `searchQuery` 相同
- `searchIdx` 相同
- `lastSegments` 引用相同

#### Scenario: Completed message with unchanged props
- **WHEN** 虚拟列表重渲染，某条已完成消息的 `item`、`searchQuery`、`searchIdx`、`lastSegments` 引用均不变
- **THEN** `MessageRendererRow` 跳过重渲染

#### Scenario: Streaming message never skips
- **WHEN** 当前消息为 streaming 状态（`item.data.role === "streaming"`）
- **THEN** comparator 返回 `false`，允许每帧重渲染

#### Scenario: Search query changes
- **WHEN** 用户修改搜索关键词，`searchQuery` 变化
- **THEN** comparator 返回 `false`，触发重渲染以更新高亮

### Requirement: PreBlock SHALL be memoized based on code text content
`PreBlock` 组件 SHALL 使用 `React.memo` 包裹，自定义 comparator 基于 `extractTextFromNode(children)` 比较代码文本内容。当文本内容相同时跳过重渲染。

#### Scenario: Same code content with different React element references
- **WHEN** `react-markdown` 两次渲染生成的 `PreBlock` children 引用不同，但代码文本内容相同
- **THEN** `PreBlock` 跳过重渲染，不重新执行 highlight

#### Scenario: Code content changes
- **WHEN** streaming 导致代码块内容追加了新行
- **THEN** `extractTextFromNode` 检测到文本变化
- **THEN** `PreBlock` 重新渲染并重新高亮
