## 1. Registry 数据扩展

- [ ] 1.1 为 `mcp-registry.json` 每个 entry 添加 `brandColor` 字段（十六进制色值）
- [ ] 1.2 为每个 entry 添加 `author` 字段（如 "Anthropic"、"Community"）
- [ ] 1.3 为每个 entry 添加 `tags` 数组（如 `["official"]`、`["popular"]`）
- [ ] 1.4 更新 `McpRegistryEntry` TypeScript 接口，添加 `brandColor?`、`author?`、`tags?` 字段

## 2. CSS 动画补充

- [ ] 2.1 在 `index.css` 中添加 `@keyframes pv-float` 浮动呼吸动画（上下 6px、3s 循环）
- [ ] 2.2 添加 `.pv-float` 工具类引用该 keyframe
- [ ] 2.3 添加 `@keyframes modal-enter` (scale 0.96→1 + opacity 0→1, 200ms ease-out)
- [ ] 2.4 添加 `.pv-modal-enter` 工具类

## 3. Explore 卡片网格重设计

- [ ] 3.1 将卡片容器从 `flex flex-col gap-2` 改为 `grid grid-cols-2 gap-3`
- [ ] 3.2 重构单张卡片为竖式布局：顶部 icon 区域(40x40 + brandColor bg) → 名称+作者 → 分类 badge → 描述(line-clamp-2) → tags 行 → 安装按钮
- [ ] 3.3 实现卡片 hover 效果：`-translate-y-0.5` + `shadow-md` + 200ms transition
- [ ] 3.4 实现卡片 stagger 入场动画（复用 `pv-stagger` + `--stagger-i` 变量）
- [ ] 3.5 添加响应式断点：viewport < 480px 时回退到 `grid-cols-1`
- [ ] 3.6 美化搜索栏：圆角加大、focus 时 ring 效果

## 4. McpDetailModal 沉浸式升级

- [ ] 4.1 构建 `registryMap` (useMemo) 从 registry 按 id 查找 entry 元数据
- [ ] 4.2 添加 Hero 顶部区域：大图标(48px) + brandColor 背景 + 服务器名(18px) + 描述 + 状态 badge
- [ ] 4.3 添加 3px 渐变色条：category color 40% → transparent
- [ ] 4.4 工具列表增加折叠/展开切换（默认展开）
- [ ] 4.5 工具列表在 toolCount > 5 时显示搜索 input
- [ ] 4.6 添加"编辑配置"按钮 → 打开 AddServerModal(prefill 当前配置)
- [ ] 4.7 应用 `pv-modal-enter` 动画到 modal 容器

## 5. 空状态与已安装列表增强

- [ ] 5.1 McpEmptyState 图标容器添加 `pv-float` 动画
- [ ] 5.2 McpEmptyState 增加第二个 CTA 按钮："手动添加"(ghost 样式，打开 AddServerModal)
- [ ] 5.3 构建 `registryMap` 供 PluginRow 使用（与 DetailModal 共享同一逻辑）
- [ ] 5.4 PluginRow 替换 StatusDot 为 registry-matched 图标 + brandColor 背景
- [ ] 5.5 对于非 registry 服务器，PluginRow 显示 PuzzlePiece 默认图标 + tint 色

## 6. 国际化与验证

- [ ] 6.1 更新 `plugins.json`（zh/en）添加新增 i18n key（author 标签、tags、编辑配置等）
- [ ] 6.2 运行 `npx tsc --noEmit` 确认零类型错误
- [ ] 6.3 Tauri MCP E2E 验证：截图对比 Explore 网格、Detail Hero、空状态动画
