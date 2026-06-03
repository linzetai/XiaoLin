## ADDED Requirements

### Requirement: Streaming content SHALL be split into frozen and active parts
当 `streaming` 为 `true` 时，`StreamingMarkdown` 组件 SHALL 将 `content` 按最后一个换行符拆分为两部分：
- **Frozen 部分**：最后一个 `\n` 之前的所有内容，由 `FrozenMarkdown` 渲染
- **Active 部分**：最后一个 `\n` 之后的内容（当前正在流入的行），用轻量纯文本渲染

#### Scenario: Normal streaming with multiple lines
- **WHEN** streaming content 为 `"# Title\n\nHello world\nThis is"`
- **THEN** frozen 部分为 `"# Title\n\nHello world"`，active 部分为 `"This is"`
- **THEN** frozen 部分使用完整 markdown 解析渲染（含 remark-gfm）
- **THEN** active 部分使用纯文本 span 渲染，不经过 react-markdown

#### Scenario: Content has no newline yet
- **WHEN** streaming content 为 `"Hello world"`（无换行符）
- **THEN** frozen 部分为空字符串，不渲染 FrozenMarkdown
- **THEN** active 部分为 `"Hello world"`，纯文本渲染

#### Scenario: Content ends with newline
- **WHEN** streaming content 为 `"Line one\nLine two\n"`
- **THEN** frozen 部分为 `"Line one\nLine two"`
- **THEN** active 部分为空字符串，不渲染 ActiveLine

### Requirement: FrozenMarkdown SHALL be memoized on content value
`FrozenMarkdown` 组件 SHALL 使用 `React.memo` 包裹，当 `content` 字符串值不变时跳过重渲染。

#### Scenario: Same frozen content across frames
- **WHEN** 连续两帧的 frozen content 都是 `"# Title\n\nParagraph"`（active 部分在变化）
- **THEN** `FrozenMarkdown` 不重新执行 react-markdown 解析

#### Scenario: New line appended to frozen content
- **WHEN** 上一帧 frozen 为 `"Line 1"` 且当前帧 frozen 变为 `"Line 1\nLine 2"`
- **THEN** `FrozenMarkdown` 重新执行 react-markdown 解析

### Requirement: Unclosed code block SHALL prevent splitting
当 frozen 部分包含未闭合的代码块（奇数个 ` ``` `）时，SHALL 放弃拆分，将整个 content 作为单一 MarkdownContent 渲染（退化为当前行为）。

#### Scenario: Unclosed code block in frozen part
- **WHEN** streaming content 为 `` "Some text\n```python\ndef foo():\n  return 1" ``
- **THEN** 检测到 frozen 部分有未闭合代码块
- **THEN** 不拆分，整个 content 使用 `MarkdownContent` streaming 模式渲染

#### Scenario: Closed code block in frozen part
- **WHEN** streaming content 为 `` "```python\ndef foo():\n  return 1\n```\n\nNext text" ``
- **THEN** frozen 部分代码块已闭合，正常拆分
- **THEN** frozen 使用 FrozenMarkdown，active 使用 ActiveLine

### Requirement: StreamingMarkdown SHALL replace MarkdownContent in streaming branch
在 `MessageRendererRow` 的 streaming 分支中，对于 `type === "text"` 的 grouped segment，SHALL 使用 `StreamingMarkdown` 替代 `<MarkdownContent streaming />`。

#### Scenario: Streaming text segment rendering
- **WHEN** streaming 分支遍历到 `group.type === "text"` 的 segment
- **THEN** 渲染 `<StreamingMarkdown content={group.segment.content} />` 而非 `<MarkdownContent streaming />`

#### Scenario: Non-streaming messages unchanged
- **WHEN** 渲染已完成的 AI 消息（非 streaming）
- **THEN** 仍然使用 `<MarkdownContent content={msg.content} />`（无 streaming prop）
