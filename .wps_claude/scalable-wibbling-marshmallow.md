# XiaoLin 前端优化方案

## Context

XiaoLin 是 Tauri v2 桌面应用，前端 React 19 + TypeScript + Vite 8 + Tailwind CSS v4。经过对全部 ~50 个组件源码的深入分析，发现性能、UI 和组件架构三方面的问题，分 8 个阶段共 22 项优化。

---

## Phase 1: 构建优化（P0）

### 1.1 Vite 手动分包

**文件**: `crates/xiaolin-app/vite.config.ts`

**变更**: 添加 `manualChunks`，将 907KB 单 chunk 拆分：
- `vendor-react` (~45KB): react, react-dom
- `vendor-markdown` (~180KB): react-markdown, remark-gfm, rehype-highlight
- `vendor-virtuoso` (~50KB): react-virtuoso
- `vendor-lucide` + `vendor-zustand`

**收益**: 主 chunk 从 907KB 降至 150-200KB，首屏解析时间减少 40-50%

### 1.2 组件懒加载

**文件**: `src/components/layout/AppLayout.tsx`, `src/components/layout/TitleBar.tsx`

**变更**: `React.lazy()` + `Suspense` 懒加载 OnboardingWizard、AgentDetail、SettingsPanel、NotificationCenter

**收益**: 首屏减少 ~85KB

### 1.3 Settings/AgentDetail Tabs 懒加载

**文件**:
- `src/components/settings/SettingsPanel.tsx` — 9 个 Tab 懒加载
- `src/components/agent-detail/AgentDetail.tsx` — ChatsTab、CronTab、AgentConfigForm 懒加载

**收益**: 设置面板减少 ~30KB，AgentDetail 按需加载

---

## Phase 2: Store 重构（P0）

### 2.1 拆分 Store + immer 中间件

**核心问题**: `appendStreamDelta` 每 16ms 创建新 `agentChats` 引用链，触发所有订阅组件重渲染

**新增依赖**: `pnpm add immer`

**文件**:
- `src/lib/stores/chat-store.ts` (新建，用 immer 中间件)
- `src/lib/stores/index.ts` (重构为主导出)

```typescript
// immer: 直接修改 draft，无需手动深拷贝
appendStreamDelta: (agentId, chatId, delta) => {
  set((state) => {
    const chat = state.agentChats[agentId]?.chatList.find(c => c.id === chatId);
    const lastItem = chat?.stream.slice(-1)[0];
    if (lastItem?.data.role === "assistant") {
      lastItem.data.content += delta;
    }
  });
},
```

**收益**: streaming 期间重渲染减少 ~80%

### 2.2 细粒度 Selector

**问题组件**: `AgentList.tsx:35` 订阅整个 `agentChats`；`MessageStream.tsx:41` 同样

**变更**:
```typescript
// AgentList — 只订阅当前 agent 的 unread/lastMsg
const ac = useChatStore(useCallback(s => s.agentChats[agentId], [agentId]));
```

### 2.3 合并 Store 导出路径

**文件**: `src/lib/agent-store.ts`, `src/lib/store.ts`

**变更**: 删除 `agent-store.ts` 重复导出，统一从 `stores/` 导入，消除循环依赖

---

## Phase 3: CSS 性能（P1）

### 3.1 移除全局 `*` Transition

**文件**: `src/index.css`

**问题**: `* { transition: ... 0.2s }` 导致 streaming 时每条消息触发 transition

**变更**:
1. 删除全局 `*` transition
2. 仅 `button, a, input, select, textarea` 保留 transition
3. 添加 `.streaming-active .markdown-body { transition: none !important; }`

**收益**: streaming 期间 CPU 降低 ~30%

### 3.2 统一 Tailwind 样式（持续）

**变更**: `@theme` 注册 CSS 变量，逐步替换 `style={{ background: "var(--bg-primary)" }}` 为 Tailwind class

---

## Phase 4: 流式渲染优化（P1）

### 4.1 Markdown 渲染节流

**文件**: `src/components/message-stream/MarkdownContent.tsx`

**问题**: streaming 时每 16ms 执行完整 rehype-highlight 语法高亮

**变更**: 添加 `streaming` prop，streaming 期间跳过 rehype-highlight
```typescript
const rehypePlugins = showHighlight ? [rehypeHighlight] : [];
```

**收益**: streaming 期间渲染耗时减少 60-80%

### 4.2 detachedStreams 内存管理

**文件**: `src/components/message-stream/messageStreamRegistry.ts`

**变更**: 模块级 Map 改为 Zustand store，React 组件可正确订阅和清理

### 4.3 Avatar URL 全局缓存

**文件**: `src/lib/use-avatar-url.ts`

**变更**: `Map<string, { url, refCount }>` 引用计数缓存，同一 avatar 只读一次文件系统

---

## Phase 5: 组件渲染优化（P1）

### 5.1 AgentList 组件拆分

**文件**: `src/components/agent-list/AgentList.tsx` (529行)

**问题**:
- 整个侧边栏是一个组件，新建 Agent 弹窗、欢迎面板、搜索逻辑全混在一起
- 每次任何 state 变化（query、creating、showNewForm 等）都重渲染整个列表
- `agentChats` 整体订阅，切换任何 agent 的 chat 都触发重渲染

**变更**:
1. 抽取 `AgentListItem` 组件并用 `memo` 包裹，避免其他 agent 变化导致本项重渲染
2. 抽取 `NewAgentModal` 为独立组件，内部管理 9 个表单状态
3. 抽取 `WelcomePanel` 为独立组件
4. 每个 `AgentListItem` 只订阅自己 agent 的 chat 数据

### 5.2 MessageRendererRow 优化

**文件**: `src/components/message-stream/MessageRenderer.tsx`

**问题**:
- `MessageRendererRow` 非 memo，每个可见行在 streaming 时都重渲染
- `UserBubble`、`AiMessage`、`SystemMsg` 非 memo
- 内联箭头函数 `() => onToggleSelect?.(fullIdx)` 每次渲染创建新引用

**变更**:
1. `MessageRendererRow` 用 `memo` 包裹
2. `UserBubble`、`AiMessage` 用 `memo` 包裹
3. 稳定化 callback 引用

### 5.3 ToolCallCard / SubAgentCard 优化

**文件**: `ToolCallCard.tsx`, `SubAgentCard.tsx`

**问题**:
- `TOOL_META` 中的 icon 每次访问都返回新 ReactNode（`<FileText {...ICON_PROPS} />`）
- `ElapsedTimer` 每 200ms 触发重渲染，向上传播导致 ToolCallCard 重渲染
- `extractKeyInfo` 每次 render 执行 JSON.parse

**变更**:
1. `TOOL_META` 的 icon 预创建为常量 ReactElement（已通过 `as const` 部分解决，但对象内部仍重新创建）
2. `ElapsedTimer` 用 `memo` 包裹，仅更新计时部分
3. `extractKeyInfo` 结果用 `useMemo` 缓存

### 5.4 ChatTabsBar 优化

**文件**: `src/components/message-stream/ChatTabsBar.tsx`

**问题**:
- 内联 `onDragStart/onDragEnd/onDragOver/onDrop` 每次渲染创建新函数
- `hoveredTab` state 变化导致所有 tab 重渲染
- `openChats.filter()` 每次 render 执行

**变更**:
1. 抽取 `ChatTab` 为 `memo` 组件
2. `openChats` 用 `useMemo` 缓存
3. 稳定化 drag handler 引用

---

## Phase 6: UI 交互优化（P2）

### 6.1 AgentDetail 面板动画与布局

**文件**: `src/components/agent-detail/AgentDetail.tsx`

**问题**:
- 面板关闭时 `width: 0 + opacity: 0` 但内容仍渲染，DOM 节点仍在
- Tab 切换无过渡效果
- `agentColor` prop 传入但未使用（`_agentColor`）

**变更**:
1. 面板关闭时用 `display: none` 或条件渲染，避免隐藏内容占用 DOM
2. Tab 内容切换添加淡入动画
3. 清理未使用的 `agentColor` prop

### 6.2 搜索面板增强

**文件**: `src/components/message-stream/MessageStream.tsx`

**问题**:
- 搜索结果只高亮背景色，无法定位到具体匹配文字
- 无搜索结果计数进度条
- Cmd+F 全局快捷键与浏览器默认冲突（已 preventDefault，但用户可能期望浏览器原生搜索）

**变更**:
1. 搜索结果中高亮匹配文字（黄色标记）
2. 匹配时自动滚动到对应位置（已有 Virtuoso，需接 `scrollToIndex`）

### 6.3 空状态与加载态完善

**问题**:
- AgentList 在 agents 为空且有 query 时无"无结果"提示
- Settings Panel 各 Tab 无 loading 态
- OnboardingWizard 的 `goTo` 用 `setTimeout` 实现动画过渡，250ms 硬编码

**变更**:
1. AgentList 搜索无结果时显示空状态
2. Settings Tab 异步数据加载时显示 skeleton
3. OnboardingWizard 动画用 CSS `onTransitionEnd` 替代 setTimeout

### 6.4 ContextRing 浮层定位

**文件**: `src/components/message-stream/StreamFooter.tsx`

**问题**: `ContextRing` tooltip 用 `bottom: "100%"` 定位，在底部工具栏附近可能溢出屏幕

**变更**: 用 `position: "top"` 并自动检测视口边界翻转

---

## Phase 7: 健壮性优化（P2）

### 7.1 Error Boundary

**新文件**: `src/components/ErrorBoundary.tsx`

**添加位置**:
- `App.tsx` — 包裹整个应用
- `MessageStream.tsx` — 包裹 Virtuoso 消息列表
- `MarkdownContent.tsx` — 包裹 Markdown 渲染（最易出错点）

### 7.2 Transport 错误处理增强

**文件**: `src/lib/transport.ts`

**变更**: 统一 Tauri-only 功能降级策略 + IPC 超时处理

---

## Phase 8: 代码质量（P3）

### 8.1 中文字符串提取

**新文件**: `src/lib/i18n.ts` — 轻量 i18n，渐进式替换

### 8.2 清理死代码

- `src/lib/stores/config-store.ts` — 空 store，删除
- `formatTokens` 函数在 `MessageRenderer.tsx` 和 `StreamFooter.tsx` 中重复定义，提取到 `lib/format.ts`
- `handleQuickCreateDefault` 和 `handleNewAgent` 中 `syncAgents` 调用模式重复，抽取 `createAndSyncAgent` 工具函数

---

## 实施计划

| 周次 | 优化项 | 预期效果 |
|------|--------|---------|
| 第1周 | 1.1→1.2→1.3→3.1 | 构建体积 -78%，CSS 性能提升 |
| 第2周 | 2.1→2.2→2.3 | streaming 重渲染 -80% |
| 第3周 | 4.1→4.2→4.3→5.1 | streaming 渲染耗时 -70%，列表性能 |
| 第4周 | 5.2→5.3→5.4 | 组件级渲染优化 |
| 第5周 | 6.1→6.2→6.3→6.4 | UI 交互增强 |
| 第6周 | 7.1→7.2→8.1→8.2 | 健壮性+代码质量 |

---

## 关键指标

| 指标 | 当前 | 目标 |
|------|------|------|
| 主 JS chunk | 907KB | < 200KB |
| 首屏 JS 总量 | ~930KB | < 400KB |
| Streaming 每帧渲染 | ~15ms | < 5ms |
| Streaming 期间重渲染组件数 | 全部订阅组件 | 仅当前 chat 组件 |
| AgentList 切换 agent 重渲染 | 整个列表 | 单个 ListItem |
| 组件崩溃影响 | 全应用白屏 | 局部错误提示 |

---

## Verification

1. **构建**: `pnpm build` 后检查 `dist/assets/` chunk 大小分布
2. **性能**: Chrome DevTools Performance 测量 FCP/TTI
3. **Streaming**: React DevTools Profiler 对比重渲染次数
4. **功能**: 手动测试流式输出、切换 agent/chat、新建 chat、设置面板各 Tab
5. **UI**: 手动验证搜索高亮、空状态提示、面板动画

---
---

## Review 反馈与补充建议

> 以下为方案 Review 及补充的 UI/UX 优化建议。

### Review 总评: 7.5/10

技术方向正确，优先级划分合理。主要问题集中在数据预估乐观、部分优化的 trade-off 分析缺失。

### 需修正/补充的点

#### R1. Phase 1 构建数据过于乐观

- `manualChunks` 只是把 907KB 拆散到多个文件，**总 JS 量不变**。真正有意义的指标是 gzip 后体积和 critical path 的加载量。建议用 `npx vite-bundle-analyzer` 对比 gzip 前后。
- OnboardingWizard 的 85KB 只在首次引导时加载。如果用户已 onboarded，lazy loading 的实际首屏收益为 0。应标注为"新用户首屏"收益。

#### R2. Phase 2 immer Proxy 开销需 Benchmark

- `appendStreamDelta` 高频场景（16ms/次），每次 `set()` 会创建 Proxy 遍历整个 state tree。
- 替代方案：直接 `useRef` 持有 streaming buffer + `useSyncExternalStore` 手动 notify，完全绕过 Zustand 的 immutable 约束。
- **建议**：先跑 benchmark 对比 immer vs 手写 mutation，确认 Proxy 开销是否可接受。

#### R3. Phase 4.1 rehype-highlight 闪烁问题

- 完全跳过高亮会导致 streaming → 结束时突然整段代码变色，视觉跳跃明显。
- **改进方案**：只对已闭合的代码块（检测到 ``` 配对）执行高亮；正在输入的代码块保持纯文本。或用 `requestIdleCallback` 分批高亮。

#### R4. Phase 5.3 TOOL_META icon 非真实瓶颈

- 代码验证：`TOOL_META` 是模块级 `const`，其中每个 `<FileText {...ICON_PROPS} />` 在模块加载时只创建一次 ReactElement 对象。后续渲染中取出的是同一引用，不会导致额外 reconciliation。
- **建议**：re-profile 确认 `ToolCallCard` 的重渲染根因。更可能是 `ElapsedTimer` 的 200ms interval 和 `extractKeyInfo` 的 JSON.parse。

#### R5. Phase 7.1 Error Boundary 位置

- `MarkdownContent` 已是 `memo` 组件，内部 crash 概率较低。
- 真正容易出错的是 `ToolCallCard`（`JSON.parse(args)` 可能 throw）和 `SubAgentCard`。
- **建议**：在 **Virtuoso Item level**（MessageRendererRow）加 boundary，一次覆盖所有消息类型。

#### R6. 缺少回归验证机制

- 每个 Phase 完成后应有 perf baseline 对比（建议用 `playwright` + `performance.mark` 写自动化脚本）。
- Phase 2 Store 重构是 breaking change，需考虑 localStorage 中 persisted state 的 migration。

---

## Phase 9: UI/UX 深度优化（补充）

### 9.1 流式输出视觉反馈增强

**问题**: streaming 时只有文字追加，用户无法区分"正在输出"和"输出完成"。

**变更**:
1. 末尾呼吸态光标已有（`cursor-blink`），但 streaming 结束后应平滑渐隐（`fade-out 0.3s`）而非突然消失
2. ToolCallCard running 状态添加 indeterminate progress bar 替代纯 spinner
3. 消息气泡底部添加微妙 shimmer 效果，仅 streaming 期间显示

### 9.2 消息列表滚动体验

**问题**: streaming 时 auto-scroll 与用户手动上滑冲突。

**变更**:
1. 检测用户距底部 < 100px 时才 auto-scroll
2. 用户上滑后显示「回到底部」FAB 按钮（带未读消息计数 badge）
3. FAB 点击后平滑滚动到底部并渐隐

### 9.3 连续工具调用分组折叠

**问题**: 10+ 工具调用产生长列表，推远实际回复。

**变更**:
1. 新建 `ToolCallGroup.tsx`，连续 3+ 个 tool call 自动分组
2. 折叠态显示摘要：`✓ 5 个工具调用完成 (2.3s) — read_file ×3 · shell_exec ×2`
3. Streaming 期间用竖向时间轴模式（每行 ~28px vs 当前 ~44px）
4. 智能折叠策略：最近 2 个展开，之前的折叠；error 始终展开
5. 折叠阈值可配置（Settings → 通用 → "工具调用折叠阈值"，默认 3）

### 9.4 Agent 切换上下文保留

**问题**: 切换 agent 时输入框 draft 丢失。

**变更**:
1. 每个 agent 独立维护 `draftText` 和 `draftMentions` 在 store 中
2. 切换 agent 时自动保存/恢复 draft
3. 附件列表同理

### 9.5 键盘导航

**变更**:
1. AgentList: `↑↓` 选择 agent，`Enter` 确认，`/` 聚焦搜索
2. ChatTabsBar: `Ctrl+Tab` / `Ctrl+Shift+Tab` 切换 tab
3. 消息列表: `Escape` 取消选择模式
4. 所有交互元素添加 `aria-label` 和正确 `role`

### 9.6 空状态设计

**变更**:
1. StreamEmptyState 显示当前 agent 的能力标签（tools/skills 概览）
2. 提供 3-5 个快捷 prompt 模板按钮（根据 agent system prompt 自动生成）
3. AgentList 搜索无结果时显示"未找到"空状态

### 9.7 通知反馈一致性

**变更**:
1. 统一 NotificationCenter 为应用内消息中心
2. 重要错误（网关断连）用 persistent banner
3. 次要通知（cron 完成）用 inline notification，3s 自动消失

### 9.8 Settings 表单体验

**变更**:
1. 未保存更改时 tab 标题旁显示小圆点
2. 离开未保存 tab 时 confirm 提示
3. 表单验证即时反馈

---

## Phase 10: 输入与 Mention 体验

### 10.1 输入框自适应高度

**文件**: `src/components/message-stream/MentionInput.tsx`, `src/index.css`

**问题**: 初始 `rows=2`/`min-height: 72px` 对短消息太高，MAX_HEIGHT=160px 对长文本不够。

**变更**:
1. 最小高度降为 44px（单行），最大升至 240px（~9 行）
2. 用 `ResizeObserver` 替代 `onInput → autoGrow`，消除 1-frame 闪烁
3. 始终 `overflow-y: auto` + `scrollbar-gutter: stable`，避免高度切换跳变

### 10.2 发送按钮状态增强

**文件**: `src/components/message-stream/StreamFooter.tsx`

**变更**:
1. 空输入时 ArrowUp 图标降低 opacity 0.3 + 禁用
2. 有内容时变为 `--tint` 色 + hover scale(1.02)
3. 发送到 streaming 开始前按钮变为 loading spinner

### 10.3 历史消息编辑

**变更**:
1. `↑` 键空输入框时调出最近用户消息
2. 编辑模式显示 "编辑中" badge + ESC 取消

### 10.4 Mention Popup 增强

**文件**: `src/components/message-stream/MentionInput.tsx`

**变更**:
1. 空匹配时显示 `未找到 "xxx"` 提示而非直接消失
2. 选中项 `scrollIntoView({ block: 'nearest' })`
3. 使用 `@floating-ui/react` 替代手动 `window.innerHeight` 定位
4. 添加 `/` 触发 slash commands、`#` 触发会话引用

### 10.5 Fuzzy 匹配改进

**变更**:
1. 替换 `includes()` 为 fzf-style fuzzy scoring
2. 匹配字符高亮（split + mark）
3. 排序：精确前缀 > 路径末段 > 模糊

### 10.6 Mention Chip 升级

**文件**: `src/index.css` (.mention-chip)

**变更**:
1. Chip 内添加 type icon 前缀
2. Hover 时可点击跳转到文件/skill 详情
3. 长路径智能截断：`/very/long/path/file.rs` → `…/file.rs`

---

## Phase 11: 字体与排版

### 11.1 字体栈调整

**文件**: `src/index.css` (`:root`)

**变更**:
```css
--font: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Inter",
  "Segoe UI Variable", system-ui, sans-serif;
--font-display: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Inter",
  "Segoe UI Variable", system-ui, sans-serif;
--font-mono: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code",
  Menlo, Monaco, Consolas, monospace;
```

- 正文用 `--font`（Text weight），标题用 `--font-display`
- 增加 Inter 作为 Linux 高质量 fallback
- 等宽字体加入 JetBrains Mono

### 11.2 排版微调

**变更**:
1. `letter-spacing`: 正文从 `-0.016em` 调至 `-0.011em`；小字号（11-12px）用 `0`
2. Markdown 区 `line-height`: 从 1.75 调至 1.8（中文阅读性更佳）
3. 数字密集区域添加 `font-variant-numeric: tabular-nums`
4. 代码块行高从 1.65 调至 1.55

### 11.3 用户可配置字体大小

**文件**: Settings → GeneralTab

**变更**:
- 小 (13px/14px)、标准 (14px/15px, 默认)、大 (15px/16px)、特大 (16px/17px)
- 修改 `html { font-size }` 全局缩放

---

## Phase 12: 动画系统

### 12.1 动画 Token 系统

**文件**: `src/index.css`

**新增 CSS 变量**:
```css
:root {
  --duration-instant: 100ms;
  --duration-fast: 150ms;
  --duration-normal: 200ms;
  --duration-slow: 350ms;
  --ease-out: cubic-bezier(0.16, 1, 0.3, 1);
  --ease-in: cubic-bezier(0.55, 0, 1, 0.45);
  --ease-in-out: cubic-bezier(0.45, 0, 0.55, 1);
  --ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1);
  --move-sm: 4px;
  --move-md: 8px;
  --move-lg: 16px;
}
```

所有现有动画统一引用 token，而非各处硬编码不一致的值。

### 12.2 消息入场动画改进

**变更**:
1. 只对**新增**消息播放入场动画，历史加载不播放
2. Streaming 生成的消息用 `fade-in 100ms` 而非 `slide`
3. 消息间隔 < 200ms 时跳过动画
4. 动画元素添加 `will-change: transform, opacity`

### 12.3 状态切换动画

**变更**:
1. AgentDetail Tab 切换：crossfade（opacity 交叉）
2. AgentDetail open/close：改用 `clip-path` 动画（GPU 加速）
3. AgentList 选中态：手动添加 `background-color` transition（全局 `*` 移除后需显式声明）

### 12.4 Streaming 动画去抖

**变更**:
1. 连续 ToolCallCard：首个 `slide-up`，后续 `fade-in`
2. Typing indicator 添加 min-display-time 300ms 避免闪烁

---

## Phase 13: 配色系统

### 13.1 默认主题引入品牌色 Tint

**问题**: 当前默认 `--tint: #1d1d1f`（light）/ `#e5e5ea`（dark）与 fill-primary 几乎相同，无法区分强调元素。

**变更**:
```css
:root, [data-theme="light"] {
  --tint: #2563EB;
  --tint-bg: rgba(37, 99, 235, 0.06);
  --tint-subtle: rgba(37, 99, 235, 0.03);
}
[data-theme="dark"] {
  --tint: #60A5FA;
  --tint-bg: rgba(96, 165, 250, 0.1);
  --tint-subtle: rgba(96, 165, 250, 0.05);
}
```

保留原灰色方案为 `data-accent="monochrome"` 选项。

### 13.2 暗色对比度修复

**问题**: `--fill-tertiary: #636366` 在 `#000000` 上对比度仅 4.2:1，低于 WCAG AA 4.5:1。

**变更**:
```css
[data-theme="dark"] {
  --fill-tertiary: #8E8E93;    /* 5.5:1 对比度 */
  --fill-quaternary: #636366;
  --bg-selected: #2c2c2e;     /* 与 bg-secondary #1c1c1e 区分 */
  --separator: rgba(84, 84, 88, 0.48);
}
```

### 13.3 User Bubble 配色跟随 Tint

**变更**:
```css
--bubble-user: var(--tint);
--bubble-user-text: #ffffff;
```

视觉上与品牌色统一，不再是纯黑/纯灰。

### 13.4 新增 Accent Theme

**变更**: 添加 `sage` (温和绿) 和 `rose` (柔和粉) 两套 accent。

### 13.5 主题切换 Transition 统一

**问题**: html 0.35s vs `*` 0.2s 导致割裂。

**变更**:
1. 所有主题相关 transition 统一为 `var(--duration-slow)` = 0.35s
2. Streaming 期间 `.markdown-body` 禁用 transition（配合 Phase 3.1）

---

## 综合实施计划（含补充）

| 周次 | 优化项 | 预期效果 |
|------|--------|---------|
| 第0周 | **FE-000 回归基建** | Perf baseline 脚本 + Core C1-C10 自动化 + Screenshot 基线 + CI gate |
| 第1周 | 1.1→1.2→1.3→3.1 + **13.1→13.2** | 构建优化 + 配色基础修复 |
| 第2周 | 2.1→2.2→2.3 + **12.1** | Store 重构 + 动画 token 系统 |
| 第3周 | 4.1→4.2→4.3→5.1 + **9.3** | Streaming 优化 + 工具调用分组 |
| 第4周 | 5.2→5.3→5.4 + **12.2→12.4** | 组件渲染 + 动画去抖 |
| 第5周 | 6.1→6.2→6.3→6.4 + **9.1→9.2→9.4** | UI 交互 + 流式反馈 + 滚动 |
| 第6周 | 7.1→7.2→8.1→8.2 + **9.5→9.6** | 健壮性 + 键盘导航 + 空状态 |
| 第7周 | **10.1→10.2→10.4→10.5** | 输入框 + Mention 深度优化 |
| 第8周 | **11.1→11.2→13.3→13.4** | 字体排版 + 配色扩展 |

---

## 关键指标（更新）

| 指标 | 当前 | 目标 |
|------|------|------|
| 主 JS chunk | 907KB | < 200KB (gzip < 60KB) |
| 首屏 JS 总量 | ~930KB | < 400KB (gzip < 120KB) |
| Streaming 每帧渲染 | ~15ms | < 5ms |
| Streaming 期间重渲染组件数 | 全部订阅组件 | 仅当前 chat 组件 |
| AgentList 切换 agent 重渲染 | 整个列表 | 单个 ListItem |
| 组件崩溃影响 | 全应用白屏 | 局部错误提示 |
| 连续 10 tool calls 占用高度 | ~440px | < 200px (折叠态 ~60px) |
| 暗色模式正文对比度 (fill-tertiary) | 4.2:1 | ≥ 5.5:1 (WCAG AA+) |
| 输入框首帧响应 | 有 1-frame 跳变 | 0-frame (ResizeObserver) |
| Mention 匹配方式 | includes() | fuzzy scoring + 字符高亮 |

---

## 回归验证方案

### 性能 Baseline 脚本（自动化）

**工具**：Playwright + Chrome DevTools Protocol + React Profiler API

**采集指标**：
1. 构建体积（各 chunk gzip 大小）
2. 首屏 FCP/LCP/TTI（Performance API）
3. Streaming 帧时间（连续 100 帧的 renderTime 中位数/P95）
4. Streaming 重渲染计数（React Profiler onRender callback）
5. 内存占用（performance.memory）

**标准操作序列**：
```
[首屏] 冷启动 → AgentList 渲染完成 → 记录 FCP/LCP
[切换] 依次切换 3 个 agent → 记录 re-render count
[发消息] "Hello" → streaming 完成 → 记录帧时间
[长消息] 触发 10+ tool call → 记录渲染性能
[设置] Settings → 切换每个 Tab → 记录加载时间
[搜索] 搜索关键词 → 记录匹配高亮响应时间
```

**执行时机**：每个 Phase 开始前 + 完成后各跑一次，存入 `perf/build-{phase}-{before|after}.json`

### 功能回归 Checklist

#### Core（每个 Phase 必跑）

| # | 场景 | 验证点 | 方式 |
|---|------|--------|------|
| C1 | 冷启动 | 无白屏/crash，正常渲染 | Playwright |
| C2 | 新建 Agent | 表单 → 创建 → 列表出现 | Playwright |
| C3 | 发消息 streaming | 输入 → 流式输出 → 正确结束 | Playwright |
| C4 | Tool call | 触发 read_file/shell → ToolCallCard 正确 | Playwright |
| C5 | 切换 Agent | 侧边栏切换 → 消息列表正确 | Playwright |
| C6 | 切换 Chat | tab 切换 → 消息对应 | Playwright |
| C7 | 新建 Chat | Cmd+K → 空状态 | Playwright |
| C8 | 设置面板 | 各 Tab 可切换 → 保存生效 | 手动 |
| C9 | 主题切换 | light ↔ dark 正常 | Playwright |
| C10 | 窗口操作 | 最小化/恢复/关闭/托盘 | 手动 |

#### 针对性验证

- **FE-001**: 懒加载组件有 loading 态；slow 3G 下 Settings Tab 体验
- **FE-002**: hover transition 保留；主题切换无割裂；AgentList 选中有过渡
- **FE-003**: 清空 localStorage 正常启动；旧数据 migrate；双 agent 并行 streaming
- **FE-004**: 代码块 streaming 无闪烁；结束后高亮完整；avatar 不重复加载
- **FE-005**: NewAgentModal 功能完整；搜索正常；拖拽排序正常
- **FE-006**: 单 tool call 不分组；3+ 自动折叠；error 始终可见；SubAgent 不参与
- **FE-008**: 所有 accent light+dark 无异常；focus ring 用 tint 色

### 视觉回归 Screenshot Diff

**工具**：`@playwright/test` + `toHaveScreenshot()`

**截图矩阵**：

| 页面状态 | Light | Dark | Ocean | Sunset | Midnight |
|----------|:-----:|:----:|:-----:|:------:|:--------:|
| AgentList + 空聊天 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 消息列表(含 tool call) | ✓ | ✓ | ✓ | ✓ | ✓ |
| Settings General Tab | ✓ | ✓ | - | - | - |
| MentionPopup 打开态 | ✓ | ✓ | - | - | - |
| Streaming 进行中 | ✓ | ✓ | - | - | - |

**阈值**：pixel diff < 0.5%，超过则人工确认。

### 性能门禁（CI）

```yaml
# .github/workflows/perf-gate.yml
- Hard gate (阻止 merge):
    - 任何 chunk gzip > 80KB
    - FCP > 1.5s
    - Streaming P95 帧时间 > 12ms
- Soft gate (warn 不阻止):
    - 总 JS gzip > 400KB
    - Streaming P50 帧时间 > 5ms
```

### PR Before/After 模板

每个 FE-xxx 的 PR description 必须包含：

```markdown
## Performance Comparison
| Metric | Before | After | Change |
|--------|--------|-------|--------|
| Main chunk (gzip) | xxxKB | xxxKB | -xx% |
| Streaming P95 frame | xxms | xxms | -xx% |
| Re-renders during stream | xx | xx | -xx% |

## Screenshots
[Before] [After]

## Regression
- [ ] Core C1-C10 passed
- [ ] Phase-specific checks passed
- [ ] Screenshot diff < 0.5%
```

### Rollback 策略

1. `git revert` 该 Phase 所有 commits
2. tasks.json 标记 `"status": "blocked"`，添加 `blocked_reason`
3. 新 branch 修复后重新提交

**FE-003 (Store) 特殊处理**（最高风险）：
- Feature branch 完整实现 + 全量 Core checklist
- 合入 main 后用 feature flag 保留旧 store 代码 1 周
- 确认无问题后删除旧代码
