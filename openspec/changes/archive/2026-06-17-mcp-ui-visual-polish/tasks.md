## 1. Registry 数据扩展

- [x] 1.1 为 `mcp-registry.json` 每个 entry 添加 `brandColor` 字段
- [x] 1.2 为每个 entry 添加 `author` 字段
- [x] 1.3 为每个 entry 添加 `tags` 数组
- [x] 1.4 更新 `McpRegistryEntry` TypeScript 接口

## 2. CSS 动画补充

- [x] 2.1 在 `index.css` 中添加 `@keyframes pv-float` 浮动呼吸动画
- [x] 2.2 添加 `.pv-float` 工具类
- [x] 2.3 添加 `@keyframes modal-enter`
- [x] 2.4 添加 `.pv-modal-enter` 工具类

## 3. Explore 卡片网格重设计

- [x] 3.1 将卡片容器改为 grid 布局 — 使用 `auto-fill, minmax(240px, 1fr)`
- [x] 3.2 重构单张卡片为竖式布局
- [x] 3.3 实现卡片 hover 效果
- [x] 3.4 实现卡片 stagger 入场动画
- [x] 3.5 添加响应式断点：viewport < 480px 时回退到 `grid-cols-1`
- [x] 3.6 美化搜索栏：focus 时 ring 效果

## 4. McpDetailModal 沉浸式升级

- [x] 4.1 构建 `registryMap` (useMemo)
- [x] 4.2 添加 Hero 顶部区域
- [x] 4.3 添加 3px 渐变色条
- [x] 4.4 工具列表增加折叠/展开切换
- [x] 4.5 工具列表在 toolCount > 5 时显示搜索 input
- [x] 4.6 添加"编辑配置"按钮 → 打开 AddServerModal(prefill) — PluginsView 传入 onEditConfig，通过 mcpDetail 获取配置后 prefill
- [x] 4.7 应用 `pv-modal-enter` 动画到 modal 容器

## 5. 空状态与已安装列表增强

- [x] 5.1 McpEmptyState 图标容器添加 `pv-float` 动画
- [x] 5.2 McpEmptyState 增加第二个 CTA 按钮："手动添加"
- [x] 5.3 构建 `registryMap` 供 PluginRow 使用
- [x] 5.4 PluginRow 替换为 registry-matched 图标 + brandColor 背景
- [x] 5.5 对于非 registry 服务器，PluginRow 显示 PuzzlePiece 默认图标

## 6. 国际化与验证

- [x] 6.1 更新 `plugins.json`（zh/en）添加新增 i18n key
- [x] 6.2 运行 `npx tsc --noEmit` 确认零类型错误
- [x] 6.3 Tauri MCP E2E 验证：DOM snapshot 确认 Explore 网格渲染（搜索栏、category pills、15 张卡片带 icon/tags/安装按钮）
