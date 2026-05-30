# tool-category-colors

## 概述

为工具调用按语义类型分配色彩编码，使用户在扫视对话流时能快速辨别操作类型。

## 类别定义

| 类别 | 工具名 | 色相 | Light BG | Light FG | 语义 |
|------|--------|------|----------|----------|------|
| `shell` | `shell`, `shell_exec`, `code_execute` | 紫 280° | `#EDE5F4` | `#6D44A0` | 系统命令执行 |
| `read` | `file_read`, `read_file`, `read_skill`, `list_skills`, `list_directory` | 青 200° | `#E0EDF3` | `#2D6A86` | 文件/信息读取 |
| `write` | `file_write`, `write_file`, `write_skill` | 绿 155° | `#DFF2E6` | `#2B6B48` | 文件创建/写入 |
| `edit` | `edit_file` | 粉 320° | `#F3E0EC` | `#8A3572` | 文件编辑/补丁 |
| `search` | `file_search`, `hub_search`, `memory_search` | 琥珀 60° | `#F0EAD4` | `#7B6225` | 本地搜索 |
| `web` | `web_search`, `web_fetch`, `http_fetch` | 蓝绿 180° | `#E0F0F0` | `#2D7070` | 网络请求 |
| `mcp` | `mcp_*` 前缀 | 灰 | `#E8E8ED` | `#5A5A65` | 外部 MCP 工具 |
| `default` | 其他所有 | — | `bg-secondary` | `fill-tertiary` | 未分类 |

## CSS 变量命名

```
--tc-{category}-bg   背景色
--tc-{category}-fg   前景色（图标+文字）
```

## Dark Mode

Dark mode 下 bg 使用 `rgba()` 半透明值（避免过于鲜艳），fg 使用高明度值。

## 使用方式

React 组件中通过 `getToolCategory(toolName)` 获取类别，然后注入 CSS 变量：

```tsx
const cat = getToolCategory(tool.name);
<div style={{
  background: `var(--tc-${cat}-bg)`,
  color: `var(--tc-${cat}-fg)`,
}}>
```
