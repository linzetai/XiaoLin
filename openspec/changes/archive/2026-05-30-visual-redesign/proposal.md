# Visual Redesign — 前端视觉重构

## Why

FastClaw 当前界面存在几个视觉问题，影响了专业感和可用性：

1. **工具调用缺乏语义** — StepIndicator 是平面的 28px 行，所有工具类型（Shell、Read、Edit、Search）使用相同灰度配色，用户无法快速扫视辨别操作类型
2. **输入栏 frosted glass 在非 macOS 平台失效** — WebView2/WebKitGTK 缺少原生毛玻璃支持，导致半透明背景在 Linux/Windows 上渲染为灰色不透明块
3. **消息布局缺少身份识别** — 用户消息和 AI 消息仅靠文本标签区分，无头像或视觉锚点
4. **会话列表项信息密度低** — 单行文本加上 truncate，无预览、无图标锚点
5. **NavRail 交互反馈弱** — 无 tooltip 提示，active 状态仅靠颜色变化
6. **设置面板** — 弹窗模态框限制了信息密度，侧边栏宽度内表单拥挤

## What Changes

### 参考原型

`/home/linzetai/Downloads/fastclaw-redesign.html` — 一份完整的静态 HTML 原型，定义了目标视觉方向。

### 核心改造

| 区域 | 当前 | 目标 |
|------|------|------|
| **工具调用卡片** | 28px 平面行，无边框无分类色 | 36px 带边框卡片 + 24px 分类色图标徽章 |
| **输入栏** | frosted glass + 内阴影 | 干净的 1.5px 边框圆角卡片 + focus 发光 |
| **消息布局** | 文本标签 + 纯文字块 | 30px 圆形头像 + 用户消息气泡（非对称圆角） |
| **会话列表项** | 单行文本 | 30px 图标盒 + 双行（标题+预览文本） |
| **NavRail** | 44px 图标 | 54px 图标 + 暗色 tooltip 弹出 + 3px 左侧 active 指示条 |
| **设置** | 弹窗 + 设置项平铺 | 全页面布局，侧边栏 tab 导航（长期目标，本轮仅微调） |
| **CSS Tokens** | hex + rgba 值 | 新增工具类别色彩 tokens，保持现有系统兼容 |

### 非目标 (Not in Scope)

- **Dark theme 全面重做** — 本轮仅确保新 tokens 有 dark mode 对应值
- **Accent theme 适配** — Ocean/Sunset/Midnight 等 accent theme 暂不调整工具类别色
- **设置面板重构为全页面** — 这是一个独立的大型改动，本轮仅微调现有弹窗
- **Onboarding 流程** — 不在本轮范围内
- **响应式布局** — 移动端适配暂不考虑

## Capabilities

### New Capabilities
- `tool-category-colors`: 工具调用按类型着色（Shell=紫、Read=青、Write=绿、Edit=粉、Search=琥珀、Web=蓝绿、MCP=灰）
- `card-step-indicator`: 卡片式工具调用展示，替代平面行
- `avatar-messages`: 消息头像系统（用户渐变圆形 + AI 图标圆形）
- `clean-input-bar`: 干净边框输入栏，移除 frosted glass 依赖

### Modified Capabilities
- `session-list`: 升级列表项为图标盒 + 双行布局
- `nav-rail`: 增强交互（tooltip、active 指示条、notification dot）
- `design-tokens`: 新增工具类别色彩变量，调整 step 相关 tokens

## Impact

### CSS / Tokens
- `src/index.css` — 新增 `--tc-*` 工具类别色彩 tokens（light + dark）+ 调整 `--step-*` 尺寸

### Components (需修改)
- `StepIndicator.tsx` — 核心重设计：分类色徽章 + 卡片边框 + 状态点
- `StepGroup.tsx` — 适配新卡片样式，语义摘要行样式更新
- `StreamFooter.tsx` — 移除 frosted glass，改用 bordered card
- `MessageRenderer.tsx` — 消息头像渲染
- `UserInput.tsx` — 用户消息气泡样式
- `SessionList.tsx` — 列表项双行布局 + 图标盒
- `NavRail.tsx` — tooltip、active 指示条

### Components (可选优化)
- `SubAgentCard.tsx` — 适配新 StepIndicator 样式
- `DiffCard.tsx` — 适配新配色系统
- `StickyContextBar.tsx` — 微调样式一致性
