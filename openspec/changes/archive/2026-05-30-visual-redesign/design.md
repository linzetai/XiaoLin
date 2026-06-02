# Visual Redesign — 设计文档

## 设计原则

1. **语义色彩** — 工具类型通过颜色编码，支持快速扫视
2. **结构化卡片** — 边框和圆角创建视觉容器，比平面行更有层次
3. **渐进增强** — 新 tokens 添加而非替换，现有 accent/dark theme 保持兼容
4. **平台无关** — 不依赖平台特有渲染能力（去 frosted glass）

## 1. CSS Token 系统扩展

### 1.1 工具类别色彩 Tokens

每个工具类别定义一对 `bg/fg` 变量，light/dark 各一套：

```
Light Mode:
  --tc-shell-bg:   #EDE5F4    --tc-shell-fg:   #6D44A0   (紫)
  --tc-read-bg:    #E0EDF3    --tc-read-fg:    #2D6A86   (青)
  --tc-write-bg:   #DFF2E6    --tc-write-fg:   #2B6B48   (绿)
  --tc-edit-bg:    #F3E0EC    --tc-edit-fg:    #8A3572   (粉)
  --tc-search-bg:  #F0EAD4    --tc-search-fg:  #7B6225   (琥珀)
  --tc-web-bg:     #E0F0F0    --tc-web-fg:     #2D7070   (蓝绿)
  --tc-mcp-bg:     #E8E8ED    --tc-mcp-fg:     #5A5A65   (灰)
  --tc-default-bg: var(--bg-secondary)
  --tc-default-fg: var(--fill-tertiary)

Dark Mode:
  --tc-shell-bg:   rgba(139,92,246,0.15)   --tc-shell-fg:   #A78BFA
  --tc-read-bg:    rgba(56,189,248,0.12)   --tc-read-fg:    #7DD3FC
  --tc-write-bg:   rgba(52,211,153,0.12)   --tc-write-fg:   #6EE7B7
  --tc-edit-bg:    rgba(244,114,182,0.12)  --tc-edit-fg:    #F9A8D4
  --tc-search-bg:  rgba(251,191,36,0.12)   --tc-search-fg:  #FCD34D
  --tc-web-bg:     rgba(45,212,191,0.12)   --tc-web-fg:     #5EEAD4
  --tc-mcp-bg:     rgba(148,163,184,0.12)  --tc-mcp-fg:     #94A3B8
```

### 1.2 Step Indicator 尺寸 Tokens

```
--step-height: 28px → 36px    (更多呼吸空间)
--step-gap: 2px → 3px
--step-border: var(--separator)
--step-radius: 8px
--step-icon-size: 24px
--step-icon-radius: 6px
```

### 1.3 位置

在 `:root` 和 `[data-theme="dark"]` 块中各添加一次。不需要在每个 accent theme 中重复（工具类别色是语义色，不随主题色变化）。

## 2. 工具调用卡片 (StepIndicator + StepGroup)

### 2.1 工具类别映射

```typescript
type ToolCategory = "shell" | "read" | "write" | "edit" | "search" | "web" | "mcp" | "default";

const CATEGORY_TOOLS: Record<string, ToolCategory> = {
  shell: "shell", shell_exec: "shell", code_execute: "shell",
  file_read: "read", read_file: "read", read_skill: "read",
  list_skills: "read", list_directory: "read",
  file_write: "write", write_file: "write", write_skill: "write",
  edit_file: "edit",
  file_search: "search", hub_search: "search", memory_search: "search",
  web_search: "web", web_fetch: "web", http_fetch: "web",
};
```

### 2.2 卡片布局 — StepIndicator

从当前的无边框行：
```
✓ 📄 读取文件 src/server.ts                    0.2s ▸
```

变为带边框的卡片：
```
┌──────────────────────────────────────────────────────┐
│  ┌────┐  Read  src/server.ts              ●  0.2s ▼ │
│  │ 📄 │                                              │
│  └────┘                                              │
└──────────────────────────────────────────────────────┘
```

结构：
```
<div class="tc">                           ← 1px border, 8px radius
  <button class="tc-h">                    ← header row (clickable)
    <div class="tico {category}">          ← 24x24 colored icon badge
      <icon />
    </div>
    <span class="tl">Read</span>           ← tool label (bold)
    <span class="tp">src/server.ts</span>  ← key info (mono, truncated)
    <span class="ts">                      ← status area
      <span class="sd {status}" />         ← 5px status dot (green/red/spinner)
      <span class="dur">0.2s</span>        ← duration
    </span>
    <ChevronDown class="tv" />             ← expand toggle
  </button>
  <div class="tc-bd">                      ← expandable body
    <!-- code / output / diff / error -->
  </div>
</div>
```

### 2.3 图标徽章样式

```css
.tico {
  width: var(--step-icon-size);      /* 24px */
  height: var(--step-icon-size);
  border-radius: var(--step-icon-radius);  /* 6px */
  display: grid;
  place-items: center;
  flex-shrink: 0;
}

/* 通过 CSS 变量动态着色 */
.tico { background: var(--_tc-bg); color: var(--_tc-fg); }
```

React 中通过 inline style 注入类别色：
```tsx
style={{
  "--_tc-bg": `var(--tc-${category}-bg)`,
  "--_tc-fg": `var(--tc-${category}-fg)`,
} as React.CSSProperties}
```

### 2.4 状态指示

- **运行中**: 5px 圆形 spinner（`border` 动画），背景微弱 tint
- **成功**: 5px 绿色实心圆 `var(--green)`
- **失败**: 5px 红色实心圆 `var(--red)`

移除原先的 14px 状态图标（Check/X/Spinner），用更小的状态点，减少左侧视觉重量。

### 2.5 StepGroup 适配

StepGroup 摘要行保持当前结构（语义文本 + 总计时间 + 展开/折叠），但:
- 增加 1px 底部分隔线
- 展开后的子 StepIndicator 使用新卡片样式但去掉外层边框（嵌套时无需双重边框）

### 2.6 展开区域样式（原型对齐）

```
tc-code:  代码预览区 — 浅灰背景, mono 字体, 行号, 语法高亮
tc-out:   命令输出区 — 浅绿背景, 等宽字体
tc-err:   错误输出区 — 浅红背景, 等宽字体
tc-diff:  差异对比区 — 绿底(+行) / 红底(-行)
```

## 3. 会话列表项升级

### 3.1 列表项结构

从：
```
模型身份确认
```

到：
```
┌──────────────────────────────────────────┐
│  ┌────┐  模型身份确认                     │
│  │ 💬 │  已完成模型初始化验证              │
│  └────┘                                   │
└──────────────────────────────────────────┘
```

### 3.2 图标盒

30x30 圆角方块，默认状态使用 `--bg-secondary` 背景 + `--separator` 边框，active 状态使用 accent 填充 + 白色图标。

### 3.3 Preview 文本

SessionList 数据中已有消息内容可用（最后一条消息摘要）。如果不可用，显示 "等待输入..." 占位。

## 4. 输入栏改造

### 4.1 外壳

移除 frosted glass：
```css
/* 移除 */
backdrop-filter: blur(...)
background: rgba(..., 0.78)

/* 替换为 */
border: 1.5px solid var(--separator);
border-radius: 18px;
background: var(--bg-surface);
```

### 4.2 Focus 状态

```css
.input-box:focus-within {
  border-color: var(--tint);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--tint) 8%, transparent);
}
```

### 4.3 模式切换

当前使用单独的 toggle pill，原型使用 segmented control：
```
┌──────────┬──────────┐
│  Agent   │   Plan   │
└──────────┴──────────┘
```

1px 边框分段控件，active 项使用 accent 背景。Plan active 时使用 `--plan-a` 色（紫色）。

## 5. 消息布局

### 5.1 用户消息

```
┌──────────────────────────────────────────┐
│  [U]  You  16:42                          │
│                                           │
│       ┌─────────────────────────────────┐ │
│       │ 用户消息文本...                 │ │
│       └─────────────────────────────────┘ │
└──────────────────────────────────────────┘
```

- 30px 圆形头像：渐变背景 (accent-ish) + 白色字母 "U"
- 消息文本在边框卡片中，`border-radius: 14px 14px 14px 4px`（气泡形状）
- 背景 `var(--bg-surface)`, 边框 `var(--separator)`

### 5.2 AI 消息

- 30px 圆形头像：`var(--bg-surface)` 背景 + `var(--separator)` 边框 + icon
- 消息文本 document flow（无气泡框）
- 显示名称 "XiaoLin" + 时间 + 耗时 pill
- hover 显示 action buttons（复制、点赞、踩、重新生成）

### 5.3 头像组件

```tsx
function MessageAvatar({ role }: { role: "user" | "assistant" }) {
  if (role === "user") {
    return (
      <div className="w-[30px] h-[30px] rounded-full grid place-items-center
                       text-[12px] font-bold text-white shrink-0"
           style={{ background: "linear-gradient(135deg, var(--tint), color-mix(in srgb, var(--tint) 70%, #6366F1))" }}>
        U
      </div>
    );
  }
  return (
    <div className="w-[30px] h-[30px] rounded-full grid place-items-center shrink-0"
         style={{ background: "var(--bg-surface)", border: "1.5px solid var(--separator)" }}>
      <ClawIcon size={14} />
    </div>
  );
}
```

## 6. NavRail 增强

### 6.1 尺寸

`--nav-rail-w: 48px → 54px`

### 6.2 Active 指示条

active 按钮左侧显示 3px 宽的 accent 色条：
```css
.nav-rail-btn.active::after {
  content: '';
  position: absolute;
  left: 0;
  width: 3px;
  height: 16px;
  background: var(--tint);
  border-radius: 0 3px 3px 0;
}
```

### 6.3 Tooltip

hover 时在按钮右侧 12px 处显示暗色 tooltip：
```css
[data-tooltip]::before {
  content: attr(data-tooltip);
  position: absolute;
  left: calc(100% + 12px);
  top: 50%;
  transform: translateY(-50%);
  padding: 4px 10px;
  background: oklch(18% 0.01 250);
  color: oklch(92% 0.005 250);
  font-size: 12px;
  font-weight: 500;
  border-radius: 6px;
  white-space: nowrap;
  opacity: 0;
  pointer-events: none;
  transition: opacity 100ms;
  z-index: 200;
}
[data-tooltip]:hover::before { opacity: 1; }
```

### 6.4 Notification Dot

在技能按钮右上角放一个 7px 红色圆点（有新 skill 可用时）。

## 7. 动画与过渡

### 7.1 工具调用展开

使用 `grid-template-rows: 0fr → 1fr` 过渡（260ms，原型的 `--ease`）:
```css
.tc-bd {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 260ms cubic-bezier(0.23, 1, 0.32, 1);
}
.tc.open .tc-bd { grid-template-rows: 1fr; }
.tc-bd-in { overflow: hidden; }
```

### 7.2 消息入场

```css
@keyframes mIn {
  from { opacity: 0; transform: translateY(4px); }
}
.message { animation: mIn 220ms cubic-bezier(0.23, 1, 0.32, 1) both; }
```

## 8. 不变的部分

- **Tailwind CSS 4** 继续使用，不引入新的 CSS 框架
- **Geist 字体族** 保持不变（原型使用系统字体，但 Geist 是 XiaoLin 的品牌字体）
- **Zustand 状态管理** 不涉及变化
- **WebSocket 协议** 不涉及变化
- **后端 Rust 代码** 不涉及变化
