## Context

XiaoLin 的 MCP 插件管理 UI 在 Batch A（T12/T12.5/T15）中完成了核心 CRUD 功能，包括 AddServerModal、McpExplorePanel、McpDetailModal。当前 Explore 面板使用单列行式列表，每行包含 36px 图标 + 名称 + 分类 badge + 描述 + 安装按钮。对比 Codex App 的插件目录（双列/三列卡片网格、品牌色系统、丰富元数据、沉浸式详情页），当前 UI 在视觉层次、信息密度和微交互方面有显著提升空间。

当前技术栈：React 19 + Tailwind CSS v4 + Phosphor Icons + Zustand + CSS 变量设计令牌系统。所有组件使用内联 `style={{ }}` 引用 CSS 变量，配合 Tailwind 工具类。动画使用 CSS keyframes 通过 `pv-fade-in`、`pv-stagger` 等工具类触发。

## Goals / Non-Goals

**Goals:**
- 将 Explore 面板从扁平列表升级为视觉丰富的双列网格卡片
- 通过 brandColor、author、tags 扩展 registry 数据，提升卡片信息密度
- 为 McpDetailModal 添加沉浸式 Hero 区域和可搜索工具列表
- 补充动画系统：浮动空状态、Modal 过渡、卡片 stagger
- 为已安装列表增加品牌色识别，视觉对齐 Explore 面板

**Non-Goals:**
- 不新增后端 API 或修改 WebSocket 协议
- 不引入新前端依赖（如 framer-motion）——纯 CSS 动画
- 不修改 transport 层或 plugin-store 数据流
- 不实现服务器评分/评论系统
- 不实现远程 registry 拉取（保持本地 JSON）

## Decisions

### D1: 网格布局方案 — CSS Grid vs Flexbox wrap

**选择**: CSS Grid `grid-cols-2`

**理由**: CSS Grid 提供等宽双列且对齐更可控，Tailwind 的 `grid grid-cols-2` 一行搞定。Flexbox wrap 在卡片高度不一致时对齐差。响应式通过 `@container` 或 media query 在 < 480px 时回退到 `grid-cols-1`。

### D2: 品牌色存储 — Registry JSON vs 运行时计算

**选择**: Registry JSON 静态声明 `brandColor`

**理由**: 每个 MCP server 的品牌色是固定的（如 GitHub #333、Slack #4A154B），适合写死在 registry。运行时计算（如从 icon 名称推断）不够灵活。缺省时 fallback 到已有的 `CATEGORY_COLORS` 系统。

### D3: PluginRow 品牌色匹配 — 实时查 registry vs 缓存 Map

**选择**: 在组件顶层构建 `registryMap: Record<string, McpRegistryEntry>`（`useMemo`），PluginRow 通过 `registryMap[plugin.id]` 查找。

**理由**: registry 是静态 JSON import，创建 Map 的开销可忽略。避免在每个 PluginRow 内部遍历 registry 数组。

### D4: Modal 动画 — CSS keyframes vs CSS transitions

**选择**: CSS keyframes + animation 属性

**理由**: 需要 scale + opacity 组合的入场效果，keyframes 更灵活。当前项目已使用 `@keyframes fade-in` 和 `fade-slide-up`，新增 `modal-enter` keyframe 保持一致性。不需要出场动画（modal 关闭是 unmount，不需要 exit animation，保持简单）。

### D5: DetailModal Hero — 从哪获取 description 和 icon

**选择**: 优先从 registry 匹配（按 `pluginId`），匹配不到则显示 `detail.id` + PuzzlePiece 默认图标。

**理由**: registry 是本地数据，匹配成本极低。对于用户手动添加的非 registry 服务器，退化到默认样式即可。

### D6: 工具列表搜索 — 前端过滤 vs 后端搜索

**选择**: 前端 `useMemo` 过滤

**理由**: 单个 MCP server 的工具数量通常在 5-30 之间，前端过滤完全足够。搜索仅在工具数 > 5 时显示。

## Risks / Trade-offs

- **[卡片高度不一致]** → 通过 `min-h` 约束和固定结构（icon/name/desc 各占固定行）缓解。description 用 `line-clamp-2` 限制。
- **[Registry 数据维护成本]** → brandColor/author/tags 是可选字段，现有 entry 不加也不影响。逐步补充。
- **[Modal 无 exit 动画]** → 入场有动画但关闭是 unmount，可能感觉突兀。暂时接受，后续可用 `AnimatePresence` 类方案改进，但不在本次范围内。
- **[PluginRow 渲染开销增加]** → 新增 registry lookup 和条件渲染图标。由于 registry map 是 useMemo 且列表通常 < 20 项，性能影响可忽略。
