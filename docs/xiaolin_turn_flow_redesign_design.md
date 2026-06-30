# XiaoLin Turn Flow 信息组织重构设计文档

> 目标：将聊天区改造成接近 Codex App 的 Turn Flow。一个用户请求下，按顺序展示公开工作说明、工具活动、审批/异常与最终答复；实时、历史回放、重连恢复由同一条 canonical timeline 生成一致的 TurnDisplayNode[]。
>
> 本文以仓库 `linzetai/XiaoLin` 当前 main 分支为基线：Timeline 已存在，但 MessageStream、streamSegments 与 TimelineTranscript 仍并存，需要收敛。

---

## 1. 目标、边界与原则

### 1.1 目标

1. **Turn 是第一组织单元**

```text
用户问题
  公开工作说明
  已运行 N 条命令 / 正在运行某条命令
  审批、失败、取消等显著状态
  最终答复（Markdown，视觉权重最高）
```

2. **Canonical Timeline 是正式 transcript 的唯一事实来源**
   - authoritative WebSocket timeline event；
   - history timeline event；
   - reconnect / gap-fill 补包；
   - 以上全部进入同一 reducer，生成同一 `TurnDisplayNode[]`。

3. **不暴露模型原始 CoT**
   - 原始 provider reasoning 永远是 private；
   - 可见的“我正在检查构建配置”等内容必须由独立、安全的 activity narration producer 生成；
   - private reasoning 不进入 DB、WS、history、reconnect、display-node API。

4. **仅一个 virtualizer owner**
   - `TimelineTranscript` 是聊天正文唯一的 virtualizer、滚动锚点和 at-bottom owner；
   - `MessageStream` 只保留壳层、输入区、工具栏和对 transcript controller 的调用。

5. **旧会话可读、但不伪造时序**
   - legacy history / legacy live 协议可转换为本地 synthetic timeline；
   - 旧 reasoning 原文一律隐藏；
   - 缺少精确分段信息时保留最终答复和工具摘要，并明确标记“步骤顺序可能不准确”。

### 1.2 非目标

- 不恢复或公开旧 reasoning 内容。
- 不从无法逆推的数据中伪造“文本 → 工具 → 文本”的精确顺序。
- 不持久化实时终端片段输出。
- 本期不实现 timeline 的最新窗口 / before cursor；先采用**全量分页加载 authoritative timeline**确保顺序正确，窗口化单独立项。

---

## 2. 当前代码基线与问题

### 2.1 已有 Timeline 基础

`crates/xiaolin-app/src/lib/timeline/types.ts` 已将 Timeline 定义为 UI-visible transcript 的 canonical 模型，且注释表明 live WS 和 history replay 应通过同一 reducer；当前 `TIMELINE_SCHEMA_VERSION = 1`。

现有事件已覆盖：

```text
turn_started / user_message_created
assistant_text_delta / snapshot
reasoning_delta / snapshot
tool_call_started / progress / finished
approval_requested / resolved
iteration_boundary / assistant_message_finalized / turn_finished
compact_boundary / system_notice
```

现有 `ToolCallProgressPayload` 已有 `partial_output?: string`，`ApprovalResolvedPayload.decision` 仍是 `string`。

### 2.2 当前 presentation 只有一个全局 process summary

`crates/xiaolin-app/src/lib/timeline/presentation.ts` 当前将 reasoning、tool、approval、system notice 全部装入一个 `process_summary`。完成态中后续 process node 仍追加到同一个 summary。

因此如下真实顺序：

```text
过程 A → 工具 → 最终答复 A → 过程 B → 工具 → 最终答复 B
```

可能被错误展示为：

```text
已处理
  过程 A / 工具 / 过程 B / 工具
最终答复 A
最终答复 B
```

需要改为多个 `ProcessInterval`，final answer 必须切断 interval。

### 2.3 当前 AssistantResponseBlock 只做工具聚合

`AssistantResponseBlock.tsx` 会按 tool family 将工具聚合成 `AssistantActivityGroup`，而 completed summary 展开后还会嵌套同样的 group。

问题：

- running tool 与 completed tool 未强制分离；
- failed/cancelled tool、pending approval 等 attention node 未在聚合前剥离；
- completed process 使用 whole-turn `elapsed_ms`，无法表示多个 interval；
- “展开后至少可看见每个工具标题”的契约不稳定。

### 2.4 当前有两个 virtualizer

- `TimelineTranscript.tsx` 已经按 TurnGroup 使用 `useVirtualizer`；
- `MessageStream.tsx` 同时对 legacy `stream` / streaming placeholder 使用另一套 virtualizer，并负责 scroll、FAB、未读数、历史加载和跳转。

`TimelineTranscript` 当前构建 turn groups 时传入 `events: []`，没有消费完整 timeline state；`MessageStream` 仍把 legacy stream 当作主要滚动数据。

### 2.5 当前旧实时协议直接写 streamSegments

`useMessageStreamChat.ts` 直接处理：

```text
content_delta / reasoning_delta / tool_executing / tool_progress / tool_result / turn_end
```

并维护 `streamSegments`、`streamAccRef`、`streamStore`。移除 legacy renderer 后，这些事件必须经 adapter 进入 Timeline，不能继续作为聊天正文显示源。

### 2.6 详细差距清单

基于 main 分支实际代码的全面审查，共发现 **10 个架构层面差距**和 **7 个协议层面差距**。详见 [§15. 当前代码基线详细差距分析](#15-当前代码基线详细差距分析)。

---

## 3. 总体架构：Canonical Timeline + Ephemeral Overlay

```text
                ┌────────────────────────────────────┐
                │ Canonical Timeline                  │
                │ durable / ordered / replayable      │
                │ server-issued sequence              │
                └────────────────────────────────────┘
                           │
                    reduceTimelineEvents
                           │
                    TurnDisplayNode[]
                           │
                    selectTurnGroups
                           │
                     TimelineTranscript
                           │
                           ▼
              正式 transcript：实时、历史、重连一致

                ┌────────────────────────────────────┐
                │ Ephemeral UI Overlay                │
                ├────────────────────────────────────┤
                │ optimistic user bubble              │
                │ tool_output_patch (partial output)  │
                └────────────────────────────────────┘
                           │
                           ▼
              仅实时叠加；不参与 canonical sequence
```

### 3.1 不把 origin 放进 canonical TurnTimelineEvent

不要将 `authoritative | legacy | provisional` 作为 Rust `TurnTimelineEvent` 的必填字段：

- `legacy` 和 `provisional` 是客户端展示/兼容概念，不是服务端事实；
- 旧 DB 与 history API 没有这个字段；
- provisional event 混入 canonical reducer 会污染 seq、turnIndex、nodeIdIndex 和 source trace；
- source 应由前端 store envelope 管理。

推荐模型：

```ts
type TimelineSource =
  | "none"
  | "probing"
  | "authoritative_pending_snapshot"
  | "authoritative"
  | "legacy";

interface ProvisionalUserMessage {
  clientMessageId: string;
  localTurnId: string;
  content: string;
  attachments?: string[];
  createdAtMs: number;
  status: "sending" | "failed";
}

interface ToolOutputPatch {
  callId: string;
  content: string;
  truncated: boolean;
  updatedAtMs: number;
}

interface SessionTimelineRecord {
  source: TimelineSource;
  canonical: TimelineState;
  meta: TimelineMeta;
  optimisticUsers: Record<string, ProvisionalUserMessage>;
  toolOutputPatches: Record<string, ToolOutputPatch>;
}
```

- authoritative event：服务端 canonical event；
- legacy synthetic event：仅 legacy source 下进入当前客户端 canonical reducer；
- provisional user：只属于 overlay；
- partial output：只属于 overlay。

---

## 4. 协议与安全边界

### 4.1 Schema version

`TIMELINE_SCHEMA_VERSION`：`1 → 2`。

所有新增协议字段必须 optional；旧 schema 仍可 deserialize。

### 4.2 ReasoningVisibility

Rust 与 TS 的 reasoning delta/snapshot 新增：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningVisibility {
    Public,
    Private,
}

pub struct ReasoningDeltaPayload {
    pub node_id: String,
    pub delta: String,
    pub offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ReasoningVisibility>,
}
```

```ts
export type ReasoningVisibility = "public" | "private";

export interface ReasoningDeltaPayload {
  node_id: string;
  delta: string;
  offset?: number;
  visibility?: ReasoningVisibility;
}
```

安全规则：

| visibility | DB / History / Reconnect | WebSocket | 前端 |
|---|---:|---:|---:|
| `private` | 不写入 | 不发送 | 不渲染 |
| `public` | 正常 | 正常 | 公开 activity narration |
| 缺失 | 不应作为新公开内容 | 不展示 | 不渲染 |

**private reasoning 必须在写入 SessionStore 前过滤，而不是只在 WS emit 时过滤。**

### 4.3 Activity narration producer

public narration 不是 provider CoT 的标签版本，而是独立 producer 的输出。

```text
provider raw reasoning
  → 永远 private，丢弃于可信后端边界

tool name / target / state / result summary
  → activity narration producer
  → reasoning(visibility=public) 或 assistant_text(text_role=activity)
```

可见文本要短、可验证、面向用户：

```text
正在检查构建配置
已读取 3 个相关文件，接下来验证打包脚本
构建已完成，正在检查错误输出
```

### 4.4 AssistantTextRole

在 assistant text delta/snapshot 中新增：

```ts
export type AssistantTextRole = "activity" | "final";

export interface AssistantTextDeltaPayload {
  node_id: string;
  delta: string;
  offset?: number;
  text_role?: AssistantTextRole;
}
```

| role | 含义 | completed 模式 |
|---|---|---|
| `activity` | 公开过程说明 | 可归入 ProcessInterval |
| `final` / 缺失 | 最终正式答复 | 始终可见，切断 interval |

### 4.5 client_message_id

不要放在 `TurnTimelineEvent` 顶层；新增到 `UserMessageCreatedPayload`：

```ts
export interface UserMessageCreatedPayload {
  message_id?: string;
  client_message_id?: string;
  content: string;
  attachments?: string[];
}
```

发送时客户端生成 UUID；服务端创建 authoritative `user_message_created` 时原样回传；前端据此移除 optimistic overlay。

### 4.6 partial output：独立 realtime patch

不把实时终端片段作为 durable timeline payload。

```ts
// durable timeline event
{
  event_type: "tool_call_progress",
  payload_json: { call_id, message, progress }
}

// live-only patch
{
  session_id,
  call_id,
  partial_output,
  truncated,
  updated_at_ms
}
```

后端：

```text
stdout/stderr
  → gateway memory tail buffer（最后 8KB）
  → 500ms 节流推送 tool_output_patch
  → tool_call_finished 时释放缓冲
```

前端：

```text
tool_output_patch → toolOutputPatches[callId]
ToolStepView running 时显示
工具完成后删除 patch，完成态使用 output_preview/output_detail
```

### 4.7 Approval decision 维持 string wire format

不要把 `decision: String` 改成 enum wire type。projection 层 normalize：

```ts
type ApprovalDecision =
  | "allow_once"
  | "allow_always"
  | "deny"
  | "abort"
  | "other";

function normalizeApprovalDecision(raw?: string): ApprovalDecision {
  switch (raw) {
    case "allow_once":
    case "approved":
      return "allow_once";
    case "allow_always":
    case "approved_for_session":
      return "allow_always";
    case "deny":
    case "denied":
      return "deny";
    case "abort":
    case "aborted":
      return "abort";
    default:
      return "other";
  }
}
```

`other` 显示中性 attention 文案“审批状态未知”，不能冒充“已拒绝”。

---

## 5. Canonical sequence、乱序、gap 与 reducer 完整性

### 5.1 对外 seq 分配时机

```text
agent/provider 原始事件
  → private reasoning 过滤
  → activity narration 生成
  → tool progress coalesce / throttle
  → SessionStore 为 durable event 分配 canonical seq
  → DB 持久化
  → WS push
```

因此 private reasoning 和被合并掉的 progress 不会制造对外 seq 空洞。

### 5.2 node_id 不可变性

同一个 `node_id` 首次确定：

```text
kind
turn_id
visibility（reasoning）
text_role（assistant_text）
```

冲突时：

```text
拒绝整条 event
记录 protocol violation
不追加 content
不更新 source trace
```

不能只 warning 后继续 append，否则 private delta 可能被拼进 public node。

### 5.3 nodeIdIndex 进入可重放 TimelineState

`nodeIdIndex` 不可只存在于 Zustand `_meta`；历史加载、replace、full replay 后会失效。

```ts
interface TimelineState {
  events: TurnTimelineEvent[];
  nodes: TurnDisplayNode[];
  maxSeq: number;
  turnIndex: Record<string, string[]>;
  eventTraces: Record<string, SourceEventTrace>;
  nodeIdIndex: Record<string, {
    kind: TurnDisplayNodeKind;
    turnId: string;
    visibility?: ReasoningVisibility;
    textRole?: AssistantTextRole;
  }>;
}
```

### 5.4 known / applied / pending 分层

不能把存在 gap 的 future event 提前拿去 reducer materialize。

```ts
interface TimelineMeta {
  knownById: Map<string, TurnTimelineEvent>;
  appliedEvents: TurnTimelineEvent[];      // 仅连续前缀
  pendingBySeq: Map<number, TurnTimelineEvent>;
  lastContiguousSeq: number;
  gapFillInFlight: boolean;
}
```

处理思路：

```text
收到 seq=15，当前连续到 10
→ 15 保存 pending，不展示
→ 发起 afterSeq=10 的补包
→ 得到 11..14
→ 重新计算连续前缀
→ 11..15 一起 reducer materialize
```

要求：

- 同 seq、不同 event id：protocol violation，不能覆盖；
- gap fill cursor 必须使用 `lastContiguousSeq`，不能用最大已见 seq；
- 全量 replay 时必须把 committed + pending + incoming 都放入 known 集合，但仅连续前缀进入 reducer；
- `turn_finished` 后的 late `tool_call_finished` 用真实 success/failed 覆盖临时 cancelled。

---

## 6. Source 状态机、历史加载与 optimistic user

### 6.1 Source 状态

```ts
type TimelineSource =
  | "none"
  | "probing"
  | "authoritative_pending_snapshot"
  | "authoritative"
  | "legacy";
```

```text
none → probing
probing + snapshot 有数据 → authoritative
probing + 收到任何 authoritative WS event → authoritative_pending_snapshot
probing + snapshot 为空且无 authoritative event → legacy
authoritative_pending_snapshot + snapshot 完整 → authoritative
legacy + 后续 snapshot 有数据 → authoritative_pending_snapshot → authoritative（replace）
authoritative 永不降级 legacy
```

### 6.2 Phase 1 使用全量 pagination

本期明确采用全量 authoritative timeline 加载，不与 window model 混用：

```ts
async function fetchFullAuthoritativeTimeline(sessionId: string) {
  const result: TurnTimelineEvent[] = [];
  let afterSeq = 0;
  const limit = 500;

  while (true) {
    const page = await api.getSessionTimeline(sessionId, afterSeq, limit);
    if (!page?.events?.length) break;
    result.push(...page.events);
    if (page.events.length < limit) break;
    afterSeq = page.events[page.events.length - 1].seq;
  }
  return result;
}
```

后续若做窗口化，必须先新增 `before_seq` 或 opaque cursor API；不能用 `afterSeq=0` 伪装成“加载最新窗口”。

### 6.3 probe buffer 与 replace

`probeBuffer` 仅存 probing 期间的 authoritative WS events，不存：

```text
provisional overlay
legacy synthetic event
tool output patch
```

完成 probe：

```ts
const merged = dedupeById([
  ...snapshotEvents,
  ...probeBuffer,
]).sort((a, b) => a.seq - b.seq);

replaceCanonicalTimeline(sessionId, merged);
```

如果 probing 阶段已收到 authoritative WS event，但 snapshot 暂时失败：

```text
source = authoritative_pending_snapshot
按已收到连续前缀继续渲染
后台重试 snapshot
绝不降级到 legacy
```

### 6.4 optimistic user overlay

```ts
const clientMessageId = crypto.randomUUID();

optimisticUsers[clientMessageId] = {
  clientMessageId,
  localTurnId: `optimistic-turn-${clientMessageId}`,
  content,
  attachments,
  createdAtMs: Date.now(),
  status: "sending",
};

transport.sendMessage({ content, client_message_id: clientMessageId });
```

UI selector 将 optimistic user 作为尾部临时 TurnGroup 显示。收到 authoritative `user_message_created` 且 payload 的 `client_message_id` 匹配后：

```text
同一 store transaction：删除 overlay → ingest authoritative event
```

不能让 provisional event 进入 canonical `state.events`，也不能给它伪造 canonical seq。

---

## 7. Legacy 兼容

### 7.1 原则

- legacy source 由 store envelope 管理，不用 `syn-` / `evt-` 前缀推断来源；
- synthetic events 仅本地渲染，不回写服务端；
- legacy reasoning 一律不展示；
- 无法精确恢复顺序时明确降级。

### 7.2 历史 migration

新增：`lib/timeline/legacy-migration.ts`。

Turn 归属：

```text
user(id=10) → legacy-turn-10
assistant(id=11) / assistant(id=12) → legacy-turn-10
user(id=13) → legacy-turn-13
```

每个 turn 必须完整生成：

```text
turn_started
user_message_created
[tool summary / final assistant text / notice]
turn_finished
```

稳定 ID：

```ts
eventId = `legacy:${sessionId}:${sourceMessageId}:${segmentIndex}`;
```

稳定 order key：

```ts
orderKey = stableSourceMessageOrder * 10_000 + segmentIndex;
```

`stableSourceMessageOrder` 必须来自稳定历史字段（DB id 或 `(createdAt, id)` rank），不能依赖本次分页数组下标。

### 7.3 segmentOrder 的限制

若旧记录只有：

```text
reasoningContent
toolCallsJson
content
```

则无法表示多个独立 text segment。即使 `segmentOrder` 中出现多个 `text`，也不得把同一 `msg.content` 重复多次。

降级展示：

```text
工具摘要（可提取时）
最终答复（只出现一次）
notice：旧格式，步骤顺序可能不准确
```

### 7.4 Legacy live adapter

新增：`lib/timeline/legacy-live-adapter.ts`。

```ts
interface LegacyLiveAdapter {
  ingest(event: ChatStreamEvent): TurnTimelineEvent[];
  flush(): TurnTimelineEvent[];
  createUserMessage(content: string, attachments?: string[]): TurnTimelineEvent[];
}
```

旧事件映射：

| 旧 WS | synthetic timeline 输出 |
|---|---|
| `turn_start` | `turn_started` |
| `content_delta` | assistant text delta / buffered snapshot |
| `reasoning_delta` | reasoning delta，visibility 缺失，前端隐藏 |
| `tool_executing` | flush text + `tool_call_started` |
| `tool_progress` | `tool_call_progress`，输出文本走 patch overlay |
| `tool_result` | flush text + `tool_call_finished` |
| `approval_required` | `approval_requested` |
| `approval_resolved` | `approval_resolved` |
| `turn_end` | flush buffers + `turn_finished` |
| `error` | error `system_notice` |

legacy 与 authoritative 绝不 merge。authoritative snapshot 到达时 replace legacy state；optimistic overlay 保持独立。

---

## 8. Presentation：多 ProcessInterval、attention 与三态 UI

### 8.1 类型

`ProcessInterval` 只在 `presentation.ts` 中存在：

```ts
export type AssistantPresentationItem =
  | { kind: "visible"; node: AssistantVisibleNode }
  | { kind: "attention"; node: AssistantVisibleNode }
  | { kind: "process_interval"; interval: ProcessInterval };

export interface ProcessInterval {
  id: string;
  nodes: AssistantProcessNode[];
  startMs: number;
  endMs: number;
  durationMs: number;
}
```

### 8.2 attention item

必须独立展示：

```text
pending approval
deny / abort / unknown approval
failed / cancelled tool
tool group 内任一步失败或取消
warning / error system notice
abnormal turn status
```

attention 必须在工具聚合之前剥离。

### 8.3 分段算法

前提：工具、审批、notice 边界后的 public activity 必须使用新的 node_id；前端不做 `source_trace.min_seq` 近似恢复。

```ts
for (const node of nodes) {
  if (node.kind === "iteration_boundary" && !showDiagnostics) continue;
  if (isNormalCompletionStatus(node)) continue;

  if (node.kind === "reasoning" && node.visibility !== "public") {
    continue;
  }

  if (isAttentionItem(node)) {
    flushInterval();
    items.push({ kind: "attention", node });
    continue;
  }

  if (node.kind === "assistant_text") {
    if (node.text_role === "activity") {
      appendToInterval(node);
    } else {
      flushInterval();
      items.push({ kind: "visible", node });
    }
    continue;
  }

  if (isFoldableProcessNode(node)) {
    appendToInterval(node);
    continue;
  }

  flushInterval();
  items.push({ kind: "visible", node });
}
flushInterval();
```

interval 时长：

```ts
durationMs = Math.max(0, last.updated_at_ms - first.created_at_ms);
```

不得复用整回合 `elapsed_ms`。

### 8.4 Live UI

```text
用户问题
  正在检查构建配置…                       ← public narration
  ✅ 已运行 3 条命令 [展开]               ← completed batch
  ⏳ 正在运行：pnpm test                  ← running tool 独立行
     读取测试结果…
     <partial output patch>
  接下来验证打包脚本…                     ← 下一段 narration
```

- public narration、`assistant_text(activity)`：正文，默认可见；
- completed non-attention tools：可按 family 聚合；
- running tool：绝不进入 completed batch；
- attention：独立显示；
- private / legacy reasoning：不进 DOM。

### 8.5 Completed UI

```text
用户问题

已处理 12 秒 >
  公开说明
  已运行 3 条命令
  公开说明

最终答复（Markdown，默认可见）
```

- 每个 interval 单独折叠；
- 展开后保持内部 node 顺序；
- 工具标题至少可见；参数和大输出二级展开；
- final text 永远在 interval 外。

### 8.6 Abnormal UI

```text
用户问题

已处理 5 秒 >

⚠️ 工具执行未返回结果，回合已结束
⚠️ 检测到工具循环，已停止
```

attention 永远不可被“已处理”折叠吞掉。

---

## 9. Virtualizer、滚动与组件职责

### 9.1 单 owner

```text
MessageStream
  └─ scroll container
      └─ TimelineTranscript
          └─ 唯一 useVirtualizer
              └─ TurnBlock
                  └─ AssistantResponseBlock
```

`MessageStream` 删除：

```text
displayData
MessageRendererRow
legacy stream list virtualizer
旧 getItemKey / measureElement
旧 virtualizer 驱动的 FAB / unread / scroll 逻辑
```

### 9.2 Scroll controller

```ts
export interface TimelineScrollHandle {
  scrollToEnd(opts?: { behavior?: "auto" | "smooth" }): void;
  scrollToTurn(turnId: string, opts?: {
    align?: "start" | "center";
    behavior?: "auto" | "smooth";
  }): void;
  isAtEnd(): boolean;
  subscribeAtBottom(listener: (atBottom: boolean) => void): () => void;
}
```

`TimelineTranscript` 通过 `forwardRef` + `useImperativeHandle` 暴露，并成为唯一 owner：

```text
virtualizer
measureElement
scroll anchoring
at-bottom 判断
live auto-scroll
turn locator
```

`MessageStream` 只订阅 at-bottom，驱动 FAB、unread 与完成提示。

### 9.3 selector

`TimelineTranscript` 不应只拿 nodes 并传空 events；应通过统一 selector 得到 render model：

```ts
const model = selectTranscriptRenderModel({
  canonicalState,
  optimisticUsers,
  toolOutputPatches,
});
```

- canonical groups 由 canonical nodes 构建；
- optimistic users 作为尾部临时 group；
- tool output patch 通过 `callId` 传递给 `ToolStepView`，不修改 canonical node。

---

## 10. 文件改动清单

### 10.1 Rust / server

#### `crates/xiaolin-protocol/src/timeline.rs`（1625 行 → 约 1800 行）

| 改动 | 详情 |
|---|---|
| `TIMELINE_SCHEMA_VERSION: 1 → 2` | 常量升级 |
| 新增 `ReasoningVisibility` enum | `Public`, `Private`，`#[non_exhaustive]` |
| `ReasoningDeltaPayload.visibility` | `Option<ReasoningVisibility>`，`skip_serializing_if` |
| `ReasoningSnapshotPayload.visibility` | 同上 |
| 新增 `AssistantTextRole` enum | `Activity`, `Final`，`#[non_exhaustive]` |
| `AssistantTextDeltaPayload.text_role` | `Option<AssistantTextRole>`，`skip_serializing_if` |
| `AssistantTextSnapshotPayload.text_role` | 同上 |
| `UserMessageCreatedPayload.client_message_id` | `Option<String>`，`skip_serializing_if` |
| `TurnFinishedPayload` 新增字段 | `repeated_force_stops: Option<u32>`, `repeated_warns: Option<u32>`, `no_progress_count: Option<u32>` |
| 不新增字段 | `origin` / `source` 不入 canonical event |

#### `crates/xiaolin-session/src/timeline_store.rs`（1951 行 → 约 2100 行）

| 改动 | 详情 |
|---|---|
| `append()` 前过滤 | `event_type == "reasoning_delta" \|\| "reasoning_snapshot"` 且 `visibility == "private"` → 拒绝写入，返回 `Ok(None)` |
| `materialize_events_to_nodes()` | 传播 `visibility`、`text_role` 到节点；`private` reasoning 节点不生成 |
| `materialize_events_to_nodes()` | `turn_finished` 时 running tool → `Cancelled`；late `tool_call_finished` 覆盖 |
| 新增 `query_events_by_seq_range()` | 支持 `after_seq` + `limit` 的 event 级别查询（现有 API 已支持，确认） |
| 黄金测试 | `#[cfg(test)]` 中加载 `test-vectors/reducer/*.json`，assert 输出匹配 |

#### `crates/xiaolin-gateway/src/ws/timeline_emit.rs`（949 行 → 约 1100 行）

| 改动 | 详情 |
|---|---|
| `map_agent_event_to_timeline()` | reasoning 事件检查 visibility；`private` → 返回 `None`（不生成 candidate） |
| 新增 `ToolOutputPatchEmitter` | 500ms throttle、8KB tail buffer、`tool_call_finished` 时释放 |
| `build_tool_call_finished()` | 计算 `duration_ms = finished_at_ms - started_at_ms` |
| 新增 `NarrationProducer` 接入点 | 接收 `(tool_name, target, state, result_summary)` → 调用 narration producer → 生成 `reasoning(visibility=public)` 或 `assistant_text(text_role=activity)` |

#### `crates/xiaolin-gateway/src/ws/chat.rs`（改动约 50 行）

| 改动 | 详情 |
|---|---|
| `send_timeline_event()` | 新增 `private` visibility 防御性检查（双重保险） |
| `UserMessageCreated` 发送 | 从请求中提取 `client_message_id`，回传到 payload |

#### 新增：`crates/xiaolin-gateway/src/ws/narration_producer.rs`（约 200 行）

| 功能 | 详情 |
|---|---|
| `generate_activity_narration(tool_name, target, state, result_summary) -> String` | 模板驱动，生成短中文叙述 |
| 模板示例 | `"正在检查构建配置"`, `"已读取 N 个相关文件"`, `"构建已完成"` |
| 安全约束 | 不拼接原始 tool output；不暴露文件绝对路径；不包含用户数据 |

### 10.2 前端 Timeline

#### `crates/xiaolin-app/src/lib/timeline/types.ts`（约 350 行 → 约 480 行）

| 改动 | 详情 |
|---|---|
| `TIMELINE_SCHEMA_VERSION: 1 → 2` | 常量升级 |
| 新增 `ReasoningVisibility` | `"public" \| "private"` |
| 新增 `AssistantTextRole` | `"activity" \| "final"` |
| `ReasoningDeltaPayload.visibility?` | optional |
| `ReasoningSnapshotPayload.visibility?` | optional |
| `AssistantTextDeltaPayload.text_role?` | optional |
| `AssistantTextSnapshotPayload.text_role?` | optional |
| `UserMessageCreatedPayload.client_message_id?` | optional |
| `TurnFinishedPayload` 新增字段 | `repeated_force_stops?`, `repeated_warns?`, `no_progress_count?` |
| 新增 `ToolOutputPatch` | `{ callId, content, truncated, updatedAtMs }` |
| 新增 `ProvisionalUserMessage` | `{ clientMessageId, localTurnId, content, attachments?, createdAtMs, status }` |
| 新增 `TimelineSource` | `"none" \| "probing" \| "authoritative_pending_snapshot" \| "authoritative" \| "legacy"` |
| 新增 `SessionTimelineRecord` | `{ source, canonical, meta, optimisticUsers, toolOutputPatches }` |
| `TimelineState` 新增字段 | `nodeIdIndex: Record<string, NodeIdentityInfo>` |
| `TurnDisplayNode` 新增 variant | `tool_group` (presentation-only) |

#### `crates/xiaolin-app/src/lib/timeline/reducer.ts`（约 750 行 → 约 950 行）

| 改动 | 详情 |
|---|---|
| `reduceTimelineEvent()` | reasoning delta/snapshot 检查 `visibility`：`"private"` → 不创建/更新 node，返回原 state |
| 同上 | assistant text delta/snapshot 传播 `text_role` 到 node |
| 同上 | `turn_finished`：同 turn 所有 `running`/`pending` node → `cancelled`（tool/approval/reasoning） |
| 同上 | late `tool_call_finished`：找到同 `call_id` 的 cancelled node → 用真实 success/failed 覆盖 |
| 同上 | node identity invariant：新增 node 时检查 `nodeIdIndex`；冲突（kind/turn_id/visibility/text_role 不一致）→ 拒绝整条 event，log protocol violation |
| `reduceTimelineEvents()` | 排序后批量应用；构建 `nodeIdIndex` |
| 新增 `materializeNodes()` | 纯函数，输入 `TurnTimelineEvent[]`，输出 `TurnDisplayNode[]`（已有，确认签名） |
| `TurnStatusNode.diagnosis` | 透传 `repeated_force_stops`、`repeated_warns`、`no_progress_count` |

#### `crates/xiaolin-app/src/lib/timeline/presentation.ts`（约 200 行 → 约 400 行）

| 改动 | 详情 |
|---|---|
| 删除 `process_summary` presentation item | 替换为 `process_interval` |
| 新增 `ProcessInterval` | `{ kind: "process_interval", id, nodes, startMs, endMs, durationMs }` |
| 新增 `attention` item | `{ kind: "attention", node }` |
| `AssistantPresentationItem` | `visible \| attention \| process_interval` |
| 新增 `buildPresentationItems()` | 分段算法：reasoning(visibility!=public) 跳过；attention 剥离；activity text 归入 interval；final text 切断 interval |
| `selectAssistantTurnPresentation()` | 重构使用 `buildPresentationItems`；支持多 interval |
| 新增 `isAttentionItem()` | `failed/cancelled tool`, `pending/deny/abort/other approval`, `warning/error notice`, `abnormal turn status` |
| 新增 `isFoldableProcessNode()` | `reasoning(public)`, `tool_step(completed)`, `approval(allow_once/allow_always)`, `system_notice(info)` |

#### `crates/xiaolin-app/src/lib/timeline/selectors.ts`（约 120 行 → 约 200 行）

| 改动 | 详情 |
|---|---|
| 新增 `selectTranscriptRenderModel()` | 输入 `{ canonicalState, optimisticUsers, toolOutputPatches }`，输出 `TurnGroup[]`（含 optimistic tail group） |
| `selectTurnGroups()` | 适配 `nodeIdIndex`，支持 `ProcessInterval` 粒度的分组 |
| 新增 `selectIsAtEnd()` | 基于 `TimelineState.maxSeq` 和 last visible seq 判断 |
| 新增 `selectUnreadCount()` | 基于 `lastSeenSeq` 和 `maxSeq` 差值 |

#### `crates/xiaolin-app/src/lib/timeline/reconnect.ts`（约 120 行 → 约 180 行）

| 改动 | 详情 |
|---|---|
| `recoverTimelineAfterReconnect()` | 改用 `lastContiguousSeq`（非 `maxSeq`）作为 gap fill cursor |
| 同上 | gap fill 返回 events → `ingestEvents` → 自动 recalculate contiguous |
| 同上 | 超时/失败 → `replaceCanonicalTimeline` 全量重载 |
| `initTimelineForSession()` | 全量分页 probe → 设置 source → `replaceCanonicalTimeline` |

#### `crates/xiaolin-app/src/lib/timeline/fixtures.ts`（约 300 行 → 约 450 行）

| 改动 | 详情 |
|---|---|
| 所有 payload 工厂 | 适配新增 optional 字段（`visibility`, `text_role`, `client_message_id`） |
| 新增 `privateReasoningTurnFixture()` | 含 private reasoning 的完整 turn |
| 新增 `multiIntervalTurnFixture()` | activity → final → activity → final |
| 新增 `attentionMixTurnFixture()` | failed tool + denied approval + warning notice |
| 新增 `legacySyntheticTurnFixture()` | 模拟 legacy adapter 输出 |

#### 新增：`crates/xiaolin-app/src/lib/timeline/legacy-migration.ts`（约 300 行）

| 功能 | 详情 |
|---|---|
| `migrateLegacySessionToTimeline(messages: LegacyMessage[]): TurnTimelineEvent[]` | 历史消息 → synthetic timeline events |
| Turn 归属算法 | 连续 user → assistant(s) → 下一个 user 前为一 turn |
| 稳定 ID | `legacy:{sessionId}:{sourceMessageId}:{segmentIndex}` |
| 稳定 order key | `stableSourceMessageOrder * 10_000 + segmentIndex` |
| 降级处理 | segmentOrder 缺失 → 工具摘要 + 最终答复 + notice |

#### 新增：`crates/xiaolin-app/src/lib/timeline/legacy-live-adapter.ts`（约 350 行）

| 功能 | 详情 |
|---|---|
| `createLegacyLiveAdapter(): LegacyLiveAdapter` | 工厂函数 |
| `ingest(event: ChatStreamEvent): TurnTimelineEvent[]` | 旧 WS 事件 → synthetic timeline events（可能返回空数组） |
| `flush(): TurnTimelineEvent[]` | 刷新 text/reasoning buffer，生成 snapshot |
| `createUserMessage(content, attachments?): TurnTimelineEvent[]` | 生成 `turn_started` + `user_message_created` |
| 内部 buffer | `textBuffer`, `reasoningBuffer`, `currentToolCallId`, `currentTurnId` |
| reasoning 处理 | visibility 缺失 → 不生成 reasoning node（安全默认） |

### 10.3 前端 Store

#### `crates/xiaolin-app/src/lib/stores/timeline-store.ts`（166 行 → 约 500 行）

| 改动 | 详情 |
|---|---|
| `states: Record<string, TimelineState>` | 替换为 `records: Record<string, SessionTimelineRecord>` |
| `addEvent` | 替换为 `ingestEvent`（gap-aware） |
| `loadEvents` | 替换为 `ingestEvents` + `replaceCanonicalTimeline` |
| `loadNodes` | 删除（由 `replaceCanonicalTimeline` 替代） |
| 新增 actions | `setSource`, `startProbe`, `completeProbe`, `recordPending`, `fillGap`, `recalculateContiguous` |
| 新增 overlay actions | `upsertOptimisticUser`, `removeOptimisticUser`, `upsertToolOutputPatch`, `removeToolOutputPatch` |
| `initSession` | 初始化 `SessionTimelineRecord` 默认值 |
| Map 字段 | `knownById`、`pendingBySeq` 使用 `Map`（不可序列化，runtime-only）；store 持久化时排除 |

### 10.4 前端 UI

#### `crates/xiaolin-app/src/components/message-stream/MessageStream.tsx`

| 改动 | 详情 |
|---|---|
| 删除 `MessageRendererRow` 导入和使用 | Phase 5 |
| 删除旧 `useVirtualizer`（stream list） | Phase 5 |
| 删除 `displayData` / `getItemKey` / `measureElement` | Phase 5 |
| 删除 `stream.length` 驱动的 isEmpty / loadChatStream | 替换为 `timelineNodes.length` |
| FAB / unread / scroll 逻辑 | 改为订阅 `TimelineTranscript` 的 `TimelineScrollHandle` |
| 保留 | 输入区、工具栏、MentionInput、PlanApprovalCard、StreamFooter、StickyContextBar |
| 新增 `scrollControllerRef = useRef<TimelineScrollHandle>(null)` | 传递给 `TimelineTranscript` |

#### `crates/xiaolin-app/src/components/message-stream/TimelineTranscript.tsx`

| 改动 | 详情 |
|---|---|
| `forwardRef` + `useImperativeHandle` | 暴露 `TimelineScrollHandle` |
| `useVirtualizer` | 从 `selectTranscriptRenderModel()` 获取 items（含 optimistic tail） |
| 新增 | `at-bottom` 检测、live auto-scroll、turn locator |
| `measureElement` | 稳定：使用 `node_id` 作为 key |
| 移除 `events: []` 空传参 | 通过 selector 完整消费 timeline state |

#### `crates/xiaolin-app/src/components/message-stream/AssistantResponseBlock.tsx`

| 改动 | 详情 |
|---|---|
| `buildPresentationItems()` | 替换为 `presentation.ts` 的 `buildPresentationItems` |
| `process_summary` → `process_interval` | 支持多个 interval |
| running tool | 从 completed group 中分离，独立渲染 |
| attention | 在工具聚合前剥离，独立渲染（不可被折叠吞掉） |
| tool group | 按 family 聚合 completed non-attention tools |

#### `crates/xiaolin-app/src/components/message-stream/AssistantActivityGroup.tsx`

| 改动 | 详情 |
|---|---|
| 适配 `ProcessInterval` | 接受 `interval: ProcessInterval` 而非 `nodes[] + elapsedMs` |
| 展开/折叠 | 基于 `interval.id` 管理展开状态 |
| 内部顺序 | 保持 interval.nodes 的顺序渲染 |

#### `crates/xiaolin-app/src/components/message-stream/ReasoningBlock.tsx`

| 改动 | 详情 |
|---|---|
| visibility 检查 | `visibility !== "public"` → 不渲染（`return null`） |
| legacy reasoning | visibility 缺失 → 不渲染 |
| public narration | 正文渲染，默认可见 |

#### `crates/xiaolin-app/src/components/message-stream/ToolStepView.tsx`

| 改动 | 详情 |
|---|---|
| running 态 | 读取 `toolOutputPatches[callId]` 显示 partial output |
| completed 态 | 使用 `output_preview` / `output_detail`，不显示 patch |
| patch 清理 | 工具完成后 patch 自动移除 |

#### `crates/xiaolin-app/src/components/message-stream/TurnNodeRenderer.tsx`

| 改动 | 详情 |
|---|---|
| 新增 `process_interval` 渲染 | 委托给 `AssistantActivityGroup` |
| 新增 `attention` 渲染 | 独立 attention 行样式 |
| `TurnStatusNode` | 仅 abnormal 渲染；正常 completed → null |
| `IterationBoundaryNode` | `showDiagnostics` 为 false 时跳过（已有） |

#### `crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts`

| 改动 | 详情 |
|---|---|
| 旧 WS 事件处理 | 不再直接写 `streamSegments`；改为调用 `LegacyLiveAdapter.ingest()` → `timelineStore.ingestEvent()` |
| `streamSegments` / `streamAccRef` / `streamStore` | Phase 5 删除 |
| 新增 `sendMessage()` | 生成 `client_message_id` → `upsertOptimisticUser` → transport.send |
| `client_message_id` reconciliation | 收到 authoritative `user_message_created` 时 `removeOptimisticUser` |

---

## 11. 分阶段实施

### Phase 1：协议、安全与 reducer

1. Rust schema v2：visibility、text role、client message id。
2. SessionStore 写入前过滤 private reasoning。
3. seq 在 filter/coalesce 后分配。
4. 引入安全 activity narration producer。
5. TS reducer 实现 node identity invariant。
6. `turn_finished` 收口 running tool，late result 覆盖 cancelled。

验收：private reasoning 不存在于 DB、WS、history、reconnect、display API；属性冲突 event 不改变 reducer state。

### Phase 2：authoritative source、补包与 overlay

1. TimelineSource 状态机。
2. 全量分页 probe。
3. known/applied/pending 与 `lastContiguousSeq`。
4. gap fill / reconnect。
5. optimistic user overlay 与 client id reconciliation。
6. tool output patch overlay。

验收：probe + WS race 不重复、不丢；gap 不提前显示 future event；provisional 不污染 canonical seq。

### Phase 3：legacy 兼容

1. history migration；
2. live adapter；
3. 每 turn 完整生命周期；
4. 不完整 segmentOrder 的安全降级；
5. legacy → authoritative replace。

### Phase 4：presentation 与 UI

1. 多 ProcessInterval；
2. attention 分离；
3. running/completed tool 分离；
4. public-only ReasoningBlock；
5. ToolStepView patch；
6. TimelineTranscript controller。

### Phase 5：MessageStream 收敛

1. 删除旧 virtualizer/render path；
2. TimelineTranscript 接管 at-bottom、FAB、unread、jump；
3. `streamSegments` 不再承担 transcript 可见内容职责。

---

## 12. 测试矩阵

### Reducer

- public/private/undefined visibility；
- `activity/final` role 不可变；
- node id 的 kind/turn/visibility/role 冲突整条拒绝；
- turn finish 收口 tool；
- late tool result；
- 同 call id 原地 progress 更新；
- schema version 传播。

### Store / 顺序

- private filter / coalesce 不制造 external seq gap；
- seq 10 后收到 15：15 不显示；
- 补到 11..14 后：15 自动连上；
- replay 不丢 pending；
- 同 seq 不同 id protocol violation；
- gap fill 从 lastContiguousSeq 开始；
- probing 收到 authoritative WS 不降级 legacy；
- snapshot + buffer 去重排序；
- provisional 不进 canonical events；
- `client_message_id` 原子 reconciliation。

### Presentation

- public narration → 3 tools → final：1 interval + final；
- activity → final → activity → final：多个 interval；
- private/undefined reasoning 不出现；
- failed/cancelled tool、pending/deny/other approval、warning/error notice、abnormal status 都是 attention；
- interval duration 自身计算；
- running tool 不进入 completed group。

### Legacy

- user 和 assistant 同 turn；
- 每 turn 有 started/finished；
- turn finish 最后；
- stable IDs/order；
- 多个 text token 不重复 content；
- segmentOrder 缺失时保留工具摘要 + notice；
- legacy → authoritative replace 无重复。

### UI / E2E

- live narration、completed batch、running command、partial patch；
- completed 多 interval；
- failed tool / approval 不被折叠；
- history replay 与 live canonical transcript 一致；
- partial output 仅 live 显示，结束后切换正式 output；
- probing 中发送消息立即出现 user bubble；
- scroll away 不拉回，回到底部清 unread；
- 长会话 resize、展开 summary、重连无明显跳动。

---

## 13. 验收标准

### 数据正确性

- canonical seq 不含 private/coalesced 幽灵空洞；
- 同 session 不允许同 seq 不同 event id；
- UI 只 materialize 连续前缀；
- authoritative、legacy、provisional 不互相污染。

### 安全性

- 原始 private reasoning 永不离开可信后端边界；
- public activity 只能来自安全 producer；
- 历史 reasoning 未确认安全时不展示。

### UI 体验

- running command 比 completed tools 醒目；
- final answer 始终位于折叠过程之外；
- error / approval / cancellation 不被“已处理”吞掉；
- 滚动与 unread 只有一个 owner。

### 工程可维护性

- TimelineState 可纯函数重放；
- presentation 是纯 selector；
- overlay 不混入 durable reducer event log；
- legacy adapter/migration 独立可测。

### 代码有效性

- 不出现无用的代码，例如不保留MessageStream
- 不出现死代码，定义了但没人调用

---

## 14. 与当前代码的映射

| 当前文件 | 现职责 | 改造后职责 |
|---|---|---|
| `lib/timeline/types.ts` | protocol mirror，schema v1 | schema v2、payload/node 字段 |
| `lib/timeline/reducer.ts` | event → nodes | 可重放 invariant、tool terminal 收口 |
| `lib/timeline/presentation.ts` | 单 process summary | 多 ProcessInterval + attention |
| `AssistantResponseBlock.tsx` | 工具聚合 / completed summary | running/attention/interval 分层 |
| `TimelineTranscript.tsx` | 一套 turn virtualizer | 唯一 scroll / virtualizer owner |
| `MessageStream.tsx` | legacy list virtualizer、FAB、历史分页 | 控制壳 + transcript ref |
| `useMessageStreamChat.ts` | 旧 WS / streamSegments | source-aware adapter + optimistic send |
| `timeline-store.ts` | session timeline state | source、known/applied/pending、overlay、reconciliation |

---

## 15. 当前代码基线详细差距分析

基于对 main 分支实际代码的全面审查，以下差距必须在实施中解决。

### 15.1 架构层面

| # | 差距 | 位置 | 影响 | 修复方案 |
|---|---|---|---|---|
| **G1** | **双归约器实现** | `reducer.ts` vs `timeline_store.rs::materialize_events_to_nodes()` | Rust 和 TS 各自实现了相同的归约逻辑，但实现细节不同：TS 通过 `findIndex` 不可变替换，Rust 通过 `update_tool_step`/`update_approval` 原地修改。没有编译时保证等价性。 | Phase 1 新增黄金测试：相同事件输入 → 相同节点输出。`normalize.ts` 已有的 `nodesAreEquivalent` 可复用，但需覆盖全部 16 种事件类型的交叉组合。长期考虑从 Rust 生成 JSON 测试矢量，TS 侧消费验证。 |
| **G2** | **旧渲染路径仍存活** | `MessageStream.tsx:988` | `timelineNodes.length > 0 ? <TimelineTranscript /> : <MessageRendererRow />` — 旧 `stream` store 和 `MessageRendererRow` 路径完全可用，与 timeline 路径形成双轨。`StreamSegment` 类型未被标记 deprecated。 | Phase 5：删除旧 virtualizer、`MessageRendererRow` 渲染分支、`stream` store 的显示数据消费。`MessageStream` 仅保留壳层（输入区、工具栏、FAB、scroll 订阅）。 |
| **G3** | **`loadNodes` 不完整状态** | `timeline-store.ts:136-155` | `loadNodes` 设置 `nodes` 但不重置 `events`/`maxSeq`/`turnIndex`，导致 reconnect 后 `TimelineState` 内部不一致（events 为空但 nodes 非空）。 | Phase 2 引入 `replaceCanonicalTimeline` action，原子替换整个 `TimelineState`（events + nodes + maxSeq + turnIndex + eventTraces + nodeIdIndex），不允许部分更新。 |
| **G4** | **Text delta 服务端不合并** | `chat.rs:1122-1170` | 每个 WS text chunk 生成独立 `TurnTimelineEvent` 行，长文本流产生大量小事件。合并只在 materialize 阶段发生。 | 可选优化：在 `build_text_delta` 中增加 200ms 或 256-byte 合并窗口。本期不强制，但需在 `timeline_store.rs` 的 `materialize_events_to_nodes` 中确保合并正确性不受事件数量影响。 |
| **G5** | **显示节点 API 无分页** | `routes/timeline.rs:get_session_display_nodes` | 全量 materialize 整个会话，数百回合时可能昂贵。 | 本期采用全量分页 event 后客户端 reducer materialize。服务端 display-nodes 端点保留用于调试/admin，生产路径统一走 events + 客户端 reducer。 |
| **G6** | **`TurnFinished` 状态行为分歧** | Rust `timeline_store.rs:679` vs TS `reducer.ts:601-639` | Rust 仅在 `end_reason != "completed"` 时发出 `TurnStatusNode`；TS 始终发出。TS 的 `TurnNodeRenderer` 对正常 completed 返回 `null`，所以视觉效果一致，但语义流有差异。 | 统一为：两者都始终发出 `TurnStatusNode`，由 presentation 层决定是否可见。已完成态的正常 completed 不渲染 DOM。 |
| **G7** | **`TerminalDiagnosisMetadata` 字段丢失** | TS `reducer.ts` 构建 `TurnStatusNode` 时 | `TurnFinishedPayload` 当前不包含 `repeated_force_stops`、`repeated_warns`、`no_progress_count` 字段，这些只在 Rust `TerminalDiagnosisMetadata` 结构体中定义。 | Phase 1 将 `TerminalDiagnosisMetadata` 字段加入 `TurnFinishedPayload`（optional），TS reducer 透传。 |
| **G8** | **`ToolGroupNode` 仅客户端生成** | 两个归约器都不生成 | `ToolGroupNode` 仅在 `AssistantResponseBlock.buildPresentationItems()` 中按 tool family 聚合生成，不是 canonical node。 | 保持现状：`ToolGroupNode` 是 presentation-only 构造，不出现在 canonical node 列表中。在 `presentation.ts` 的 `ProcessInterval` 中完成分组。 |
| **G9** | **`duration_ms` 未计算** | `timeline_emit.rs:183` | `ToolCallFinished` 映射设置 `duration_ms: None`，应该从 `ToolExecuting` 和 `ToolResult` 时间差计算。 | Phase 1 在 `map_agent_event_to_timeline` 中利用 gateway 内存 state 计算 `duration_ms`。 |
| **G10** | **旧 stream store 仍为滚动数据源** | `MessageStream.tsx:170,748` | `stream.length` 用于判断是否触发历史加载、是否为空态。删除旧渲染路径后这些判断必须迁移到 timeline state。 | Phase 5：`stream.length` → `timelineNodes.length`；`loadChatStream` → timeline pagination；空态判断统一。 |

### 15.2 协议层面

| # | 差距 | 位置 | 修复方案 |
|---|---|---|---|
| **P1** | `ReasoningVisibility` 不存在 | `timeline.rs` / `types.ts` | Phase 1 新增枚举，optional 字段向后兼容 |
| **P2** | `AssistantTextRole` 不存在 | 同上 | Phase 1 新增枚举，optional 字段向后兼容 |
| **P3** | `client_message_id` 在顶层而非 payload | `UserMessageCreatedPayload` | Phase 1 移入 payload |
| **P4** | `nodeIdIndex` 仅在 Zustand `_meta` | `timeline-store.ts` | Phase 2 移入 `TimelineState`，可随 replay 重建 |
| **P5** | `ToolOutputPatch` 不存在 | 无 | Phase 2 新增 ephemeral overlay 类型和 WS 推送路径 |
| **P6** | `ProvisionalUserMessage` 不存在 | 无 | Phase 2 新增 optimistic overlay 类型 |
| **P7** | `SessionTimelineRecord` 不存在 | 无 | Phase 2 新增 store envelope 类型，包装 canonical + overlay |

---

## 16. Rust/TS 归约器对齐策略

### 16.1 问题

Rust (`timeline_store.rs::materialize_events_to_nodes`) 和 TypeScript (`reducer.ts::reduceTimelineEvent`) 各自独立实现了事件→节点的归约逻辑。当前通过人工审查和有限黄金测试保证一致性，不可持续。

### 16.2 策略：JSON 测试矢量

```
┌─────────────────────────────────────────────────────────────┐
│                  test_vectors/                              │
│  reducer/                                                   │
│    basic_text.json        ← 单文本 delta → snapshot         │
│    reasoning_visibility.json ← public/private/undefined     │
│    tool_lifecycle.json    ← start → progress → finish       │
│    approval_flow.json     ← request → allow/deny            │
│    turn_finish_cleanup.json ← running → cancelled 收口       │
│    late_tool_result.json  ← turn_finished 后的 finish       │
│    delta_coalesce.json    ← 多次 delta → 单 node            │
│    node_identity.json     ← kind/turn/visibility 冲突拒绝    │
│    gap_and_reorder.json   ← 乱序 seq、pending 不 materialize │
│    legacy_synthetic.json  ← legacy adapter 输出             │
│    full_complex_turn.json ← 多 interval、attention 混合      │
└─────────────────────────────────────────────────────────────┘
```

每个矢量文件结构：

```json
{
  "description": "Basic text delta → snapshot → finalize",
  "schema_version": 2,
  "input_events": [...],
  "expected_nodes": [...],
  "expected_state": {
    "maxSeq": 5,
    "turnIndex": { "turn-1": ["node-at-1"] }
  }
}
```

### 16.3 工作流

1. **Rust 侧**：`cargo test` 中运行 `reducer_golden_tests`，对每个矢量文件调用 `materialize_events_to_nodes`，assert 输出匹配 `expected_nodes`。
2. **TS 侧**：`reducer.test.ts` 中导入相同 JSON 矢量，调用 `reduceTimelineEvents`，assert `nodesAreEquivalent(actual, expected)`。
3. **CI**：两个测试套件必须都通过。矢量文件在 `crates/xiaolin-protocol/test-vectors/` 下，Rust 和 TS 各通过自己的导入机制读取。
4. **矢量更新**：当新增事件类型或修改归约行为时，同步更新矢量文件，两个实现一起修改。

### 16.4 长期方向

考虑将 Rust 归约器编译为 WASM，在前端直接调用，消除双实现。本期不强制。

---

## 17. 新 store 形状详细设计

### 17.1 SessionTimelineRecord（替代当前 `states: Record<string, TimelineState>`）

```ts
// lib/stores/timeline-store.ts

interface SessionTimelineRecord {
  // —— Source tracking ——
  source: TimelineSource;
  probeStartedAtMs: number;
  probeCompletedAtMs: number;

  // —— Canonical state ——
  canonical: TimelineState;  // events, nodes, maxSeq, turnIndex, eventTraces, nodeIdIndex

  // —— Gap management ——
  knownById: Map<string, TurnTimelineEvent>;       // runtime-only, 不可序列化
  appliedEvents: TurnTimelineEvent[];               // 连续前缀
  pendingBySeq: Map<number, TurnTimelineEvent>;     // runtime-only, 不可序列化
  lastContiguousSeq: number;
  gapFillInFlight: boolean;

  // —— Ephemeral overlay ——
  optimisticUsers: Record<string, ProvisionalUserMessage>;
  toolOutputPatches: Record<string, ToolOutputPatch>;
}

interface TimelineStore {
  records: Record<string, SessionTimelineRecord>;
  lastSeenSeq: Record<string, number>;

  // Actions
  initSession(sessionId: string): void;
  cleanupSession(sessionId: string): void;

  // Canonical ingestion
  ingestEvent(sessionId: string, event: TurnTimelineEvent): void;
  ingestEvents(sessionId: string, events: TurnTimelineEvent[]): void;
  replaceCanonicalTimeline(sessionId: string, state: TimelineState): void;

  // Source management
  setSource(sessionId: string, source: TimelineSource): void;
  startProbe(sessionId: string): void;
  completeProbe(sessionId: string, snapshotEvents: TurnTimelineEvent[]): void;

  // Gap management
  recordPending(sessionId: string, event: TurnTimelineEvent): void;
  fillGap(sessionId: string, events: TurnTimelineEvent[]): void;
  recalculateContiguous(sessionId: string): void;

  // Overlay management
  upsertOptimisticUser(sessionId: string, user: ProvisionalUserMessage): void;
  removeOptimisticUser(sessionId: string, clientMessageId: string): void;
  upsertToolOutputPatch(sessionId: string, patch: ToolOutputPatch): void;
  removeToolOutputPatch(sessionId: string, callId: string): void;

  // Utility
  setLastSeenSeq(sessionId: string, seq: number): void;
}
```

### 17.2 ingestEvent 逻辑

```ts
ingestEvent(sessionId, event) {
  const rec = this.records[sessionId];

  // 1. 检查是否已知
  if (rec.knownById.has(event.id)) return; // 幂等

  // 2. 加入 known 集合
  rec.knownById.set(event.id, event);

  // 3. 判断是否连续
  if (event.seq === rec.lastContiguousSeq + 1) {
    // 连续 → 直接 apply
    rec.appliedEvents.push(event);
    rec.canonical = reduceTimelineEvent(rec.canonical, event);
    rec.lastContiguousSeq = event.seq;

    // 检查 pending 中是否有可连上的
    this.recalculateContiguous(sessionId);
  } else if (event.seq > rec.lastContiguousSeq + 1) {
    // 有 gap → 暂存 pending
    rec.pendingBySeq.set(event.seq, event);

    // 触发 gap fill（去抖）
    if (!rec.gapFillInFlight) {
      this.triggerGapFill(sessionId, rec.lastContiguousSeq);
    }
  }
  // event.seq <= lastContiguousSeq → 已过时，忽略
}
```

### 17.3 recalculateContiguous 逻辑

```ts
recalculateContiguous(sessionId) {
  const rec = this.records[sessionId];

  while (true) {
    const nextSeq = rec.lastContiguousSeq + 1;
    const pending = rec.pendingBySeq.get(nextSeq);
    if (!pending) break;

    rec.pendingBySeq.delete(nextSeq);
    rec.appliedEvents.push(pending);
    rec.canonical = reduceTimelineEvent(rec.canonical, pending);
    rec.lastContiguousSeq = nextSeq;
  }
}
```

---

## 18. 性能考量

### 18.1 Event 体积

| 场景 | 单 event 典型大小 | 备注 |
|---|---|---|
| `turn_started` / `turn_finished` | ~200 B | 元数据为主 |
| `assistant_text_delta` | 50–500 B | 高频小包 |
| `assistant_text_snapshot` | 1–64 KB | 完成时一次性 |
| `tool_call_started` | ~300 B | tool name + args summary |
| `tool_call_progress` | ~200 B | message only |
| `tool_call_finished` | 2–10 KB | 含 output_preview |
| `reasoning_delta` (public) | 50–300 B | narration 短文本 |

### 18.2 会话规模估算

| 指标 | 典型值 | 上限 |
|---|---|---|
| 每 turn events | 10–50 | 200（长工具链） |
| 每 turn nodes | 3–15 | 50 |
| 每 session turns | 5–30 | 200 |
| 每 session events | 50–1500 | 10000 |
| 每 session nodes | 15–450 | 3000 |

### 18.3 瓶颈与对策

| 瓶颈 | 风险 | 对策 |
|---|---|---|
| 全量分页加载大 session | 10000 events × 2 KB = 20 MB 网络 | 当前 `limit=500` 分页 + 浏览器缓存；后续窗口化 |
| Reducer 重放 10000 events | ~50ms 主线程 | 可接受；后续考虑 WASM |
| Virtualizer 3000 nodes | ~30ms measureElement | `@tanstack/react-virtual` 已处理；`estimateSize` 优化初始布局 |
| `knownById` Map 10000 entries | ~5 MB 内存 | 仅当前 session 保留；`cleanupSession` 释放 |
| WS partial output 500ms 节流 | 每个 running tool 最多 2 msg/s | 8KB tail buffer 限制带宽 |

### 18.4 未来窗口化准备

当前全量分页设计为 Phase 1 正确性优先。窗口化需要：

1. 服务端新增 `before_seq` 或 opaque cursor API；
2. 前端 `TimelineState` 支持 `events` 非全量（仅当前窗口 + anchor）；
3. 滚动到顶部/跳转到特定 turn 时触发窗口加载；
4. `nodeIdIndex` 和 `turnIndex` 仍需全量维护（体积小，可单独加载）。

---

## 19. 风险登记

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| **旧 stream 路径删除引发回归** | 中 | 高：旧 session 无法显示 | feature flag `useTimelineTranscript` 控制切换；灰度发布 |
| **Rust/TS 归约器长期分歧** | 高 | 中：history replay 与 live 不一致 | 黄金测试矢量 + CI 双向验证 |
| **Private reasoning 泄漏** | 低 | 极高：安全事件 | 多层防御：写入前拒绝 + WS emit 前拒绝 + history API 前拒绝 + 前端不渲染 undefined visibility |
| **Gap fill 死循环** | 低 | 中：CPU 100% | `gapFillInFlight` 标志 + 最大重试 3 次 + 指数退避 |
| **Optimistic user 与 authoritative 不同步** | 中 | 低：用户看到重复气泡 | `client_message_id` 原子 reconciliation；超时 30s 自动移除 optimistic |
| **全量分页大 session OOM** | 低 | 高：浏览器崩溃 | 监控 `events.length`；超过 5000 events 警告用户刷新；后续窗口化 |
| **Legacy migration 数据丢失** | 中 | 中：旧会话部分内容不可见 | 明确降级标记 + notice；不伪造顺序 |
| **ToolOutputPatch 带宽** | 低 | 低：WS 拥堵 | 8KB tail + 500ms throttle；完成立即释放 |

---

## 20. 未决事项

以下事项需在实施前或实施中决策：

1. **Activity narration producer 实现位置**：Rust gateway 侧独立模块，还是 agent 流程中的独立步骤？建议 gateway 侧 `narration_producer.rs`，接收 `(tool_name, target, state, result_summary)` 生成短文本。

2. **Legacy reasoning 安全确认**：存量 reasoning 内容是否可能包含敏感信息？若无法确认，统一不展示（当前设计已采用此策略）。

3. **Feature flag 策略**：是否使用 `useTimelineTranscript` feature flag 控制新旧路径切换？建议 Phase 4–5 之间引入，允许一键回退。

4. **WASM 归约器**：是否将 Rust 归约器编译为 WASM 以消除双实现？本期不强制，但黄金测试矢量设计为 WASM 迁移做准备（矢量文件与实现语言无关）。

5. **SmallOutputPolicy 阈值**：当前 `SMALL_OUTPUT_MAX_BYTES=8000`（~2K tokens）。是否需要可配置？建议先硬编码，收集使用数据后调整。

6. **显示节点 API 去留**：`GET /display-nodes` 是否保留？建议保留用于 admin/debug，生产路径走 events + 客户端 reducer。

---

## 21. 总结与后续步骤

### 21.1 设计核心决策回顾

```
                   ┌──────────────────────────────────┐
                   │ 1. Turn 是第一组织单元             │
                   │ 2. Canonical Timeline 唯一事实来源  │
                   │ 3. Private reasoning 永不离开后端   │
                   │ 4. Virtualizer 单一 owner          │
                   │ 5. 旧会话可读但不伪造时序           │
                   └──────────────────────────────────┘
```

### 21.2 实施优先级

```
Phase 1 (协议+安全) ──→ Phase 2 (source+补包) ──→ Phase 3 (legacy)
                                                       │
                                                       ▼
                                              Phase 4 (presentation)
                                                       │
                                                       ▼
                                              Phase 5 (收敛)
```

Phase 1 和 Phase 2 可部分并行（协议定义与 store 改造独立）。

### 21.3 文档审批后行动

1. 创建 feature branch `turn-flow-redesign`；
2. 按 Phase 1–5 拆分 issue/task；
3. 每个 Phase 以对应验收标准为 Definition of Done；
4. 黄金测试矢量在 Phase 1 即建立，后续 Phase 持续扩展；
5. Phase 4 完成后引入 feature flag，Phase 5 完成后移除 flag 和旧代码。

---

> **文档版本**: v1.0  
> **最后更新**: 2026-06-30  
> **基线**: `linzetai/XiaoLin` main 分支 `07a6c63`  
> **状态**: 已审批
