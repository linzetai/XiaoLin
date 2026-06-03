## Why

`MarkdownContent` 组件是聊天消息流中最重的渲染单元。当前实现基于 `react-markdown` + `rehype-highlight` 全量解析渲染，存在两个核心问题：

1. **Streaming 全量重解析**：每次 RAF 刷新 segment content 时，整条 `micromark → remark-gfm → rehype → rehype-highlight` 链路从头执行，单帧耗时 5-15ms，几乎占满 16.6ms 帧预算，导致长文本 streaming 卡顿。
2. **Bundle 浪费**：`highlight.js` 全量注册 189 种语言（156K 压缩），实际 AI 回答常用不超过 15 种；`react-markdown` 生态链（react-markdown + unified + remark + rehype）合计约 160K，首屏加载代价高。

这两个问题在消息量增长和长对话场景下持续恶化，是前端性能瓶颈的第二优先级（仅次于 Store 级联重渲染）。

## What Changes

- 引入 **streaming 增量渲染策略**：将 streaming 文本拆分为「已冻结段落」与「活跃行」，仅对活跃行做实时 parse，已冻结部分 memo 跳过
- **highlight.js 按需注册**：从全量 189 语言缩减为 ~15 种常用语言，不常见语言通过 `import()` 懒加载
- 代码块组件 **独立 memo**：`PreBlock` 基于代码文本内容做 shallow comparison，避免 react-markdown 重建 children 引用导致的无效重渲染
- **requestIdleCallback 延迟高亮**：历史消息先渲染纯文本 markdown，空闲时补充代码高亮，优化快速滚动体验
- 虚拟列表行 **MessageRendererRow memo 强化**：对已完成的消息行加入自定义 comparator，减少滚动时的 mount/unmount 开销

## Capabilities

### New Capabilities
- `streaming-incremental-render`: Streaming 场景下的增量 markdown 渲染策略，将 content 拆分为冻结段与活跃行，控制重解析范围
- `highlight-on-demand`: highlight.js 语言包按需注册与懒加载机制，减少 bundle 体积
- `idle-highlight`: 非 streaming 消息的延迟代码高亮策略，使用 requestIdleCallback 在空闲时补全高亮

### Modified Capabilities

## Impact

- **文件**：`MarkdownContent.tsx`、`MessageRenderer.tsx`、`MessageStream.tsx`、`vite.config.ts`（manualChunks 调整）
- **依赖**：可能新增 `lowlight`（highlight.js 的轻量接口）替代默认 highlight.js 全量加载；或调整 `rehype-highlight` 配置
- **Bundle**：预计减少 ~100-120K 压缩后体积（highlight.js 全量 → 按需）
- **渲染性能**：streaming 场景帧内 markdown parse 耗时从 5-15ms 降至 1-3ms；历史消息首次渲染延迟高亮可降低 mount 时间 50%+
