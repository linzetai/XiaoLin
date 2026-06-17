## Why

当前 MCP 插件 UI（Explore 面板、Detail Modal、已安装列表、空状态）功能已完整，但视觉质感与 Codex App 的插件目录存在明显差距。Codex 将插件做成了"工作流目录"风格——大图标卡片、品牌色、分类色彩体系、丰富的元数据（author、brandColor、tags）、沉浸式详情页和流畅的微交互动画。XiaoLin 当前采用的是扁平列表行式布局，信息层次单一，缺少品牌识别和动效。需要全面抛光以达到商业级桌面应用的视觉标准。

## What Changes

- **Explore 卡片网格重设计**: 从单列行式列表升级为双列网格卡片布局，每张卡片包含大图标（带品牌色背景）、名称、作者、分类 badge、简介，hover 时微弱上移+阴影加深
- **Registry 数据扩展**: 为每个 MCP Server entry 添加 `brandColor`、`author`、`tags` 字段，提升卡片的视觉丰富度和信息密度
- **McpDetailModal 沉浸式升级**: 顶部增加 Hero 区域（大图标+渐变色背景条+名称+描述），工具列表增加搜索/折叠，新增"编辑配置"入口
- **空状态增强**: 添加浮动呼吸动画、双按钮 CTA（"浏览目录" + "手动添加"）
- **已安装列表品牌色**: PluginRow 左侧增加分类色条/图标背景，从 registry 匹配品牌色
- **CSS 动画补充**: 新增 `pv-float` 浮动 keyframe、Modal 出入场 scale+fade、卡片 stagger 入场
- **响应式卡片布局**: 窄视口自动切换为单列

## Capabilities

### New Capabilities
- `explore-card-grid`: Explore 面板网格卡片布局和品牌色视觉体系
- `detail-modal-hero`: McpDetailModal 沉浸式 Hero 区域和工具列表增强
- `plugin-ui-animation`: 插件页面动画系统（浮动、stagger、modal 过渡）

### Modified Capabilities
- `plugin-panel`: 已安装列表增加品牌色标识、空状态增强

## Impact

- **前端组件**: `McpExplorePanel.tsx`, `McpDetailModal.tsx`, `PluginsView.tsx`（McpEmptyState, PluginRow）
- **数据文件**: `mcp-registry.json`（schema 扩展）
- **样式**: `index.css`（新增 keyframes 和工具类）
- **国际化**: `plugins.json`（zh/en 新增少量 key）
- **不涉及**: 后端 Rust 代码、transport 层、store 层无变更
