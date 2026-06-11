## Why

当前项目使用 Lucide React 图标库，但在 70+ 个组件文件中，图标的 size（9~32px）和 strokeWidth（1.0~2.5）值高度分散，缺乏统一的视觉语言。虽然已有 `ui-tokens.ts` 定义了 3 个尺寸等级，但大量组件仍使用硬编码值。Lucide 缺乏 weight 语义（只能靠数字 strokeWidth 模拟），无法表达 active/inactive、primary/secondary 等 UI 状态层级。

迁移到 Phosphor Icons 可获得：6 种原生 weight 变体（thin/light/regular/bold/fill/duotone）、IconContext 全局配置、更强的跨平台生态（React Native/Flutter/Rust），以及通过 weight 语义取代 strokeWidth 微调带来的设计系统一致性。

## What Changes

- **BREAKING**: 移除 `lucide-react` 依赖，替换为 `@phosphor-icons/react`
- 创建 Phosphor 图标 Token 系统（size scale + weight 语义映射）
- 在 App 根组件添加 `IconContext.Provider` 全局配置
- 迁移所有 70+ 文件中的图标 import 和 props
- 更新自定义 SVG 图标（ClawIcon 等）使其与 Phosphor 风格协调
- 更新 `ui-tokens.ts` 为 Phosphor 适配的 Token 定义

## Capabilities

### New Capabilities
- `icon-design-tokens`: 定义统一的图标 Token 系统（size scale、weight 语义映射、color 语义），提供 IconContext 全局配置和 helper utilities
- `icon-migration`: Lucide → Phosphor 的完整迁移方案，包括名称映射表、自动化脚本、自定义 SVG 适配

### Modified Capabilities

## Impact

- **前端代码**: 70+ 组件文件需要修改 import 和 icon props
- **依赖**: 移除 `lucide-react`，新增 `@phosphor-icons/react`
- **Bundle size**: 单图标略增（~200B → ~300B），但 tree-shaking 后总体影响可控
- **ui-tokens.ts**: 需要重写以适配 Phosphor 的 weight 系统
- **开发体验**: 从 `<Icon size={14} strokeWidth={1.5} />` 变为 `<Icon weight="regular" />`，更语义化
