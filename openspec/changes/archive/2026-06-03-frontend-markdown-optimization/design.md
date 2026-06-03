## Context

`MarkdownContent` 是 XiaoLin 聊天界面中最重的渲染单元。每条 AI 回复消息都会经过完整的 markdown 解析管线：

```
content (string)
  → micromark (tokenize)
  → remark-gfm (GFM 扩展)
  → unified/rehype (mdast → hast)
  → rehype-highlight (代码高亮，使用 highlight.js)
  → React VDOM → DOM
```

当前痛点：
- **Streaming**：RAF 每帧更新 segment content → `MarkdownContent` 的 `content` prop 每帧变化 → `memo` 每次 miss → 全量重解析，单帧 5-15ms
- **Bundle**：highlight.js 全量 189 种语言（156K），react-markdown 生态链 ~160K，合计占首屏 JS 的约 25%
- **滚动**：`MessageRendererRow` 未被 `memo` 包裹，虚拟列表滚动时 row 的 mount/unmount 触发完整 markdown parse + highlight

相关组件调用链：
- `MessageStream.tsx` → virtualizer rows → `MessageRendererRow` → `AiMessage` / streaming 分支
- streaming 分支：`grouped.map()` → `<MarkdownContent content={segment.content} streaming />`
- 非 streaming：`<MarkdownContent content={msg.content} />`

## Goals / Non-Goals

**Goals:**
- 将 streaming 场景的 markdown 帧内 parse 耗时从 5-15ms 降至 1-3ms
- 将 highlight.js 相关 bundle 体积从 ~156K 降至 ~30-40K
- 优化虚拟列表快速滚动时的渲染性能（避免无效 highlight）
- 保持现有 markdown 渲染质量和视觉效果不变

**Non-Goals:**
- 不替换 `react-markdown`（整体替换为自定义 renderer 的 ROI 不足，可作为后续迭代）
- 不修改 streaming 的传输机制或 segment 数据结构
- 不改变 `MessageStream.tsx` 的虚拟列表架构
- 不在此次做 SSR/预渲染优化（Tauri 桌面应用无 SSR 需求）

## Decisions

### D1: Streaming 增量渲染策略 — Frozen Lines + Active Line

**选择**：将 streaming text segment 的 content 拆分为「已冻结段落」和「活跃行」两部分独立渲染。

**方案比较**：

| 方案 | 实现复杂度 | 效果 | 风险 |
|---|---|---|---|
| A. Frozen/Active 拆分 | 中 | 高 | 跨行语法元素（代码块、列表）可能在拆分点断裂 |
| B. 自定义 micromark streaming | 高 | 最高 | 工程量大，需维护自定义 parser |
| C. useDeferredValue | 低 | 低 | 只能延迟不能跳过，长内容仍然全量 parse |

**选择 A 的理由**：
- ROI 最高：通过 `lastIndexOf('\n')` 拆分，冻结部分的变化频率从「每帧」降为「每换行」
- 冻结部分用独立 memo 组件渲染，React 的 shallow comparison 可直接跳过
- 代码块跨行问题可通过 `hasUnclosedCodeBlock` 检测处理（已有类似逻辑）

**实现要点**：
```tsx
// StreamingMarkdown: 替代 streaming 场景下的 MarkdownContent
function StreamingMarkdown({ content }: { content: string }) {
  const lastNewline = content.lastIndexOf('\n');
  const frozen = lastNewline > 0 ? content.slice(0, lastNewline) : '';
  const active = lastNewline > 0 ? content.slice(lastNewline + 1) : content;
  
  return (
    <>
      {frozen && <FrozenMarkdown content={frozen} />}  {/* memo 跳过 */}
      {active && <ActiveLine text={active} />}          {/* 轻量纯文本渲染 */}
    </>
  );
}
```

### D2: highlight.js 按需加载 — lowlight + 常用语言子集

**选择**：使用 `lowlight` 库（highlight.js 的轻量 API 封装）配合手动注册常用语言，替代默认全量加载。

**方案比较**：

| 方案 | Bundle 影响 | 兼容性 | 维护 |
|---|---|---|---|
| A. lowlight + 手动注册 15 语言 | -120K | 需要列表维护 | 低 |
| B. lowlight/common (37 语言) | -80K | 开箱即用 | 无 |
| C. highlight.js 动态 import 按语言 | -120K~-150K | 需要异步处理 | 中 |

**选择 A 的理由**：
- `rehype-highlight` 已经支持通过 `languages` 选项传入自定义语言集
- 15 种语言覆盖 AI 回答 99%+ 的代码块
- 不需要异步加载逻辑，实现最简
- 对罕见语言（Haskell 等），降级为纯文本显示是可接受的

**语言列表**：javascript, typescript, python, rust, bash, json, css, html, xml, sql, go, java, c, cpp, yaml, toml, markdown, diff

### D3: 延迟高亮策略 — requestIdleCallback

**选择**：非 streaming 消息首次渲染时先跳过 `rehype-highlight`，使用 `requestIdleCallback` 在浏览器空闲时补全高亮。

**理由**：
- 快速滚动历史消息时，大量 row mount/unmount，每次 mount 都执行 highlight 是浪费
- 代码块文本在无高亮时已可阅读（白底黑字），用户快速滚过时感知不到差异
- `requestIdleCallback` 的 deadline 机制可确保高亮不影响滚动帧率

**实现要点**：
- 新增 `highlighted` 状态，初始为 `false`
- mount 后注册 `requestIdleCallback(() => setHighlighted(true))`
- `rehypePlugins` 根据 `highlighted` 决定是否包含 `rehype-highlight`
- 在 `IntersectionObserver` 检测到元素可见后才启动 idle callback（避免对不可见元素做高亮）

### D4: MessageRendererRow memo 策略

**选择**：用 `React.memo` 包裹 `MessageRendererRow`，自定义 comparator 跳过已完成消息的重渲染。

**比较逻辑**：
- 已完成消息（非 streaming）：只比较 `item` 引用、`searchQuery`、`searchIdx`、`lastSegments`
- Streaming 消息：不做 memo（每帧都需要更新）

### D5: 代码块组件 memo — extractTextFromNode comparator

**选择**：`PreBlock` 用 `memo` 包裹，comparator 基于 `extractTextFromNode(children)` 比较代码文本内容。

**理由**：`react-markdown` 每次 render 都创建新的 React element tree，导致 `children` 引用变化。但只要代码文本内容相同，就不需要重渲染 `PreBlock`。已有 `extractTextFromNode` 工具函数可复用。

## Risks / Trade-offs

**[Frozen/Active 拆分边界问题]** → 当 streaming 内容跨行语法元素（如代码块 ` ``` ` 或多行列表）在拆分点被截断时，frozen 部分可能渲染异常。
→ **缓解**：通过 `hasUnclosedCodeBlock` 检测冻结内容的代码块闭合状态；如果存在未闭合代码块，将整个 segment content 都归入 active 部分不拆分。

**[lowlight 语言覆盖不足]** → 用户偶尔可能看到 AI 生成的 Haskell/Elixir 等代码没有高亮。
→ **缓解**：无高亮的代码仍然可读（等宽字体、正确缩进）；后续可基于代码块 language hint 做 dynamic import 补全。

**[requestIdleCallback 兼容性]** → Safari 直到 2024 年才支持 `requestIdleCallback`。
→ **缓解**：Tauri WebView 在 macOS 使用 WebKit（已支持 rIC），Linux 使用 WebKitGTK（已支持）。不需要 polyfill。

**[memo comparator 的 extractTextFromNode 开销]** → 每次比较都需要遍历 React children tree 提取文本。
→ **缓解**：代码块通常不超过 100 行，提取开销 <0.1ms，远低于重渲染开销。
