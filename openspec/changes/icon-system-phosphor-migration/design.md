## Context

XiaoLin 前端使用 Lucide React（70+ 文件 import），图标 size 和 strokeWidth 值高度分散。已有 `ui-tokens.ts` 定义了 ICON.sm/md/lg，但覆盖率不足 60%。项目是 Tauri v2 + React 桌面应用，图标仅在 WebView 层使用。

当前的痛点是 Lucide 只提供单一线条风格，靠 strokeWidth 数值微调来表达状态差异，导致组件间不一致。Phosphor 的 weight 系统天然解决了这个问题。

## Goals / Non-Goals

**Goals:**
- 将所有图标统一到 Phosphor Icons，获得 weight 语义表达力
- 建立明确的 Token 系统：size scale + weight 语义 + color 语义
- 通过 IconContext.Provider 实现全局默认配置，减少组件层面的重复 props
- 保证迁移后 UI 视觉效果不退化（相同尺寸和密度）

**Non-Goals:**
- 不重新设计 UI 布局或间距
- 不引入 Phosphor 的 duotone/fill 变体做大规模视觉改版（未来可做）
- 不处理 Tauri 原生层图标（app icon、tray icon 走 PNG）
- 不做自定义 SVG 图标到 Phosphor 的替换（ClawIcon 保留）

## Decisions

### 1. 全局 Weight 策略

| UI 场景 | Phosphor Weight | 取代原来的 |
|---------|----------------|-----------|
| 常规图标（默认） | `regular` | strokeWidth 1.5 |
| 细线/装饰/窗口控制 | `light` | strokeWidth 1.0~1.2 |
| 强调/CTA/active 状态 | `bold` | strokeWidth 2.0+ |
| 激活/选中（如 tab 选中） | `fill` | strokeWidth 2 + fill |
| 空态/大图标 | `thin` | strokeWidth 1.0~1.2 + size 24~32 |

**决策原因**: Phosphor 的 weight 是设计师手绘的不同视觉密度，比等比缩放 strokeWidth 更优（光学补偿）。

### 2. Size Scale

```typescript
export const ICON_SIZE = {
  xs: 12,   // 徽章、状态指示器、内嵌小图标
  sm: 14,   // 紧凑 UI、按钮内图标（当前 ICON.sm）
  md: 16,   // 侧栏导航、标准控件（当前 ICON.md）
  lg: 20,   // 标题区（当前 ICON.lg）
  xl: 24,   // Empty state、Phosphor 原始设计尺寸
  "2xl": 32, // 大空态、Hero
} as const;
```

**决策原因**: 与当前 ICON token 的 14/16/20 保持兼容，同时补充 xs(12) 和 xl/2xl 覆盖现有的 10~11px 和 24~32px 场景。

### 3. IconContext.Provider 配置

```tsx
<IconContext.Provider value={{
  size: ICON_SIZE.sm,    // 14px — 全局默认，适合紧凑桌面 UI
  weight: "regular",     // 默认 weight
  color: "currentColor", // 跟随文本颜色
  mirrored: false,
}}>
```

**决策原因**: 14px + regular 是当前项目最高频的组合，设为默认后大部分图标不需要传任何 props。

### 4. 迁移方式：手动逐文件 vs 脚本

选择**半自动**：写一个 Node.js 脚本做 import 路径替换 + 图标名映射，复杂 case（带 strokeWidth 语义转换的）手动调整。

**决策原因**: 
- 纯手动改 70+ 文件太耗时且容易漏
- 纯自动无法处理 strokeWidth → weight 的语义判断
- 半自动方案：脚本处理 80% 的简单 case，人工处理 20% 的特殊 case

### 5. 图标名映射策略

Lucide 和 Phosphor 的命名差异不大，大部分是 1:1 映射：

| Lucide | Phosphor | 备注 |
|--------|----------|------|
| Search | MagnifyingGlass | 名称不同 |
| MessageCircle | ChatCircle | 名称不同 |
| Settings | Gear | 名称不同 |
| Terminal | Terminal | 相同 |
| Plus | Plus | 相同 |
| X | X | 相同 |
| ChevronDown | CaretDown | 名称不同 |
| FileText | FileText | 相同 |
| Loader2 | SpinnerGap | 名称不同 |

完整映射表在实施时建立，约 50~60 个唯一图标名。

## Risks / Trade-offs

| 风险 | 缓解措施 |
|------|---------|
| 视觉差异：Phosphor regular 比 Lucide 1.5sw 更粗/更圆 | 对比截图验证，必要时用 light weight 替代 |
| Bundle size 增加 | Tree-shaking 仍有效，监控构建产物大小 |
| 迁移过程中 UI 临时混搭两种风格 | 一次性完成所有文件，不做分批迁移 |
| 部分 Lucide 图标在 Phosphor 中无对应 | 用最接近的 Phosphor 图标替代，或保留为自定义 SVG |
| 自定义 SVG（ClawIcon）与 Phosphor 风格不完全统一 | 保留自定义 SVG 不变，后续按需重绘 |
