# clean-input-bar

## 概述

将输入栏从 frosted glass 效果改为干净的边框卡片，解决非 macOS 平台的渲染问题。

## 当前问题

`StreamFooter.tsx` 使用 `backdrop-filter: blur()` + 半透明背景 + 内阴影创建 frosted glass 效果。在以下平台表现不佳：

- **Linux (WebKitGTK)**: 无原生 blur 支持，退化为灰色不透明块
- **Windows (WebView2)**: blur 行为不一致，边缘可能出现渲染伪影

## 目标设计

```
┌─ 1.5px border, 18px radius ───────────────────────────┐
│  ┌─────────────────────────────────────────────────┐  │
│  │ textarea                                         │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─ toolbar ───────────────────────────────────────┐  │
│  │ [model▾] [📎]          [Agent|Plan]  [🎤] [↑]  │  │
│  └─────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────┘
[📁 /home/linzetai]  [📄 6 文件已索引]    ← context chips
```

## CSS 变更

```css
/* 移除 */
backdrop-filter: blur(20px);
-webkit-backdrop-filter: blur(20px);
background: var(--bg-sidebar);     /* 半透明 */
box-shadow: inset 0 0.5px 0 ...;   /* 内阴影 */

/* 替换 */
border: 1.5px solid var(--separator);
border-radius: 18px;
background: var(--bg-surface);      /* 实色 */
transition: border-color 140ms, box-shadow 140ms;

/* focus 状态 */
&:focus-within {
  border-color: var(--tint);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--tint) 8%, transparent);
}
```

## 模式切换 (Segmented Control)

原有 toggle pill 改为分段控件:

```
┌──────────┬──────────┐
│  Agent   │   Plan   │
└──────────┴──────────┘
```

- 外框: `1px solid var(--separator)`, `border-radius: 8px`
- Active 项: `background: var(--tint-bg)`, `color: var(--tint)`
- Plan active: 使用紫色系（`oklch(56% 0.18 310)` / `oklch(94% 0.05 310)`）

## 发送按钮

- 尺寸: 34×34 px
- 背景: `var(--tint)`
- 圆角: 12px
- hover: 上移 1px + 深色变体

## Context Chips

输入栏下方显示当前上下文信息：
- 工作目录 pill
- 已索引文件数 pill
- 小圆角、`var(--bg-surface)` 背景、`1px border`
