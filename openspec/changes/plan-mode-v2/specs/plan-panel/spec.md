## MODIFIED Requirements

### Requirement: PlanPanel 渲染模式
PlanPanel SHALL 支持两种渲染模式：
1. **流式模式**（Phase 2）— 接收 `plan_delta` 事件，逐行累积并渲染 markdown，末尾显示闪烁光标
2. **静态模式**（Phase 1）— 接收 `plan_file_update` 事件中的 `content` 字段直接渲染，无需 HTTP refetch

#### Scenario: 从流式切换到静态
- **WHEN** PlanPanel 正在流式模式接收 `plan_delta`，然后收到 `plan_file_update`
- **THEN** SHALL 切换到静态模式，移除光标，如 `plan_file_update` 带 `content` 字段则替换当前内容

#### Scenario: 无 plan_delta 时退化为静态
- **WHEN** agent 写入 plan 文件但未发送 `plan_delta` 事件（Phase 1 或 fallback）
- **THEN** PlanPanel SHALL 通过 `plan_file_update` 的 `content` 字段渲染，无需 HTTP refetch

#### Scenario: Phase 1 Content Push 消除 fetch 延迟
- **WHEN** 收到 `plan_file_update` 事件且含 `content` 字段
- **THEN** PlanPanel SHALL 直接使用该 content 渲染，不调用 `getPlanFile()` HTTP 请求
- **WHEN** 收到 `plan_file_update` 事件但无 `content` 字段（兼容旧版）
- **THEN** PlanPanel SHALL 降级为调用 `getPlanFile()` 获取内容

### Requirement: PlanPanel 流式 markdown 渲染性能
流式模式下 PlanPanel SHALL 采用「按行 commit」策略：delta 累积到 buffer，遇到换行符时才 commit 到渲染内容，避免每个字符都触发 react-markdown 重渲染。

#### Scenario: 大 plan 文件流式不卡顿
- **WHEN** plan 内容超过 2000 字符且仍在接收 delta
- **THEN** 每次行 commit 到渲染完成的延迟 SHALL 小于 50ms

#### Scenario: 按行 commit 策略
- **WHEN** 收到的 delta 不含换行符
- **THEN** SHALL 只追加到 buffer，不触发 react-markdown 重渲染
- **WHEN** 收到的 delta 含换行符
- **THEN** SHALL 将 buffer 中最后一个换行符之前的内容 commit 到 stableContent，触发渲染

### Requirement: PlanPanel 流式视觉反馈
流式模式下 PlanPanel SHALL 提供视觉指示器表示正在写入。

#### Scenario: 闪烁光标
- **WHEN** PlanPanel 处于流式模式
- **THEN** SHALL 在 markdown 内容末尾显示闪烁光标（竖线，0.8s 周期 blink 动画）
- **WHEN** 流式结束（收到 `plan_file_update` 或 `isComplete=true`）
- **THEN** SHALL 移除闪烁光标

#### Scenario: 新行渐入动画
- **WHEN** 流式模式下有新行被 commit 到渲染
- **THEN** 新行 SHALL 以 fadeSlideIn 动画（0.15s ease-out）出现

#### Scenario: 自动滚动
- **WHEN** PlanPanel 处于流式模式且新内容超出可视区域
- **THEN** SHALL 自动滚动到底部
- **WHEN** 用户手动向上滚动
- **THEN** SHALL 暂停自动滚动，直到用户滚回底部

### Requirement: Chat 流中的 Plan 更新提示
当 plan 文件被 write_file 工具写入时，chat 消息流中 SHALL 显示轻量提示而非完整 plan 内容。

#### Scenario: write_file plan 的工具结果展示
- **WHEN** write_file 工具成功写入 plan 文件
- **THEN** chat 消息流中的 ToolResult SHALL 显示为「方案已更新」（轻量 hint），而非展开完整文件内容
- **THEN** hint 旁 SHALL 包含「点击查看」链接，点击后打开/激活 PlanPanel

### Requirement: PlanPanel Plan 色系统
PlanPanel SHALL 使用统一的 plan 模式颜色 token，与 plan 相关的所有 UI 元素保持视觉一致。

#### Scenario: 统一 plan 色
- **THEN** PlanPanel 头部背景 SHALL 使用 `--plan-tint-soft`
- **THEN** PlanPanel 图标和标题 SHALL 使用 `--plan-tint`
- **THEN** plan 模式 banner、审批卡片、工具 hint 的强调色 SHALL 均使用同一 `--plan-tint` token

## Implementation Reference

### Phase 1: Content Push 前端改动

PlanPanel 监听 `plan_file_update` 事件时，优先使用事件中的 `content` 字段：

```typescript
useEffect(() => {
  const unsub = onWsEvent("plan_file_update", (msg) => {
    const data = msg.data;
    if (data?.sessionId === sessionId) {
      if (data.content !== undefined) {
        setContent(data.content as string);
        setLoading(false);
      } else {
        fetchContent(); // 兼容旧版
      }
    }
  });
  return unsub;
}, [sessionId, fetchContent]);
```

### Phase 2: 流式渲染前端核心逻辑

Delta 累积 + 按行 commit 策略（参考 Codex PlanStreamController）：

```typescript
const [stableContent, setStableContent] = useState("");
const bufferRef = useRef("");
const isStreamingRef = useRef(false);

function onPlanDelta(delta: string) {
  isStreamingRef.current = true;
  bufferRef.current += delta;

  const lastNewline = bufferRef.current.lastIndexOf('\n');
  if (lastNewline >= 0) {
    const committed = bufferRef.current.substring(0, lastNewline + 1);
    setStableContent(prev => prev + committed);
    bufferRef.current = bufferRef.current.substring(lastNewline + 1);
  }
}

function onStreamComplete(finalContent?: string) {
  isStreamingRef.current = false;
  if (finalContent !== undefined) {
    setStableContent(finalContent);
  } else if (bufferRef.current) {
    setStableContent(prev => prev + bufferRef.current);
  }
  bufferRef.current = "";
}
```

### CSS 动画 Token

```css
.plan-streaming-cursor::after {
  content: '';
  display: inline-block;
  width: 2px;
  height: 1em;
  background: var(--plan-tint, #0D9488);
  animation: blink 0.8s ease-in-out infinite;
  margin-left: 2px;
  vertical-align: text-bottom;
}

.plan-new-line {
  animation: fadeSlideIn 0.15s ease-out;
}

@keyframes blink {
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
}

@keyframes fadeSlideIn {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}
```

### Plan 色 Token

```css
:root {
  --plan-tint: #0D9488;
  --plan-tint-soft: rgba(13, 148, 136, 0.08);
  --plan-tint-border: rgba(13, 148, 136, 0.3);
}
```

### 竞品对标

| 维度 | Codex TUI | Claude Code CLI | XiaoLin GUI（目标） |
|------|-----------|-----------------|---------------------|
| 流式渲染 | PlanDelta + PlanStreamController + commit 动画 | 无（plan 走工具） | PlanDelta + 按行 commit + CSS 动画 |
| Plan 展示位置 | 内联在 chat 流 | 分层：hint → `/plan` → Exit 对话框 | Side panel（独立空间） |
| Markdown | 有（全量重渲染） | 有（StreamingMarkdown） | 有（react-markdown + 按行 commit） |
| 视觉反馈 | commit animation（行滑入） | 无 | 闪烁光标 + fadeSlideIn |
| 色系 | `proposed_plan_style()` | `planMode` theme token | `--plan-tint` CSS custom property |
