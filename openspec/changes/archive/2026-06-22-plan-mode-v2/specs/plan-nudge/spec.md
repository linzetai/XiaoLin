## ADDED Requirements

### Requirement: 关键词触发 Plan Nudge
当用户在 Composer 中输入包含 plan 相关关键词的文本时，SHALL 显示内联 Plan Nudge 提示条。

#### Scenario: 中文关键词触发
- **WHEN** 用户输入文本包含以下独立词: "规划"、"方案"、"设计方案"、"架构设计"、"重构方案"、"实施计划"
- **AND** 当前不在 Plan 模式
- **AND** 输入不以 "/" 开头（非 slash command）
- **THEN** SHALL 在 Composer 输入区域和底部工具栏之间显示 Plan Nudge 提示条
- **THEN** 提示条内容: "🧭 试试 Plan 模式来制定完整方案？ [切换到 Plan] Ctrl+Shift+P"

#### Scenario: 英文关键词触发
- **WHEN** 用户输入文本包含以下独立词 (word boundary, case-insensitive): "plan", "design", "architecture", "refactor plan"
- **AND** 当前不在 Plan 模式
- **THEN** SHALL 显示 Plan Nudge 提示条

#### Scenario: 已在 Plan 模式不触发
- **WHEN** 当前 executionMode === "plan"
- **THEN** SHALL 不显示任何 nudge

#### Scenario: 频率控制
- **THEN** 每个 session 对同一关键词类别最多触发 1 次
- **THEN** 两次 nudge 之间至少间隔 5 分钟

### Requirement: 复杂度启发式触发 Plan Nudge
当用户输入具有复杂任务特征的文本时，SHALL 通过启发式检测并显示 Plan Nudge。

#### Scenario: 多维度复杂度检测
- **WHEN** 用户输入文本满足以下条件中的 2 个或以上（debounce 800ms）:
  - 文本长度 > 200 字符
  - 包含编号列表（"1." "2." "3." 或连续 "- " 项，≥ 3 项）
  - @文件引用 ≥ 3 个
  - 包含多个动词短语模式（"添加...修改...删除..." 或 "create...update...delete..."）
- **AND** 当前不在 Plan 模式
- **THEN** SHALL 显示 Plan Nudge 提示条
- **THEN** 提示条内容: "🧭 这看起来是一个复杂任务。使用 Plan 模式可以先规划再实施。 Ctrl+Shift+P"

#### Scenario: 每 session 限制
- **THEN** 复杂度 nudge 每个 session 最多触发 2 次

### Requirement: 首次使用提示 Plan Nudge
对于从未使用过 Plan 模式的用户，SHALL 在合适时机显示教育性提示。

#### Scenario: 首次使用教育提示
- **WHEN** localStorage 中无 "xiaolin:plan-mode-ever-used" 记录
- **AND** 当前 session 已有 ≥ 5 条消息
- **AND** 最后一条用户消息 > 100 字符
- **AND** 当前不在 Plan 模式
- **THEN** SHALL 显示 Plan Nudge 提示条
- **THEN** 提示条内容: "🧭 知道吗？Plan 模式可以帮你先制定方案再执行。 Ctrl+Shift+P"

#### Scenario: 全局限制
- **THEN** 首次使用提示全局（across sessions）最多显示 3 次

#### Scenario: 使用后标记
- **WHEN** 用户通过任何方式进入 Plan 模式
- **THEN** SHALL 在 localStorage 设置 "xiaolin:plan-mode-ever-used" = timestamp

### Requirement: Plan Nudge UI 渲染
Plan Nudge SHALL 以内联提示条形式渲染在 Composer 内部。

#### Scenario: 提示条样式
- **THEN** 位置: Composer 输入区域下方、底部工具栏上方
- **THEN** 背景: 半透明 plan-tint 色 (`color-mix(in srgb, var(--plan-tint) 6%, transparent)`)
- **THEN** 边框: `0.5px solid color-mix(in srgb, var(--plan-tint) 15%, transparent)`
- **THEN** 内容: 左侧 🧭 图标 + 文案 + [切换到 Plan] 按钮 + 快捷键提示 + 右侧 ✕ dismiss 按钮
- **THEN** 进入动画: slideDown (150ms ease-out)
- **THEN** 退出动画: fadeOut (100ms)

#### Scenario: 按钮交互
- **WHEN** 用户点击 [切换到 Plan] 按钮
- **THEN** SHALL 切换到 Plan 模式 + nudge 消退

#### Scenario: 自动消退
- **WHEN** nudge 显示 10 秒后
- **AND** 用户未 hover 在 nudge 区域
- **THEN** SHALL 自动 fadeOut 消退

### Requirement: Plan Nudge 消退条件
Plan Nudge SHALL 在以下条件下消退。

#### Scenario: Escape 关闭
- **WHEN** nudge 显示中用户按 Escape
- **THEN** SHALL 立即消退
- **THEN** 本 session 该触发规则不再触发

#### Scenario: 点击 dismiss
- **WHEN** 用户点击 ✕ 按钮
- **THEN** SHALL 立即消退 + 本 session 该规则不再触发

#### Scenario: 发送消息后消退
- **WHEN** 用户按 Enter 发送消息
- **THEN** nudge SHALL 消退（不计入 dismiss 记录）

#### Scenario: 输入清空后消退
- **WHEN** 用户清空输入文本
- **THEN** nudge SHALL 消退（不计入 dismiss 记录）

### Requirement: Plan 模式快捷键
Composer 获得焦点时 SHALL 支持 Ctrl+Shift+P 快捷键切换 Plan 模式。

#### Scenario: 快捷键切换到 Plan
- **WHEN** Composer 有焦点且当前为 Agent 模式
- **AND** 用户按 Ctrl+Shift+P (Windows/Linux) 或 Cmd+Shift+P (macOS)
- **THEN** SHALL 切换到 Plan 模式
- **THEN** 等效于点击 ModeSelector → Plan

#### Scenario: 快捷键切换回 Agent
- **WHEN** 当前为 Plan 模式且用户按 Ctrl+Shift+P
- **THEN** SHALL 切换回 Agent 模式

#### Scenario: Nudge 显示时快捷键
- **WHEN** Plan Nudge 正在显示且用户按 Ctrl+Shift+P
- **THEN** SHALL 切换模式 + nudge 消退

#### Scenario: Streaming 时不响应
- **WHEN** 正在 streaming
- **THEN** SHALL 不响应 Ctrl+Shift+P（防止误操作）

### Requirement: Nudge 状态持久化
Plan Nudge 的 dismiss 记录和使用统计 SHALL 持久化到 localStorage。

#### Scenario: localStorage 键
- **THEN** SHALL 使用以下键:
  - `xiaolin:plan-nudge-dismissed-{sessionId}`: JSON array of dismissed rule IDs
  - `xiaolin:plan-mode-ever-used`: timestamp (首次使用 plan 模式的时间)
  - `xiaolin:plan-nudge-last-shown`: timestamp (上次显示 nudge 的时间)
  - `xiaolin:plan-nudge-education-count`: number (首次使用提示已显示次数)

## Implementation Reference

### 竞品对标

| 能力 | Codex | Claude Code | XiaoLin (目标) |
|------|-------|-------------|----------------|
| 关键词检测 | ✅ 仅 "plan" 一词 | ❌ | ✅ 中英文多关键词 |
| 复杂度启发式 | ❌ | ❌ | ✅ (差异化!) |
| 使用频率提醒 | ❌ | ✅ >7天 tip | ✅ 首次使用教育 |
| 快捷键 | ✅ Shift+Tab | ✅ Shift+Tab/Meta+M | ✅ Ctrl+Shift+P |
| UI 形态 | 终端底行文字 | 无 nudge | ✅ GUI 内联提示条 |
| 智能程度 | 单词匹配 | 无 | ✅ 多维度启发式 |
| 消退控制 | Esc per-thread | N/A | ✅ 多级频率控制 |

### 技术实现要点

**1. Nudge 检测 Hook**

```typescript
// usePlanNudge.ts
interface NudgeState {
  visible: boolean;
  message: string;
  ruleId: 'keyword' | 'complexity' | 'education';
}

function usePlanNudge(text: string, mentions: InlineMention[], executionMode: string, sessionId: string): NudgeState {
  const [nudge, setNudge] = useState<NudgeState>({ visible: false, message: '', ruleId: 'keyword' });

  // 关键词检测（即时）
  useEffect(() => {
    if (executionMode === 'plan' || text.startsWith('/')) { setNudge(n => ({...n, visible: false})); return; }
    if (isDismissed(sessionId, 'keyword')) return;
    if (containsPlanKeyword(text)) {
      setNudge({ visible: true, message: '...', ruleId: 'keyword' });
    }
  }, [text, executionMode]);

  // 复杂度检测（debounce 800ms）
  useEffect(() => {
    const timer = setTimeout(() => {
      if (executionMode === 'plan') return;
      if (isDismissed(sessionId, 'complexity')) return;
      if (detectComplexity(text, mentions)) {
        setNudge({ visible: true, message: '...', ruleId: 'complexity' });
      }
    }, 800);
    return () => clearTimeout(timer);
  }, [text, mentions, executionMode]);

  return nudge;
}
```

**2. 关键词匹配**

```typescript
const PLAN_KEYWORDS_ZH = ['规划', '方案', '设计方案', '架构设计', '重构方案', '实施计划'];
const PLAN_KEYWORDS_EN = ['plan', 'design', 'architecture', 'refactor'];

function containsPlanKeyword(text: string): boolean {
  const lower = text.toLowerCase();
  for (const kw of PLAN_KEYWORDS_ZH) {
    if (text.includes(kw)) return true;
  }
  for (const kw of PLAN_KEYWORDS_EN) {
    const regex = new RegExp(`\\b${kw}\\b`, 'i');
    if (regex.test(lower)) return true;
  }
  return false;
}
```

**3. 复杂度启发式**

```typescript
function detectComplexity(text: string, mentions: InlineMention[]): boolean {
  let score = 0;
  if (text.length > 200) score++;
  if (/(?:^|\n)\s*(?:\d+[\.\)]\s|[-*]\s){3,}/m.test(text)) score++;
  if (mentions.filter(m => m.type === 'file').length >= 3) score++;
  if ((text.match(/(?:添加|修改|删除|创建|更新|移除|create|update|delete|add|remove|modify)/gi) || []).length >= 3) score++;
  return score >= 2;
}
```

**4. 快捷键集成 (MentionInput)**

```typescript
// 在 handleKeyDown 中追加:
if (isMod && e.shiftKey && e.key === 'P') {
  e.preventDefault();
  onTogglePlanMode?.(); // 新增 prop
  return;
}
```

### Nudge 提示条布局参考

```
┌── Composer ────────────────────────────────────────────────────┐
│  [textarea 输入区域]                                           │
│                                                                 │
│  ┌─ Plan Nudge ──────────────────────────────────────── ✕ ──┐ │
│  │  🧭 这看起来是一个复杂任务。使用 Plan 模式可以先规划再    │ │
│  │     实施。 [切换到 Plan ↗] ⌃⇧P                           │ │
│  └───────────────────────────────────────────────────────────┘ │
│                                                                 │
│  [+] [Permission ▾]            [Model ▾] [Send ↩]              │
└─────────────────────────────────────────────────────────────────┘
```
