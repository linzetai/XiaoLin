# card-step-indicator

## 概述

将工具调用从 28px 平面行重设计为 36px 带边框的卡片，每张卡片包含分类色图标徽章。

## 卡片结构

```
┌─ 1px border, 8px radius ─────────────────────────────┐
│  ┌──────┐                                             │
│  │ icon │  Label  path/arg       ● status  1.2s  ▼   │
│  │ 24px │                                             │
│  └──────┘                                             │
├─ border-top (展开时) ─────────────────────────────────┤
│  代码 / 输出 / diff / 错误内容                        │
└───────────────────────────────────────────────────────┘
```

## 图标徽章

- 尺寸: 24×24 px (`--step-icon-size`)
- 圆角: 6px (`--step-icon-radius`)
- 颜色: 由 `--tc-{category}-bg/fg` 驱动
- 图标: 来自 lucide-react，14px (ICON.sm)

## 状态指示

| 状态 | 显示 |
|------|------|
| `running` | 5px 圆形 border spinner + 背景微弱 tint |
| `success` | 5px green 实心圆 |
| `error` | 5px red 实心圆 |

## 展开/折叠

- 折叠时仅显示 header 行
- 展开使用 `grid-template-rows: 0fr → 1fr` 过渡 (260ms)
- 展开区域包含:
  - 参数预览 (JSON pretty-print)
  - 输出/结果
  - 差异预览 (edit_file)
  - 错误信息

## 特殊结果卡片

以下工具的结果不走通用展开逻辑，而是在卡片下方直接渲染专用组件：
- `todo_write` → `TodoCard`
- `edit_file` → `DiffCard`
- `exit_plan_mode` → `PlanApprovalCard`

## 嵌套场景

StepGroup 内部的 StepIndicator 使用紧凑变体（无外层边框，仅保留图标徽章和水平分隔线），避免双重边框嵌套。

## 向后兼容

- `ToolCall` 类型接口不变
- `extractKeyInfo()` 导出不变
- `ImageViewer` 导出不变
