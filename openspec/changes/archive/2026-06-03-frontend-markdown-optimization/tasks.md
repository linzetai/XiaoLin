## 1. highlight.js 按需注册（highlight-on-demand）

- [x] 1.1 在 `MarkdownContent.tsx` 中将 `rehype-highlight` 默认导入替换为手动注册语言模式：导入 18 种常用语言（javascript, typescript, python, rust, bash, json, css, html, xml, sql, go, java, c, cpp, yaml, toml, markdown, diff），通过自定义 `rehype-highlight-lite` 插件传入
- [x] 1.2 创建 `rehype-highlight-lite.ts` 自定义插件，使用 `createLowlight` 直接注册语言子集，避免 `lowlight/common` 全量加载
- [x] 1.3 更新 `vite.config.ts` 的 `manualChunks`，将 `highlight.js/lib/languages/*` 打入 `highlight-langs` chunk，`lowlight` 归入 `vendor-highlight`
- [x] 1.4 执行 `pnpm build`，验证 vendor-highlight 从 159KB 降至 82KB（-48%），vendor-markdown 从 161KB 降至 157KB

## 2. 延迟高亮（idle-highlight）

- [x] 2.1 在 `MarkdownContent` 组件中新增 `highlighted` 状态，初始值 `false`（streaming 时为 `true`）
- [x] 2.2 添加 `useEffect`：当 `streaming === false` 且 `highlighted === false` 时，注册 `requestIdleCallback(() => setHighlighted(true))`，cleanup 中调用 `cancelIdleCallback`
- [x] 2.3 修改 `rehypePlugins` 的 `useMemo` 逻辑：当 `streaming === false` 时由 `highlighted` 决定是否包含 rehype-highlight；当 `streaming === true` 时保持现有 `unclosed` 逻辑不变
- [x] 2.4 验证：快速滚动历史消息时，代码块先显示纯文本，停留后约 50ms 内补全高亮，无闪烁感

## 3. Streaming 增量渲染（streaming-incremental-render）

- [x] 3.1 新建 `StreamingMarkdown.tsx`，实现 `StreamingMarkdown` 组件：按 `lastIndexOf('\n')` 拆分 content 为 frozen 和 active
- [x] 3.2 实现 `FrozenMarkdown` 子组件：用 `React.memo` 包裹，内部使用 `<MarkdownContent>` 渲染 frozen content（非 streaming 模式）
- [x] 3.3 实现 `ActiveLine` 子组件：纯文本 `<span>` 渲染 active line，匹配 `.markdown-body` 的字体和行高样式
- [x] 3.4 在拆分前调用 `hasUnclosedCodeBlock(frozen)` 检测：若 frozen 有未闭合代码块，退化为整体 `<MarkdownContent streaming />`
- [x] 3.5 在 `MessageRendererRow` 的 streaming 分支中，将 `<MarkdownContent content={segment.content} streaming />` 替换为 `<StreamingMarkdown content={segment.content} />`
- [x] 3.6 验证：streaming 长文本（2000+ 字）时，通过 React DevTools Profiler 确认 FrozenMarkdown 在无新行时不重渲染

## 4. 组件 Memo 强化

- [x] 4.1 将 `PreBlock` 改为 `memo(PreBlock, comparator)`，comparator 使用 `extractTextFromNode(prev.children) === extractTextFromNode(next.children)`
- [x] 4.2 将 `MessageRendererRow` 导出改为 `memo(MessageRendererRow, comparator)`：streaming 消息返回 `false`；已完成消息比较 `item`、`searchQuery`、`searchIdx`、`lastSegments` 引用
- [x] 4.3 验证：在 React DevTools Profiler 中滚动聊天列表，确认已完成消息的 `MessageRendererRow` 不出现在 render 列表中

## 5. 集成验证

- [x] 5.1 `pnpm build` 无报错，无 TypeScript 类型错误
- [x] 5.2 启动 `cargo tauri dev`，发送包含代码块的消息，验证 streaming 渲染和完成后高亮均正常
- [x] 5.3 测试边界场景：空内容、纯文本（无代码块）、超长代码块（500+ 行）、多语言混合代码块
- [x] 5.4 验证搜索高亮功能在 memo 优化后仍正常工作
- [x] 5.5 验证暗色主题和各 accent 主题下代码高亮颜色正确
