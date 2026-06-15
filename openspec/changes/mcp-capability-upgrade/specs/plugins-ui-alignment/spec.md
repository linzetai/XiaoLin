# Plugins UI 风格统一 Spec

> 将 PluginsView 从"独立页面"风格对齐到 XiaoLin 主界面的设计语言。

## 问题定义

当前 `PluginsView.tsx`（~1200 行）的视觉风格与主界面（Sidebar、MessageStream、ComposerCore）存在 **8 类不一致**：

| # | 差异类别 | PluginsView 现状 | 主界面规范 |
|---|---------|-----------------|-----------|
| 1 | Icon 尺寸 | 硬编码 `size={18/14/13/12/11/10}` | `ICON_SIZE.xs/sm/md/lg` from `ui-tokens.ts` |
| 2 | 字号系统 | `text-[17px/14px/13px/12px/11px/10px]` 6 个散乱值 | 收敛到 3-4 档：12/13/14/16 |
| 3 | 页面 Header | Hero 式图标+标题+副标题 | 主界面无此模式，轻量化对齐 |
| 4 | Tab Bar | 手写 inline style，无复用组件 | 应抽取 `<SegmentedControl>` 共享组件 |
| 5 | Button variants | inline `style={{}}` + 手动 hover | 使用 `BTN_ICON` token + Tailwind class |
| 6 | 内容宽度 | `max-w-[clamp(560px,65%,800px)]` | `--content-max-w` CSS variable |
| 7 | 动画 | 自带 `<style>{ANIM_CSS}</style>` 注入 | 使用 `index.css` 的 `--duration-*` / `--ease-*` / `--stagger` |
| 8 | 国际化 | 全英文硬编码 | 使用 `useTranslation` + i18n keys |

## 设计原则

1. **Token-first**：所有视觉属性（尺寸、间距、颜色、圆角、动画）必须来自 `ui-tokens.ts` 或 `index.css` CSS variables
2. **Component-level DRY**：Tab Bar、StatusDot、Action Button 等重复模式抽为共享组件
3. **Progressive Disclosure**：空状态、加载态、错误态统一使用主界面已有的模式
4. **i18n-complete**：所有用户可见文本走翻译

## 变更清单

### 变更 1: Icon 尺寸统一

**文件**: `PluginsView.tsx`

将所有 `size={N}` 替换为 `ICON_SIZE` token：

| 当前硬编码 | 替换为 | 使用场景 |
|-----------|--------|---------|
| `size={18}` | `ICON_SIZE.md` (16) | Header 图标 |
| `size={14}` | `ICON_SIZE.sm` (14) | 行内操作图标 |
| `size={13}` | `ICON_SIZE.sm` (14) | 按钮内图标 |
| `size={12}` | `ICON_SIZE.xs` (12) | 次要信息图标 |
| `size={11}` | `ICON_SIZE.xs` (12) | 标签/badge 图标 |
| `size={10}` | `ICON_SIZE.xs` (12) | 最小图标统一到 xs |
| `size={20}` | `ICON_SIZE.lg` (20) | Toggle 图标 |
| `size={32}` | `ICON_SIZE["2xl"]` (32) | 空状态大图标 |

### 变更 2: 字号收敛

| 当前字号 | 收敛为 | 语义角色 |
|---------|--------|---------|
| `text-[17px]` | `text-[16px]` (heading) | 页面标题 |
| `text-[14px]` | `text-sm` (14px) | 卡片主标题 |
| `text-[13px]` | `text-[13px]` (body) | 正文/描述 |
| `text-[12px]` | `text-xs` (12px) | 辅助信息/状态 |
| `text-[11px]` | `text-[11px]` (caption) | Tab label/section header |
| `text-[10px]` | `text-[11px]` 向上合并 | Badge/tag 统一到 11px |

### 变更 3: Header 轻量化

将当前的 hero header：
```
[图标方块] Plugins
           Extend capabilities with MCP servers, skills & channels
```

对齐为主界面的 flat header 风格，与 SettingsPanel 或 AutomationView 统一。去掉图标方块装饰，保留标题和 Tab Bar。

### 变更 4: Tab Bar → SegmentedControl 组件

抽取 `<SegmentedControl>` 共享组件：

```tsx
// components/common/SegmentedControl.tsx
interface SegmentedControlProps<T extends string> {
  value: T;
  onChange: (val: T) => void;
  items: { value: T; label: string; count?: number }[];
}
```

替换 PluginsView L57-78 和 Skills sub-tab L316-333 的两处手写实现。

### 变更 5: Button 统一

所有操作按钮使用 `BTN_ICON.sm` 或对应 class，删除 inline `style={{ cursor, background, border }}` 模式。

需要新增的 button variant：
- `BTN_TEXT_SM`：文字按钮（如 "Reload", "Disconnect"）
- `BTN_PRIMARY_SM`：主操作按钮（如 "Connect", "Get QR Code"）

### 变更 6: 内容宽度对齐

```diff
- max-w-[clamp(560px,65%,800px)]
+ style={{ maxWidth: "var(--content-max-w)" }}
```

### 变更 7: 动画迁移

删除 `ANIM_CSS` 字符串和 `<style>` 注入，改用 `index.css` 已有的 token：

| PluginsView 动画 | 迁移到 |
|-----------------|--------|
| `pvFadeIn 220ms cubic-bezier(0.16, 1, 0.3, 1)` | `animate-[fadeIn_var(--duration-normal)_var(--ease-out)]` |
| `pvFloat 4s ease-in-out infinite` | 删除（装饰性，与主界面不搭） |
| `pvFadeUp + --stagger-i * 40ms` | 使用 `--stagger: 30ms` + CSS animation |

### 变更 8: 国际化

新增 `locales/zh/plugins.json` 和 `locales/en/plugins.json`：

```json
{
  "title": "插件",
  "subtitle": "通过 MCP 服务器、技能包和频道扩展能力",
  "tab_mcp": "MCP 服务器",
  "tab_skills": "技能",
  "tab_channels": "频道",
  "reload": "重新加载",
  "loading": "加载中…",
  "no_mcp_title": "暂无 MCP 服务器",
  "no_mcp_desc": "在配置中添加 MCP 服务器以扩展 Agent 能力",
  "connected_count": "{{connected}}/{{total}} 已连接",
  "tools_count": "{{count}} 工具",
  "restart": "重启",
  "disable": "禁用",
  "enable": "启用"
}
```

## 影响的文件

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `components/plugins/PluginsView.tsx` | 重构 | 主体文件，所有 8 项变更 |
| `lib/ui-tokens.ts` | 新增 | `BTN_TEXT_SM` / `BTN_PRIMARY_SM` button variants |
| `components/common/SegmentedControl.tsx` | 新增 | 抽取的共享 Tab Bar 组件 |
| `index.css` | 新增 | 将 `pvFadeIn`/`pvFadeUp` 动画迁移为全局 keyframes |
| `locales/zh/plugins.json` | 新增 | 中文翻译 |
| `locales/en/plugins.json` | 新增 | 英文翻译 |
| `locales/zh/settings.json` | 可能修改 | 移除已迁移的 key |

## 验证标准

1. PluginsView 中零 `size={N}` 硬编码 → 全部引用 `ICON_SIZE`
2. `text-[10px]` 消失，最小字号统一到 `text-[11px]`
3. 无 `<style>` 注入
4. 无 inline `style={{ cursor, background, border }}` 按钮模式
5. `SegmentedControl` 组件被至少两处使用
6. `useTranslation("plugins")` 覆盖所有可见文本
7. 视觉回归：整体观感与 SettingsPanel、AutomationView 保持一致的信息密度和节奏感
