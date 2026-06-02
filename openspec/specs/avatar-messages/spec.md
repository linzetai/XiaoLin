# avatar-messages

## 概述

为消息添加头像系统，用户消息改为气泡样式，增强对话流中的身份识别。

## 用户消息

```
┌──────────────────────────────────────────────┐
│  ┌──┐  You  16:42                            │
│  │U │                                        │
│  └──┘  ┌──────────────────────────────────┐  │
│        │ 帮我看看 src/server.ts 里的...    │  │
│        └──────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

### 头像
- 30×30 px 圆形
- 渐变背景: `linear-gradient(135deg, var(--tint), color-mix(in srgb, var(--tint) 70%, #6366F1))`
- 白色粗体字母 "U"，12px

### 消息气泡
- `background: var(--bg-surface)`
- `border: 1px solid var(--separator)`
- `border-radius: 14px 14px 14px 4px` (左下角尖锐，模拟气泡尾巴)
- `padding: 12px 16px`
- `display: inline-block`

## AI 消息

```
┌──────────────────────────────────────────────────────┐
│  ┌──┐  XiaoLin  16:42  ⏱ 12.4s                     │
│  │🔺│                                                │
│  └──┘  让我先看一下现有的路由结构。                    │
│                                                       │
│        [tool call cards...]                           │
│                                                       │
│        项目使用 Express...                            │
│                                                       │
│        ← copy  👍  👎  ↻   (hover 显示)              │
└──────────────────────────────────────────────────────┘
```

### 头像
- 30×30 px 圆形
- `background: var(--bg-surface)`, `border: 1.5px solid var(--separator)`
- 内部: ClawIcon, 14px, `color: var(--fill-quaternary)`

### 消息文本
- 无气泡框（document flow）
- `font-size: 14px`, `line-height: 1.75`

### 消息头
- 名称: 13px, `font-weight: 650`
- 时间: 11px, `color: var(--fill-quaternary)`, tabular-nums
- 耗时 pill: 10.5px, `background: var(--bg-secondary)`, `border-radius: 10px`, 含⏱图标

### Action Bar
- `opacity: 0`, hover 时 `opacity: 1`
- 28×28 icon buttons: 复制、赞、踩、重新生成
- `border-radius: 6px`, hover `background: var(--bg-hover)`

## MessageAvatar 组件

新增独立组件 `MessageAvatar`（可在 MessageRenderer.tsx 内部定义），根据 `role` 渲染不同头像。

## 布局

消息容器改为 `flex gap-14px`：
```tsx
<div className="flex gap-[14px]">
  <MessageAvatar role={role} />
  <div className="flex-1 min-w-0">
    {/* head + body + actions */}
  </div>
</div>
```

## 影响范围

- `MessageRenderer.tsx` — AI 消息渲染
- `UserInput.tsx` — 用户消息渲染
- 可选: `SubAgentCard.tsx` — 子代理消息头像
