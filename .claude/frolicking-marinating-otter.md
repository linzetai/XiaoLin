# FastClaw 重构方案 v4：Rust 高性能 Claude Code 内核 + FastClaw 架构

## 一、目标与定位

### 1.1 核心定位

**打造 Claude Code 的 Rust 高性能内核**，融合 FastClaw 的架构优势（多 Agent、飞书 Channel、Cron、Tauri 桌面），形成一个超强智能体平台。

核心理念：**不是从零重写，而是将 Claude Code 的精华能力注入 FastClaw 的成熟骨架**。

### 1.2 核心目标

1. **内存占用 < 100MB**（硬性约束）
2. **保留 Tauri 桌面应用**（React 19 前端）
3. **IM 风格界面**（多 Agent、多会话、消息列表）
4. **核心功能**：飞书 Channel（保留）、Cron 定时任务、多 Agent 协作
5. **Claude Code 核心引擎**：4 层上下文压缩、流式工具编排、查询循环状态机

### 1.3 技术选型

- **桌面壳**: Tauri 2 + React 19 + Vite
- **核心语言**: Rust
- **通信**: Tauri IPC Command（进程内函数调用）
- **UI 风格**: IM 聊天界面

---

## 二、Claude Code 核心架构深度分析

> 基于 `/home/linzetai/workspace/my_tools/claude-code/` 源码实际分析。

### 2.1 QueryEngine（查询引擎）

**源码位置**: `src/QueryEngine.ts`

QueryEngine 是 Claude Code 的核心会话管理器，一个 QueryEngine 实例对应一个会话。

**关键设计**:
- `submitMessage()` 是 AsyncGenerator，逐步 yield SDKMessage
- 管理 `mutableMessages: Message[]`（会话历史，跨 turn 持续）
- 每次 turn 开始清理 `discoveredSkillNames`，防止跨 turn 增长
- 持有 `AbortController`，支持取消
- 持有 `FileStateCache`（文件状态缓存，避免重复读取）
- 持有 `permissionDenials`（权限拒绝追踪）
- 持有 `totalUsage`（累计 token 使用量）

**FastClaw 对应**: `crates/fastclaw-agent/src/runtime/mod.rs` 的 `AgentRuntime`。
**差距**: FastClaw 的 `AgentRuntime` 不持有会话状态（无状态设计），每次调用由外部传入 messages。需要决定是否改为有状态模式。

### 2.2 Query Loop（查询循环 — 核心引擎）

**源码位置**: `src/query.ts` — `queryLoop()` 函数

这是 Claude Code 最核心的 while(true) 循环，每轮执行：

```
┌───────────────────────────────────────────────┐
│ 1. Snip Compact (HISTORY_SNIP)                │ 按轮次删除整个旧工具交互轮
│ 2. MicroCompact                               │ 渐进式淡化旧工具结果
│ 3. Context Collapse (CONTEXT_COLLAPSE)        │ 后台 span 级别摘要
│ 4. AutoCompact                                │ LLM 驱动的全会话摘要
│ 5. 调用 LLM API                               │ 流式响应
│ 6. 工具执行 (StreamingToolExecutor)            │ 并发/串行分区
│ 7. Yield 消息                                  │ 流式推送到客户端
│ 8. Token Budget 检查                           │ 决定 continue/stop
│ 9. 回到步骤 1                                  │
└───────────────────────────────────────────────┘
```

**FastClaw 对应**: `execute_stream()` 方法中的 while 循环。
**差距**: FastClaw 已有 `try_compress_chat` (类似 AutoCompact) 和 `microcompact_tool_results`，但缺少 Snip、Context Collapse、Reactive Compact、Token Budget。

### 2.3 四层上下文压缩系统（Claude Code 精华）

Claude Code 有四层互相配合的上下文压缩机制：

#### Layer 1: MicroCompact（工具结果渐进淡化）

**源码位置**: `src/services/compact/microCompact.ts`

**原理**: 对 COMPACTABLE_TOOLS（read_file、shell、grep、glob、web_search、web_fetch、edit、write）的旧结果，渐进式替换为更短的版本。

- 时间驱动微压缩：当距上次 assistant 消息超过缓存窗口，清理旧工具结果内容
- 缓存感知微压缩（Cached MC）：利用 API 的 cache_edits 特性，仅发送删除指令而非重发完整内容

**FastClaw 现状**: `tool_executor.rs` 的 `microcompact_tool_results()` 已实现三层渐进淡化（full → preview → oneliner），且有 `dedup_repeated_tool_calls()` 去重和 `semantic_header()` 语义摘要。**这比方案 v3 描述的强很多。**

**差距**: FastClaw 缺少时间驱动微压缩（基于 cache 过期时间）。

#### Layer 2: Snip Compact（历史截断）

**源码位置**: `src/services/compact/snipCompact.ts`（当前为 stub）

**原理**: 按 API round boundary 分组，整轮删除最旧的工具交互轮次，释放 token。与 MicroCompact 互补（MC 瘦身单条结果，Snip 删除整轮）。

**FastClaw 现状**: 无对应实现。`ContextCompactor` 的 `SlidingWindow` 策略类似但粒度不同。

#### Layer 3: Context Collapse（上下文折叠）

**源码位置**: `src/services/contextCollapse/index.ts`（当前为 stub）

**原理**: 后台异步将一个 API round（assistant + tool_results）折叠为摘要 span。折叠结果存储在独立的 collapse store 中，不修改原始消息数组。`projectView()` 在每轮查询时将折叠结果投影到查询消息中。

- 90% 触发提交（commit）
- 95% 触发阻塞式生成（blocking spawn）
- 与 AutoCompact 互斥：collapse 开启时 autocompact 禁用

**FastClaw 现状**: 无对应实现。

#### Layer 4: AutoCompact（自动全会话摘要）

**源码位置**: `src/services/compact/autoCompact.ts`

**原理**:
1. `shouldAutoCompact()`: 估算 token → 与阈值比较（effectiveContextWindow - 13k buffer）
2. 优先尝试 Session Memory Compaction
3. 回退到 `compactConversation()`：fork 一个子 agent 生成摘要
4. 三次连续失败触发熔断（circuit breaker），不再重试
5. `compact.ts` 中 `compactConversation()` 使用 `runForkedAgent` 执行，支持 pre/post compact hooks

**FastClaw 现状**: `context_compressor.rs` 的 `try_compress_chat()` 已实现 LLM 驱动压缩，有结构化压缩提示词（state_snapshot 格式）、历史文件保存、安全保护。**已基本对齐 AutoCompact 核心能力。**

**差距**: 缺少 circuit breaker、Session Memory Compaction、pre/post hooks。

#### 额外: Reactive Compact（响应式紧急压缩）

**源码位置**: `src/services/compact/reactiveCompact.ts`（当前为 stub）

**原理**: 当 API 返回 prompt_too_long 错误时，紧急触发压缩后重试。是最后的安全网。

**FastClaw 现状**: 无对应实现。

### 2.4 工具编排系统

#### StreamingToolExecutor（流式工具执行器）

**源码位置**: `src/services/tools/StreamingToolExecutor.ts`

**关键设计**: 工具在流式响应过程中就开始执行（不等完整响应结束），类似流水线。

- `addTool()`: 流式过程中收到 tool_use 即加入队列
- concurrency-safe 工具并行执行
- non-concurrent 工具独占执行
- 结果按接收顺序（非完成顺序）yield
- sibling abort：一个工具出错时可以取消同批次其他工具
- 进度消息（progress）实时推送

**FastClaw 现状**: `execute_tool_batch()` 已实现 concurrent/sequential 分区执行，有 dedup、hook、progress 支持。**但不支持流式添加工具（等完整 tool_calls 才执行）。**

#### Tool Orchestration（工具编排）

**源码位置**: `src/services/tools/toolOrchestration.ts`

- `partitionToolCalls()`: 将同一轮 tool_calls 分为 concurrent-safe 和 non-concurrent 批次
- Concurrent 批次并行执行，non-concurrent 批次串行执行
- 上下文修改器（contextModifier）延迟应用，避免并发竞争

**FastClaw 现状**: `execute_tool_batch()` 已有类似逻辑（基于 ToolKind 的分区）。**基本对齐。**

### 2.5 Token Budget（预算控制）

**源码位置**: `src/query/tokenBudget.ts`

- 追踪每轮 token 消耗
- 90% 阈值自动注入 continuation message
- 连续 3 轮 delta < 500 tokens 判定为 diminishing returns，停止
- 支持 task_budget（整个任务的 token 预算上限）

**FastClaw 现状**: 无对应实现。

---

## 三、差距矩阵与优先级

| 能力 | Claude Code | FastClaw 现状 | 差距 | 优先级 |
|------|------------|--------------|------|--------|
| QueryEngine 会话管理 | 有状态 AsyncGenerator | 无状态 execute_stream | 需要包装有状态层 | P0 |
| Query Loop 状态机 | 完整 while(true) + 8 步骤 | 有循环但缺步骤 | 补齐缺失步骤 | P0 |
| MicroCompact | 三层 + 时间驱动 + cached MC | 三层渐进淡化 + dedup + semantic header | 缺时间驱动 | P1 |
| Snip Compact | 按 API round 整轮删除 | 无 | 需新增 | P1 |
| Context Collapse | 后台 span 摘要 + 投影 | 无 | 需新增 | P2 |
| AutoCompact | LLM 摘要 + circuit breaker | LLM 压缩已有 | 补 circuit breaker | P1 |
| Reactive Compact | prompt_too_long 紧急压缩 | 无 | 需新增 | P0 |
| StreamingToolExecutor | 流式添加+执行 | batch 模式 | 可选优化 | P2 |
| Tool 分区执行 | concurrent/serial | 已有 | ✅ 已对齐 | - |
| Token Budget | 预算追踪 + 自动继续 | 无 | 需新增 | P1 |
| 工具结果截断 | 基本截断 | 三层截断 + 语义 header | ✅ FastClaw 更强 | - |
| 工具去重 | 无明确实现 | dedup_repeated_tool_calls | ✅ FastClaw 更强 | - |
| Permission 系统 | canUseTool + denial tracking | hook 系统 + confirm | 已基本对齐 | - |
| Self-iter 恢复 | 无（靠 reactive compact） | SelfIterEngine + grace turn | ✅ FastClaw 更强 | - |
| Skill/Evolution | 外部技能系统 | SkillStore + Trajectory | ✅ FastClaw 更强 | - |

---

## 四、架构设计

### 4.1 核心原则

1. **保留 FastClaw 已有优势**：microcompact 三层淡化、semantic header、dedup、self-iter、tool hooks
2. **注入 Claude Code 精华**：4 层压缩流水线、reactive compact、token budget、查询状态机
3. **先清场再建基**：删除 Claude Code 没有且 IM 不需要的模块（DAG/WASM Plugin/Eval/非飞书 Channel），有价值但 IM 初版不需要的模块 feature flag 化（Evolution/Self-iter），MCP 功能从 collab 拆出独立保留
4. **渐进式重构**：每个阶段独立可验证

### 4.2 目录结构调整

```
FastClaw/
├── crates/
│   ├── fastclaw-app/           # Tauri 桌面应用（不变）
│   ├── fastclaw-cli/           # CLI（不变）
│   ├── fastclaw-core/          # 核心（不变）
│   ├── fastclaw-gateway/       # 网关（feature flag 精简依赖）
│   ├── fastclaw-agent/         # 【重构核心】Agent 运行时
│   │   ├── src/
│   │   │   ├── runtime/
│   │   │   │   ├── mod.rs              # AgentRuntime（已有）
│   │   │   │   ├── query_engine.rs     # 【新增】有状态 QueryEngine 包装
│   │   │   │   ├── query_loop.rs       # 【重构】查询循环状态机
│   │   │   │   ├── context_compressor.rs # 已有 AutoCompact
│   │   │   │   ├── tool_executor.rs    # 已有 batch 执行 + microcompact
│   │   │   │   ├── stream_engine.rs    # 已有 LoopState
│   │   │   │   ├── accumulator.rs      # 已有
│   │   │   │   ├── prompt_builder.rs   # 已有
│   │   │   │   └── trajectory.rs       # 已有
│   │   │   ├── llm/            # LLM Provider（不变）
│   │   │   └── builtin_tools/  # 内置工具（不变）
│   │   └── Cargo.toml
│   ├── fastclaw-context/       # 【增强】上下文管理
│   │   ├── src/
│   │   │   ├── lib.rs          # 已有
│   │   │   ├── engine.rs       # 已有六层上下文组装
│   │   │   ├── compressor.rs   # 已有 ImportanceBased + SlidingWindow
│   │   │   ├── keyword_interceptor.rs  # 已有
│   │   │   ├── model_context.rs        # 已有
│   │   │   ├── snip.rs         # 【新增】Snip Compact (按 API round 截断)
│   │   │   ├── collapse.rs     # 【新增】Context Collapse (后台 span 摘要)
│   │   │   ├── reactive.rs     # 【新增】Reactive Compact (prompt_too_long 恢复)
│   │   │   └── budget.rs       # 【新增】Token Budget 追踪
│   │   └── Cargo.toml
│   ├── fastclaw-session/       # 会话管理（不变）
│   ├── fastclaw-memory/        # 记忆系统（不变）
│   ├── fastclaw-mcp/           # 【从 collab 拆出】MCP client/server（保留）
│   ├── fastclaw-cron/          # 定时任务（不变）
│   ├── fastclaw-security/      # 安全层（不变）
│   ├── fastclaw-observe/       # 监控（不变）
│   ├── fastclaw-model-router/  # 模型路由（保留，context 依赖它）
│   ├── fastclaw-treesitter/    # Tree-sitter（保留，code_intel 依赖它）
│   │
│   │ # Feature flag 可选（有价值但 IM 初版不需要）
│   ├── fastclaw-evolution/     # feature = "evolution"（Skill/Trajectory）
│   ├── fastclaw-self-iter/     # feature = "self-iter"（自动诊断恢复）
│   │
│   │ # 【删除】Claude Code 没有且 IM 不需要
│   │ # fastclaw-dag/           ← 删除（CC 无 DAG 引擎，用 Tasks 替代）
│   │ # fastclaw-plugin/        ← 删除（CC 无 WASM 插件）
│   │ # fastclaw-eval/          ← 删除（CC 无独立 eval 框架）
│   │ # fastclaw-collab/        ← 删除（MCP 已拆出，delegation/debate 不需要）
│
├── extensions/
│   └── feishu/                 # 保留
│   # 【删除】discord/telegram/slack/whatsapp/matrix/msteams（CC 无 channel 集成）
│
├── config/                     # 配置模板
└── Cargo.toml
```

### 4.3 核心模块设计

#### 模块 1: QueryEngine（有状态查询引擎）

**新文件**: `crates/fastclaw-agent/src/runtime/query_engine.rs`

参考 Claude Code 的 `QueryEngine` 类，为 FastClaw 添加有状态会话管理层：

```rust
/// 有状态查询引擎，一个实例对应一个会话。
/// 封装 AgentRuntime 的无状态 execute_stream，添加：
/// - 跨 turn 的消息历史管理
/// - 文件状态缓存
/// - 累计 token 使用量
/// - AbortController
pub struct QueryEngine {
    session_id: String,
    messages: Vec<ChatMessage>,
    abort_tx: Option<tokio::sync::oneshot::Sender<()>>,
    total_usage: UsageStats,
    file_state_cache: FileStateCache,
    permission_denials: Vec<PermissionDenial>,
    runtime: Arc<AgentRuntime>,
    config: Arc<AgentConfig>,
    context_pipeline: ContextPipeline,
}

impl QueryEngine {
    /// 提交用户消息并返回流式响应。
    /// 每次调用是一个 turn，会话状态跨 turn 持续。
    pub fn submit_message(
        &mut self,
        message: &str,
    ) -> impl Stream<Item = Result<StreamEvent>> + '_ {
        // 1. 添加用户消息到 self.messages
        // 2. 运行 context_pipeline.process() (snip → microcompact → collapse → autocompact)
        // 3. 委托 self.runtime.execute_stream()
        // 4. 收集 assistant 消息到 self.messages
        // 5. 更新 self.total_usage
        // 6. Token budget 检查
    }

    pub fn abort(&mut self) { /* ... */ }
    pub fn usage(&self) -> &UsageStats { /* ... */ }
    pub fn messages(&self) -> &[ChatMessage] { /* ... */ }
}
```

#### 模块 2: ContextPipeline（上下文压缩流水线）

**增强**: `crates/fastclaw-context/src/`

将 4 层压缩组装为流水线，每轮查询前按序执行：

```rust
/// 4 层上下文压缩流水线
/// 参考 Claude Code query.ts 的 queryLoop 中步骤 1-4
pub struct ContextPipeline {
    snip: Option<SnipCompactor>,
    microcompact_enabled: bool,
    collapse: Option<ContextCollapser>,
    autocompact: AutoCompactor,
    reactive: ReactiveCompactor,
    budget: TokenBudgetTracker,
}

impl ContextPipeline {
    /// 在每轮 LLM 调用前执行全部压缩层。
    /// 返回处理后的消息和元数据。
    pub async fn pre_query_compact(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        model: &str,
    ) -> CompactionMetadata {
        let mut metadata = CompactionMetadata::default();

        // Layer 1: Snip (删除最旧的完整 API round)
        if let Some(ref mut snip) = self.snip {
            let result = snip.compact_if_needed(messages);
            metadata.snip_tokens_freed = result.tokens_freed;
        }

        // Layer 2: MicroCompact (渐进淡化旧工具结果)
        // 已有 microcompact_tool_results() 和 dedup_repeated_tool_calls()
        if self.microcompact_enabled {
            dedup_repeated_tool_calls(messages);
            microcompact_tool_results(messages, 3);
        }

        // Layer 3: Context Collapse (后台 span 摘要投影)
        if let Some(ref mut collapse) = self.collapse {
            collapse.project_view(messages).await;
        }

        // Layer 4: AutoCompact (LLM 驱动全会话摘要)
        let compact_result = self.autocompact.compact_if_needed(
            messages, model, metadata.snip_tokens_freed,
        ).await;
        metadata.autocompacted = compact_result.was_compacted;

        metadata
    }

    /// API 返回 prompt_too_long 时的紧急恢复。
    pub async fn reactive_compact(
        &mut self,
        messages: &mut Vec<ChatMessage>,
    ) -> Result<bool> {
        self.reactive.compact_on_overflow(messages).await
    }
}
```

#### 模块 3: SnipCompactor（历史截断）

**新文件**: `crates/fastclaw-context/src/snip.rs`

参考 Claude Code 的 `grouping.ts` + `snipCompact.ts`：

```rust
/// 按 API round boundary 分组消息，删除最旧的完整轮次释放 token。
/// 与 MicroCompact 互补：MC 瘦身单条结果，Snip 删除整轮。
pub struct SnipCompactor {
    threshold_tokens: usize,
    min_rounds_to_keep: usize,
}

impl SnipCompactor {
    pub fn compact_if_needed(
        &self,
        messages: &mut Vec<ChatMessage>,
    ) -> SnipResult {
        let current_tokens = estimate_messages_tokens(messages);
        if current_tokens < self.threshold_tokens {
            return SnipResult::no_op();
        }

        let rounds = group_by_api_round(messages);
        // 从最旧的 round 开始删除，直到 token 在阈值内
        // 保留至少 min_rounds_to_keep 个最近轮次
        // ...
    }
}

/// 按 assistant message ID 边界分组（参考 Claude Code grouping.ts）
fn group_by_api_round(messages: &[ChatMessage]) -> Vec<Range<usize>> {
    // 每当出现新的 assistant 消息（不同 id）时开始新组
    // 一个组 = 一次完整的 API round-trip
}
```

#### 模块 4: ReactiveCompactor（响应式紧急压缩）

**新文件**: `crates/fastclaw-context/src/reactive.rs`

```rust
/// 当 LLM API 返回 prompt_too_long / context_length_exceeded 时的紧急压缩。
/// 这是最后的安全网。
pub struct ReactiveCompactor {
    provider: Arc<dyn LlmProvider>,
    max_attempts: u32,
}

impl ReactiveCompactor {
    /// 紧急压缩：使用更激进的策略（更短的 keep_recent、更低的阈值）
    pub async fn compact_on_overflow(
        &self,
        messages: &mut Vec<ChatMessage>,
    ) -> Result<bool> {
        // 1. 先尝试激进的 microcompact (keep_recent=1)
        // 2. 如果不够，执行 LLM 压缩（更激进的参数）
        // 3. 如果还不够，滑动窗口截断
    }
}
```

#### 模块 5: TokenBudgetTracker（预算追踪）

**新文件**: `crates/fastclaw-context/src/budget.rs`

```rust
/// Token 预算追踪器（参考 Claude Code query/tokenBudget.ts）
pub struct TokenBudgetTracker {
    budget: Option<usize>,
    continuation_count: u32,
    last_delta_tokens: usize,
    last_global_turn_tokens: usize,
    started_at: Instant,
}

pub enum BudgetDecision {
    Continue {
        nudge_message: String,
        pct: u32,
    },
    Stop {
        diminishing_returns: bool,
    },
}

impl TokenBudgetTracker {
    pub fn check(&mut self, global_turn_tokens: usize) -> BudgetDecision {
        // 90% 完成度自动 continue
        // 连续 3 轮 delta < 500 判定 diminishing returns → stop
    }
}
```

#### 模块 6: AutoCompact 增强

**增强文件**: `crates/fastclaw-agent/src/runtime/context_compressor.rs`

在现有 `try_compress_chat` 基础上添加：

```rust
/// AutoCompact 包装器，添加 circuit breaker 和追踪。
pub struct AutoCompactor {
    provider: Arc<dyn LlmProvider>,
    threshold: f32,
    consecutive_failures: u32,
    max_consecutive_failures: u32,  // 默认 3
    compacted_this_session: bool,
    turns_since_compact: u32,
}

impl AutoCompactor {
    pub async fn compact_if_needed(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        model: &str,
        snip_tokens_freed: usize,
    ) -> AutoCompactResult {
        // Circuit breaker
        if self.consecutive_failures >= self.max_consecutive_failures {
            return AutoCompactResult::skipped();
        }

        // 阈值检查（已有 token 估算 - snip 已释放的）
        // ...

        // 委托现有 try_compress_chat
        match try_compress_chat(messages, context_window, &self.provider, model, 0).await {
            ok if ok.compressed => {
                self.consecutive_failures = 0;
                self.compacted_this_session = true;
                self.turns_since_compact = 0;
                AutoCompactResult::compacted(ok)
            }
            _ => {
                self.consecutive_failures += 1;
                AutoCompactResult::failed()
            }
        }
    }
}
```

### 4.4 查询循环状态机重构

**重构文件**: `crates/fastclaw-agent/src/runtime/mod.rs` 的 `execute_stream`

在现有 while 循环中注入 ContextPipeline：

```rust
// 在现有 execute_stream 的 while 循环中，在调用 LLM 之前添加：

// Step 0: 上下文压缩流水线（参考 Claude Code query.ts:438-510）
let compact_metadata = context_pipeline.pre_query_compact(
    &mut messages, model
).await;

// ... 现有的 LLM 调用 ...

// 在收到 prompt_too_long 错误时：
if is_prompt_too_long_error(&error) {
    if context_pipeline.reactive_compact(&mut messages).await? {
        continue; // 重试
    }
}

// 在每轮结束时：
if let Some(budget) = &mut context_pipeline.budget {
    match budget.check(total_turn_tokens) {
        BudgetDecision::Continue { nudge_message, .. } => {
            messages.push(ChatMessage::system(&nudge_message));
        }
        BudgetDecision::Stop { .. } => break,
    }
}
```

---

## 五、工具系统差距分析与补齐计划

> 基于 Claude Code `packages/builtin-tools/src/tools/` 全部 55 个工具目录的逐一对比。

### 5.1 已对齐工具（无需变动）

| Claude Code 工具 | FastClaw 对应 | 备注 |
|-----------------|-------------|------|
| BashTool | `ShellTool` / `SandboxedShellTool` | FC 有沙箱模式，更强 |
| FileReadTool (Read) | `ReadFileTool` | ✅ |
| FileWriteTool (Write) | `WriteFileTool` | ✅ |
| FileEditTool | `EditFileTool` + `MultiEditTool` + `ApplyPatchTool` | FC 有三种编辑工具，更强 |
| GlobTool | `GlobTool` | ✅ |
| GrepTool | `SearchInFilesTool` | ✅ |
| WebFetchTool | `WebFetchTool` + `HttpFetchTool` | FC 有两种，更强 |
| WebSearchTool | `WebSearchTool` (7 引擎: Google/Baidu/Bing/Sogou/360/Searxng/Tavily) | FC 远更强 |
| TodoWriteTool | `TodoWriteTool` | ✅ |
| LSPTool | `UnifiedLspTool` + `FileOutlineTool` + `CodeChunkTool` | FC 更强 |
| AgentTool | `SubAgentTool` | ✅ |
| AskUserQuestionTool | `AskQuestionTool` | ✅ |
| SkillTool | `UnifiedSkillTool` (list + read + write) | ✅ |
| ScheduleCronTool (Create/List/Delete) | `fastclaw-cron` | ✅ |
| SendMessageTool | `SessionsSendTool` | ✅ |
| ListPeersTool | `ListAgentsTool` + `GetAgentInfoTool` | ✅ |
| ListMcpResources / ReadMcpResource | MCP client in `fastclaw-collab` | ✅ (feature-gated) |
| TeamCreate/TeamDelete | Agent 配置系统 | 概念对齐 |
| WebBrowserTool | `BrowserTool` (feature = "browser") | ✅ |

**FastClaw 独有优势工具**（Claude Code 没有的）:
- `ImageGenerateTool` / `TtsTool`（AI 生成能力）
- `MemorySearchTool` / `MemoryStoreTool` / `UnifiedMemoryTool`（长期记忆）
- `UnifiedIdentityTool`（SOUL.md / USER.md / AGENTS.md 管理）
- `ListDirectoryTool`（独立目录列举）
- `CalculatorTool` / `CurrentTimeTool`（基础工具）
- `ConfirmTool`（确认交互）

### 5.2 需要补齐的工具（P0 — 核心能力缺失）

#### 5.2.1 SnipTool — Agent 主动上下文裁剪

**Claude Code 功能**: Agent 可以主动选择 message_ids 从历史中删除，替换为摘要。这是 Agent 自主管理上下文的关键工具。

**为什么 P0**: 长对话场景下，Agent 能自主判断哪些旧信息不再需要，主动释放空间。这比被动压缩更精准。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/snip.rs
pub struct SnipTool;

impl Tool for SnipTool {
    fn name(&self) -> &str { "snip" }
    fn kind(&self) -> ToolKind { ToolKind::Think } // read-only, concurrent-safe

    // 输入: { message_ids: ["id1", "id2"], reason: "不再需要这些旧搜索结果" }
    // 输出: { snipped_count: 2, summary: "已移除2条旧消息，释放约3k tokens" }
    async fn execute(&self, args: &str) -> ToolResult {
        // 1. 从当前会话历史中查找指定 message_ids
        // 2. 生成简短摘要替换被删除的消息
        // 3. 返回释放的 token 数量
    }
}
```

#### 5.2.2 ToolSearchTool — 动态工具发现

**Claude Code 功能**: 当工具数量很多时，不在初始 prompt 中暴露所有工具，而是将低频工具标记为 "deferred"。Agent 通过 ToolSearch 按关键词查找并激活需要的工具。

**为什么 P0**: FastClaw 有 30+ 内置工具 + MCP 工具，全部暴露会浪费大量 context。ToolSearch 让 Agent 按需发现工具。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/tool_search.rs
pub struct ToolSearchTool {
    registry: Arc<ToolRegistry>,
}

impl Tool for ToolSearchTool {
    fn name(&self) -> &str { "tool_search" }
    fn kind(&self) -> ToolKind { ToolKind::Search }

    // 输入: { query: "image generation", max_results: 5 }
    // 或: { query: "select:image_generate" } (直接激活)
    // 输出: { matches: ["image_generate", "tts"], total_deferred_tools: 15 }
    async fn execute(&self, args: &str) -> ToolResult {
        // 1. 从 registry 中查找 deferred 工具
        // 2. 按 name + description + searchHint 模糊匹配
        // 3. 返回匹配的工具列表
        // 4. "select:" 前缀直接激活指定工具
    }
}
```

#### 5.2.3 SleepTool — 定时等待

**Claude Code 功能**: Agent 可以暂停执行指定秒数，用于等待异步操作（编译、部署等）。

**为什么 P0**: 这是最简单但缺失的基础工具。Agent 在等待 shell 命令、服务启动等场景需要它。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/utility.rs (添加到现有文件)
pub struct SleepTool;

impl Tool for SleepTool {
    fn name(&self) -> &str { "sleep" }
    fn kind(&self) -> ToolKind { ToolKind::Think }

    // 输入: { seconds: 5 }
    async fn execute(&self, args: &str) -> ToolResult {
        let seconds = parse_seconds(args).min(300); // 上限 5 分钟
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        ToolResult::ok(format!("Slept for {seconds} seconds"))
    }
}
```

### 5.3 需要补齐的工具（P1 — 重要体验提升）

#### 5.3.1 NotebookEditTool — Jupyter Notebook 编辑

**Claude Code 功能**: 对 .ipynb 文件进行 cell 级别的操作：插入、替换、删除 cell，支持 code/markdown 类型。

**为什么 P1**: 数据科学和 AI 工程师的核心工作流。不编辑整个 JSON 文件而是操作 cell 抽象。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/notebook.rs
pub struct NotebookEditTool;

impl Tool for NotebookEditTool {
    fn name(&self) -> &str { "notebook_edit" }
    fn kind(&self) -> ToolKind { ToolKind::Edit }

    // 输入: {
    //   notebook_path: "/path/to/notebook.ipynb",
    //   cell_id: "cell_3",           // 可选，不指定则新建
    //   new_source: "import pandas as pd\ndf = pd.read_csv('data.csv')",
    //   cell_type: "code",           // "code" | "markdown"
    //   insert_after: "cell_2",      // 可选，新建时指定位置
    //   delete: false,               // true 则删除该 cell
    // }
    async fn execute(&self, args: &str) -> ToolResult {
        // 1. 解析 .ipynb JSON
        // 2. 定位/创建 cell
        // 3. 应用修改
        // 4. 写回文件
    }
}
```

#### 5.3.2 BriefTool (SendUserMessage) — 主动消息推送

**Claude Code 功能**: Agent 主动向用户发送消息（而非作为回复），支持附件。区分 "normal" 和 "proactive" 状态。

**为什么 P1**: 长任务执行中，Agent 需要主动报告进度、问题、完成状态。特别是后台 agent 完成任务时需要通知用户。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/brief.rs
pub struct BriefTool;

impl Tool for BriefTool {
    fn name(&self) -> &str { "send_user_message" }
    fn kind(&self) -> ToolKind { ToolKind::Think }

    // 输入: {
    //   message: "## 编译完成\n所有测试通过 ✅",
    //   attachments: ["/path/to/build.log"],
    //   status: "proactive"  // "normal" | "proactive"
    // }
    async fn execute(&self, args: &str) -> ToolResult {
        // 通过 StreamEvent::UserMessage 推送到前端
    }
}
```

#### 5.3.3 TaskTool 系列 — 并行后台任务管理

**Claude Code 功能**: TaskCreate/TaskGet/TaskList/TaskUpdate/TaskStop/TaskOutput — 创建并行后台任务（不同于 TodoWrite 的清单追踪）。每个 Task 是一个独立的子 Agent 执行流。

**为什么 P1**: 支持"best-of-N"并行策略、后台编译/测试、多分支探索。这是 Claude Code 多 Agent 能力的核心。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/task.rs
pub struct TaskCreateTool { manager: Arc<TaskManager> }
pub struct TaskListTool { manager: Arc<TaskManager> }
pub struct TaskGetTool { manager: Arc<TaskManager> }
pub struct TaskUpdateTool { manager: Arc<TaskManager> }
pub struct TaskStopTool { manager: Arc<TaskManager> }

pub struct TaskManager {
    tasks: DashMap<String, TaskState>,
    max_concurrent: usize,
}

pub struct TaskState {
    id: String,
    subject: String,
    status: TaskStatus, // pending | in_progress | completed | cancelled
    agent_handle: Option<JoinHandle<()>>,
    output: Option<String>,
}
```

#### 5.3.4 EnterPlanMode / ExitPlanMode — Agent 发起的模式切换

**Claude Code 功能**: Agent 可以主动请求进入 Plan 模式（只读探索）或退出回到执行模式。在 Plan 模式下，Agent 不执行写操作，只分析和规划。

**为什么 P1**: 复杂任务需要先规划再执行。Agent 自主判断何时需要停下来思考，是高质量输出的关键。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/plan_mode.rs
pub struct EnterPlanModeTool;
pub struct ExitPlanModeTool;
pub struct VerifyPlanExecutionTool;

// EnterPlanMode: 切换到只读模式，write 类工具被禁用
// ExitPlanMode: 恢复正常模式
// VerifyPlanExecution: 退出前验证计划是否完成
```

#### 5.3.5 WorkflowTool — 预定义工作流执行

**Claude Code 功能**: 加载 `.claude/workflows/` 下的工作流定义文件，按步骤执行。支持 start/status/advance/cancel/list。

**为什么 P1**: 企业场景中，常见操作（发布流程、代码审查、环境搭建）可以定义为可复用工作流。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/workflow.rs
pub struct WorkflowTool {
    workflow_dir: PathBuf,
}

pub struct WorkflowRun {
    run_id: String,
    workflow: String,
    status: WorkflowStatus, // running | completed | cancelled
    steps: Vec<WorkflowStep>,
    current_step_index: usize,
}
```

#### 5.3.6 TerminalCaptureTool — 终端面板捕获

**Claude Code 功能**: 读取 IDE 终端面板的当前输出内容。用于查看正在运行的进程、编译日志等。

**为什么 P1**: 与 ShellTool 互补 — Shell 执行新命令，TerminalCapture 读取已有终端的输出。

```rust
// 文件: crates/fastclaw-agent/src/builtin_tools/terminal.rs
pub struct TerminalCaptureTool;

impl Tool for TerminalCaptureTool {
    fn name(&self) -> &str { "terminal_capture" }
    fn kind(&self) -> ToolKind { ToolKind::Search }

    // 输入: { lines: 50, panel_id: "terminal-1" }
    // 输出: 终端最近 50 行内容
}
```

### 5.4 可选工具（P2 — 特定场景）

| Claude Code 工具 | 用途 | FastClaw 建议 |
|-----------------|------|-------------|
| PowerShellTool | Windows PowerShell 执行 | ShellTool 已支持跨平台，可延后 |
| EnterWorktree/ExitWorktree | Git worktree 隔离 | 需要时实现 |
| ConfigTool | 配置管理 | FastClaw 已有 config ACL |
| DiscoverSkillsTool | 技能发现 | UnifiedSkillTool 已含 list |
| ReviewArtifactTool | 制品审查 | 可延后 |
| SendUserFileTool | 文件发送 | BriefTool 的 attachments 覆盖 |
| SubscribePRTool | PR 订阅 | 可延后 |
| SuggestBackgroundPRTool | 后台 PR 建议 | 可延后 |
| RemoteTriggerTool | 远程触发 | 可延后 |
| PushNotificationTool | 推送通知 | 桌面端可用系统通知 |
| REPLTool | REPL 模式 | ShellTool 已覆盖 |
| CtxInspectTool | 上下文检视 | 调试用，可延后 |
| McpAuthTool | MCP 认证 | fastclaw-collab 已有 |
| MonitorTool | 监控 | fastclaw-observe 已有 |
| OverflowTestTool | 溢出测试 | 仅测试用 |
| SyntheticOutputTool | 合成输出 | 内部用 |
| TungstenTool | Tungsten 集成 | Anthropic 内部 |

### 5.5 工具系统架构增强

除了补齐具体工具，还需要以下架构增强：

#### 5.5.1 Deferred Tools（延迟加载工具）

参考 Claude Code 的 ToolSearch 机制：

```rust
// 在 ToolRegistry 中添加 deferred 支持
impl ToolRegistry {
    /// 注册为 deferred 工具（不在初始 prompt 中暴露）
    pub fn register_deferred(&self, tool: Arc<dyn Tool>) { /* ... */ }

    /// 获取所有 eager（非 deferred）工具的定义
    pub fn eager_definitions(&self) -> Vec<ToolDefinition> { /* ... */ }

    /// 搜索 deferred 工具并激活
    pub fn search_and_activate(&self, query: &str) -> Vec<ToolDefinition> { /* ... */ }
}
```

#### 5.5.2 Tool searchHint（搜索提示）

每个工具添加 `searchHint` 字段，用于 ToolSearch 的模糊匹配：

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn search_hint(&self) -> &str { "" } // 新增：搜索关键词
    fn is_deferred(&self) -> bool { false } // 新增：是否延迟加载
    // ...
}
```

#### 5.5.3 maxResultSizeChars（工具级输出限制）

Claude Code 每个工具有 `maxResultSizeChars` 属性。FastClaw 的 `tool_output_char_limit()` 已实现类似功能（通过 match），但应改为工具自声明：

```rust
pub trait Tool: Send + Sync {
    // ...
    fn max_result_size_chars(&self) -> usize { 1500 } // 默认值
}
```

### 5.6 实施计划补充

将工具补齐纳入阶段规划：

| 阶段 | 新增工具 | 工作量 |
|------|---------|--------|
| 阶段一（压缩流水线）| 无 | - |
| 阶段二（QueryEngine + Feature Flag）| SleepTool, SnipTool, ToolSearchTool + Deferred 机制 | +3d |
| 阶段三（前端 IM 界面）| BriefTool (SendUserMessage) | +1d |
| 阶段四（Context Collapse + 增强）| NotebookEditTool, TaskTool 系列, WorkflowTool, PlanMode 工具, TerminalCaptureTool | +5d |

---

## 六、基建策略

### 6.1 模块处置决策

基于排查结果（Claude Code 是否有对应物 + IM 桌面端是否使用），模块分三类处置：

**直接删除**（CC 没有且 IM 不需要）：

| 模块 | 删除理由 | 涉及文件 |
|------|---------|---------|
| `fastclaw-dag` | CC 无 DAG 引擎，用 Tasks（sub-agent spawn）替代 | `crates/fastclaw-dag/`、`gateway/routes/dag.rs`、`gateway/state/builder.rs` 中引用 |
| `fastclaw-plugin` | CC 无 WASM 插件，用 MCP 扩展替代 | `crates/fastclaw-plugin/`、`gateway/state/helpers.rs`、`gateway/state/builder.rs` 中引用 |
| `fastclaw-eval` | CC 无独立 eval 框架，gateway 也未依赖 | `crates/fastclaw-eval/` |
| `fastclaw-collab` | delegation/debate/committee 不需要，MCP 拆出后删除 | `crates/fastclaw-collab/`（MCP 部分先拆出） |
| 6 个非飞书 channel | CC 无 channel 集成 | `extensions/telegram,discord,slack,whatsapp,matrix,msteams`、`gateway/state/mod.rs` 中引用 |

**Feature flag 化**（有价值但 IM 初版不需要）：

| 模块 | 保留理由 | 改造方式 |
|------|---------|---------|
| `fastclaw-evolution` | CC 有 `services/skillLearning/evolution.ts` 对应物，SkillStore/Trajectory 长期有价值 | `fastclaw-agent` 层 `feature = "evolution"`，`#[cfg]` 包裹 runtime/mod.rs 中 12 处引用 |
| `fastclaw-self-iter` | FastClaw 独有优势（CC 无），SelfIterEngine 对 Agent 质量有价值 | `fastclaw-agent` 层 `feature = "self-iter"`（默认开启），`#[cfg]` 包裹 stream_engine.rs 中引用 |

**拆分保留**：

| 模块 | 处理方式 |
|------|---------|
| `fastclaw-collab` 中 MCP 部分 | 拆出为 `fastclaw-mcp` crate，保留 `SharedMcpClient`、`register_mcp_tools`、`register_mcp_tools_sse`。gateway 引用从 `fastclaw_collab::mcp::*` 改为 `fastclaw_mcp::*` |

### 6.2 Feature Flag 配置

在 `fastclaw-agent/Cargo.toml` 中（内层）：

```toml
[features]
default = ["self-iter"]

evolution = ["dep:fastclaw-evolution"]
self-iter = ["dep:fastclaw-self-iter"]
```

在 `fastclaw-gateway/Cargo.toml` 中（外层，透传）：

```toml
[features]
default = ["self-iter"]

evolution = ["fastclaw-agent/evolution"]
self-iter = ["fastclaw-agent/self-iter"]
```

### 6.3 条件编译示例

```rust
// fastclaw-agent/src/runtime/mod.rs
#[cfg(feature = "evolution")]
use fastclaw_evolution::{
    format_candidate_skills_for_prompt, format_skills_for_prompt, infer_task_type, SkillStatus,
    SkillStore, Trajectory, TrajectoryOutcome, TrajectoryStep, TrajectoryStore,
};
#[cfg(feature = "self-iter")]
use fastclaw_self_iter::{SelfIterEngine, ToolCallTrace};

pub struct AgentRuntime {
    default_provider: Arc<dyn LlmProvider>,
    agent_providers: ArcSwap<HashMap<String, Arc<dyn LlmProvider>>>,
    #[cfg(feature = "self-iter")]
    self_iter_engine: Option<Arc<SelfIterEngine>>,
    #[cfg(feature = "evolution")]
    skill_store: ArcSwap<Option<Arc<SkillStore>>>,
    #[cfg(feature = "evolution")]
    trajectory_store: ArcSwap<Option<Arc<TrajectoryStore>>>,
}
```

### 6.4 轻量构建

```bash
# 最小构建（IM 桌面端默认）
cargo build --release -p fastclaw-gateway --no-default-features

# 带 self-iter 的标准构建（推荐）
cargo build --release -p fastclaw-gateway

# 完整构建（开启所有可选能力）
cargo build --release -p fastclaw-gateway --features "evolution"
```

---

## 七、实施计划

### 阶段零：基建 — 清场与解耦（1.5~2 周）

**目标**: 删除无用模块，feature flag 化可选模块，确保最小构建通过。

| 任务 | 文件 | 工作量 | 说明 |
|------|------|--------|------|
| 0.1 依赖图分析 | `cargo tree` | 0.5d | 梳理每个待删模块的被依赖关系 |
| 0.2 删除 6 个非飞书 channel | `extensions/` + `gateway/state/mod.rs` | 1d | 从 gateway 代码移除引用 → Cargo.toml 移除 → 删目录 |
| 0.3 删除 fastclaw-eval | `crates/fastclaw-eval/` + `Cargo.toml` | 0.5d | 无引用方，直接删 |
| 0.4 删除 fastclaw-dag | `crates/fastclaw-dag/` + `gateway/routes/dag.rs` + `gateway/state/builder.rs` + `gateway/lib.rs` | 1d | 移除 gateway 中 DAG 路由和 CheckpointStore |
| 0.5 拆分 fastclaw-collab → fastclaw-mcp | `crates/fastclaw-collab/src/mcp/` → `crates/fastclaw-mcp/` | 1.5d | 提取 MCP client/server 为独立 crate，gateway 引用改为 fastclaw_mcp |
| 0.6 删除 fastclaw-collab 残余 | `crates/fastclaw-collab/` | 0.5d | MCP 拆出后删除 delegation/debate/pipeline |
| 0.7 删除 fastclaw-plugin | `crates/fastclaw-plugin/` + `gateway/state/helpers.rs` + `gateway/state/builder.rs` | 1d | 移除 gateway 中 WASM host/bridge/hot-reload |
| 0.8 Feature flag: fastclaw-evolution | `fastclaw-agent/Cargo.toml` + `runtime/mod.rs` + `prompt_builder.rs` | 1.5d | `#[cfg(feature = "evolution")]` 包裹 ~12 处引用，提供无 evolution 退化路径 |
| 0.9 Feature flag: fastclaw-self-iter | `fastclaw-agent/Cargo.toml` + `runtime/mod.rs` + `stream_engine.rs` | 0.5d | `#[cfg(feature = "self-iter")]` 包裹引用，默认开启 |
| 0.10 Gateway feature flag 透传 | `fastclaw-gateway/Cargo.toml` | 0.5d | 透传 agent 层的 evolution/self-iter feature |
| 0.11 编译验证（三种组合） | workspace | 1d | `--no-default-features` / default / `--all-features` 全通过 + clippy 零警告 |
| 0.12 测试回归 | workspace | 0.5d | `cargo test --workspace` 全通过 |

### 阶段一：上下文压缩流水线（2 周）

**目标**: 在干净的依赖基础上，实现 Claude Code 的 4 层压缩，注入查询循环。

| 任务 | 文件 | 工作量 | 说明 |
|------|------|--------|------|
| 1.1 SnipCompactor | `fastclaw-context/src/snip.rs` | 2d | 按 API round 分组 + 截断 |
| 1.2 ReactiveCompactor | `fastclaw-context/src/reactive.rs` | 1d | prompt_too_long 紧急恢复 |
| 1.3 TokenBudgetTracker | `fastclaw-context/src/budget.rs` | 1d | 预算追踪 + diminishing returns |
| 1.4 AutoCompact 增强 | `agent/runtime/context_compressor.rs` | 1d | 添加 circuit breaker |
| 1.5 ContextPipeline | `fastclaw-context/src/pipeline.rs` | 2d | 4 层组装 + 配置 |
| 1.6 注入查询循环 | `agent/runtime/mod.rs` | 2d | 在 execute_stream 中调用 pipeline |
| 1.7 测试 | 各模块 | 1d | 单元测试 + 集成测试 |

### 阶段二：QueryEngine + P0 工具（2 周）

| 任务 | 文件 | 工作量 | 说明 |
|------|------|--------|------|
| 2.1 QueryEngine | `agent/runtime/query_engine.rs` | 3d | 有状态会话包装 |
| 2.2 Gateway 适配 | `gateway/` | 1d | 适配 QueryEngine 接口 |
| 2.3 SleepTool | `agent/builtin_tools/utility.rs` | 0.5d | 定时等待 |
| 2.4 Tool trait 增强 | `core/tool.rs` | 1d | search_hint + is_deferred + max_result_size_chars |
| 2.5 ToolRegistry deferred 支持 | `core/tool.rs` | 1d | register_deferred + eager_definitions + search_and_activate |
| 2.6 ToolSearchTool | `agent/builtin_tools/tool_search.rs` | 1d | 动态工具发现 |
| 2.7 SnipTool | `agent/builtin_tools/snip.rs` | 1d | Agent 主动上下文裁剪 |
| 2.8 测试验证 | 全局 | 1.5d | cargo test --workspace + clippy |

### 阶段三：前端 IM 界面 + BriefTool（3 周）

| 任务 | 说明 |
|------|------|
| 3.1 BriefTool | Agent 主动消息推送 |
| 3.2 Agent 列表面板 | 左侧栏：Agent 列表 + 新建 |
| 3.3 会话标签页 | 顶部：多会话切换 |
| 3.4 消息列表 | 中间：IM 风格消息流（气泡、markdown、代码高亮） |
| 3.5 工具调用展示 | 折叠式工具调用 + 结果展示 |
| 3.6 输入框 | 底部：多行输入 + 文件拖拽 + 快捷键 |
| 3.7 Tauri IPC 流式适配 | Channel 推送流式事件 |
| 3.8 会话状态管理 | Zustand/Jotai store |

### 阶段四：Context Collapse + P1 工具（3 周）

| 任务 | 说明 |
|------|------|
| 4.1 CollapseStore | span 摘要独立存储 |
| 4.2 后台摘要生成 | 异步 LLM 调用生成 span 摘要 |
| 4.3 projectView | 每轮查询时投影折叠结果 |
| 4.4 与 AutoCompact 互斥 | collapse 开启时禁用 autocompact |
| 4.5 NotebookEditTool | Jupyter Notebook cell 级编辑 |
| 4.6 TaskTool 系列 | TaskCreate/List/Get/Update/Stop |
| 4.7 WorkflowTool | 预定义工作流加载与执行 |
| 4.8 PlanMode 工具 | EnterPlanMode/ExitPlanMode/VerifyPlanExecution |
| 4.9 TerminalCaptureTool | 终端面板输出捕获 |

### 阶段五：测试与优化（1 周）

| 任务 | 说明 |
|------|------|
| 5.1 内存测试 | 确保 < 100MB |
| 5.2 长会话测试 | 200+ 轮对话的压缩效果 |
| 5.3 性能基准 | 各压缩层的延迟测量 |
| 5.4 功能回归 | cargo test --workspace 全通过 |
| 5.5 Feature flag 组合测试 | no-default-features / default / all-features |
| 5.6 Deferred Tools 效果验证 | 对比 context 节省效果 |

---

## 八、时间线

| 阶段 | 时长 | 优先级 | 说明 |
|------|------|--------|------|
| 阶段零 | 1.5~2周 | **P0** | **基建：清场 + 解耦 + feature flag**（必须先做） |
| 阶段一 | 2周 | P0 | 4 层压缩流水线（核心竞争力） |
| 阶段二 | 2周 | P0 | QueryEngine + P0 工具（Sleep/Snip/ToolSearch + Deferred 机制） |
| 阶段三 | 3周 | P0 | IM 界面 + BriefTool |
| 阶段四 | 3周 | P1 | Context Collapse + P1 工具（Notebook/Task系列/Workflow/PlanMode/TerminalCapture） |
| 阶段五 | 1周 | P0 | 测试验证 |
| **总计** | **12.5~14周** | | **~3~3.5 个月** |

---

## 九、验证标准

### 8.1 内存验证

```bash
# 空闲状态: < 30MB
# 10 个活跃会话: < 80MB
# 200 轮长会话: < 100MB (压缩后)
```

### 8.2 压缩效果验证

```rust
#[tokio::test]
async fn test_context_pipeline_200_round_session() {
    // 模拟 200 轮对话，验证 token 始终在 context window 内
    let mut pipeline = ContextPipeline::default();
    let mut messages = Vec::new();
    let context_window = 128_000;

    for round in 0..200 {
        messages.push(user_message(&format!("Question {round}")));
        messages.push(assistant_with_tools(round));
        messages.push(tool_result(round));

        let metadata = pipeline.pre_query_compact(&mut messages, "gpt-4o").await;
        let tokens = estimate_messages_tokens(&messages);

        assert!(tokens < context_window, "round {round}: {tokens} >= {context_window}");
    }
}
```

### 8.3 Reactive Compact 验证

```rust
#[tokio::test]
async fn test_reactive_compact_on_overflow() {
    let mut pipeline = ContextPipeline::default();
    // 构造一个刚好超过 context window 的消息列表
    let mut messages = create_oversized_messages(200_000);

    let recovered = pipeline.reactive_compact(&mut messages).await.unwrap();
    assert!(recovered);

    let tokens = estimate_messages_tokens(&messages);
    assert!(tokens < 128_000);
}
```

---

## 十、风险控制

### 9.1 渐进式回滚

每个阶段独立可编译、可测试。Feature flag 确保：
- 新功能可以逐个启用/禁用
- 出问题时禁用单个 feature 即可回退
- 不需要 git revert 整个阶段

### 9.2 依赖解耦策略

对于要 feature-flag 化的模块：
1. 先在 `fastclaw-agent` 中用 `#[cfg(feature)]` 包裹所有引用
2. 编译验证 `--no-default-features` 通过
3. 再修改 `fastclaw-gateway` 的引用
4. 最后修改 Cargo.toml 的 dependency 为 optional

### 9.3 测试守护

- 每个阶段完成后必须：`cargo test --workspace` 全通过
- 每个阶段完成后必须：`cargo clippy --workspace -- -D warnings` 零警告
- Feature flag 组合测试：`cargo test --no-default-features` + `cargo test --all-features`

---

---

## 十一、v6 补充：PromptEngine + 工具截断重构

> 以下为 v6 架构审视后的补充内容。Phase 0~3 已完成，Phase 4 进行到 P4-08。
> 发现三大偏移：(1) Prompt 管理系统缺失 (2) 工具截断策略需对齐 CC (3) 工具行为指导需接入流程。

### 11.1 偏移诊断

Claude Code 的 agent 质量 **70% 来自 Prompt Engineering**（`prompts.ts` 996 行），而非工具实现。

**FastClaw 当前 prompt 系统：**
- `prompts/system-base.md` — 141 行静态 markdown
- `prompts/tool-usage-guide.md` — 工具参考手册
- `workspace.rs` → `default_runtime_system_prompt_for_agent()` — 简单拼接 (base + guide + role)

**CC 的 prompt 系统：**
- 14 个动态 section（每轮/每 session 计算），7 个静态 section（跨 session 可缓存）
- `systemPromptSection()` 带 memoize 缓存，`/clear` 或 `/compact` 时失效
- `DANGEROUS_uncachedSystemPromptSection()` 每轮强制重算（如 MCP instructions）
- `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 标记分离静态/动态区域（API prompt cache 边界）
- `buildEffectiveSystemPrompt()` 优先级层叠（override > coordinator > agent > custom > default + append）
- 每个 Tool 有 `prompt()` 方法返回丰富行为指导（BashTool 370 行）

**差距：CC 有 21 个 prompt section + 每工具独立 prompt，FastClaw 有 0 个动态 section + 无工具 prompt。**

### 11.2 PromptEngine 架构设计

**新文件**: `crates/fastclaw-agent/src/runtime/prompt_engine.rs`

```rust
/// Prompt section — 延迟计算、可缓存的系统提示片段。
pub struct PromptSection {
    pub name: &'static str,
    /// 计算函数：基于运行时上下文生成 prompt 片段。返回 None 表示该 section 不适用。
    pub compute: Box<dyn Fn(&PromptContext) -> Option<String> + Send + Sync>,
    /// true = 每轮强制重算（如 MCP instructions），false = 计算一次后缓存到 /clear
    pub cache_break: bool,
}

/// 构建 prompt 所需的运行时上下文
pub struct PromptContext {
    pub agent_config: Arc<AgentConfig>,
    pub enabled_tools: HashSet<String>,
    pub deferred_tool_count: usize,
    pub model_id: String,
    pub cwd: PathBuf,
    pub is_git: bool,
    pub platform: String,
    pub shell: String,
    pub execution_mode: ExecutionMode,  // Plan / Agent / Auto
    pub mcp_clients: Vec<McpClientInfo>,
    pub language_preference: Option<String>,
    pub token_budget: Option<usize>,
    pub memory_prompt: Option<String>,
    pub session_start_date: String,
}

pub enum ExecutionMode {
    Plan,   // 只读探索模式，write 类工具禁用
    Agent,  // 完整执行模式
    Auto,   // 自动判断
}

/// 分层、可缓存、动态组装的 Prompt 引擎
pub struct PromptEngine {
    static_sections: Vec<PromptSection>,   // 启动时确定，跨 session 可缓存
    dynamic_sections: Vec<PromptSection>,  // 每轮/每 session 计算
    section_cache: DashMap<String, Option<String>>,  // name → cached value
}

impl PromptEngine {
    /// 构建完整系统提示词。
    /// 返回 Vec<String>，每个元素是一个独立 section（便于 API prompt cache）。
    pub fn build_system_prompt(&self, ctx: &PromptContext) -> Vec<String> {
        let mut parts = Vec::new();

        // 1. 静态 sections（缓存后不重算）
        for section in &self.static_sections {
            let value = self.resolve_section(section, ctx);
            if let Some(v) = value {
                parts.push(v);
            }
        }

        // 2. 动态边界标记（API prompt cache 分割点）
        parts.push("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__".into());

        // 3. 动态 sections
        for section in &self.dynamic_sections {
            let value = if section.cache_break {
                (section.compute)(ctx) // 强制重算
            } else {
                self.resolve_section(section, ctx)
            };
            if let Some(v) = value {
                parts.push(v);
            }
        }

        parts
    }

    /// 优先级层叠构建最终有效 prompt。
    /// override > agent_prompt > custom_prompt > default + append
    pub fn build_effective_prompt(
        &self,
        ctx: &PromptContext,
        override_prompt: Option<&str>,
        agent_prompt: Option<&str>,
        custom_prompt: Option<&str>,
        append_prompt: Option<&str>,
    ) -> Vec<String> {
        if let Some(ovr) = override_prompt {
            return vec![ovr.to_string()];
        }
        let base = if let Some(ap) = agent_prompt {
            vec![ap.to_string()]
        } else if let Some(cp) = custom_prompt {
            vec![cp.to_string()]
        } else {
            self.build_system_prompt(ctx)
        };
        if let Some(append) = append_prompt {
            let mut result = base;
            result.push(append.to_string());
            result
        } else {
            base
        }
    }

    /// 清除所有缓存（/clear, /compact, mode switch 时调用）
    pub fn clear_cache(&self) { self.section_cache.clear(); }

    fn resolve_section(&self, section: &PromptSection, ctx: &PromptContext) -> Option<String> {
        if let Some(cached) = self.section_cache.get(section.name) {
            return cached.clone();
        }
        let value = (section.compute)(ctx);
        self.section_cache.insert(section.name.to_string(), value.clone());
        value
    }
}
```

### 11.3 Prompt Section 清单

**静态 sections（编译期确定）:**

| Section | CC 对应 | 内容 |
|---------|---------|------|
| `intro` | `getSimpleIntroSection()` | 身份 + 安全指令（禁止生成 URL、prompt injection 防护） |
| `system` | `getSimpleSystemSection()` | system-reminder 说明、hooks 说明、压缩说明、deferred tools 提示 |
| `doing_tasks` | `getSimpleDoingTasksSection()` | 代码风格规范、验证要求、最小改动原则、注释规范 |
| `actions` | `getActionsSection()` | 可逆性判断、blast radius 评估、确认规则、具体危险操作列表 |
| `using_tools` | `getUsingYourToolsSection()` | 决策树(Step 0-3)、反模式、few-shot 示例、cost asymmetry、fallback chain |
| `tone_and_style` | `getSimpleToneAndStyleSection()` | 无 emoji、引用格式、建设性沟通 |
| `output_efficiency` | `getOutputEfficiencySection()` | 用户沟通规范、格式化、避免冗余 |

**动态 sections（运行时计算）:**

| Section | CC 对应 | cache_break | 内容 |
|---------|---------|-------------|------|
| `session_guidance` | `getSessionSpecificGuidanceSection()` | false | 基于启用工具 + 模式的动态指导 |
| `memory` | `loadMemoryPrompt()` | false | 从 memory 系统加载 |
| `environment` | `computeSimpleEnvInfo()` | false | cwd, platform, model, git, shell |
| `language` | `getLanguageSection()` | false | 语言偏好 |
| `mcp_instructions` | `getMcpInstructionsSection()` | **true** | MCP server 连接/断开，每轮重算 |
| `token_budget` | token_budget section | false | 预算指导 |
| `frc` | `getFunctionResultClearingSection()` | false | 旧工具结果自动清理提示 |

### 11.4 Tool.prompt() 方法 — 工具行为指导

CC 每个 Tool 的 `prompt()` 方法返回该工具的**完整行为指导**，作为 tool schema 的 description 字段发送给 LLM。这不是短描述，而是详细操作规范。

**修改 Tool trait:**

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;  // 短描述（UI 展示）

    /// 丰富的行为指导 prompt，发送给 LLM 作为 tool description。
    /// 包含使用说明、反模式、示例、约束等。
    /// 默认退化为短描述。
    fn prompt(&self) -> String { self.description().to_string() }
}
```

**核心工具 prompt 实现计划:**

| 工具 | CC 对应 | 预计行数 | 核心内容 |
|------|---------|----------|----------|
| `ShellTool` | BashTool (370行) | ~200行 | 专用工具优先规则、多命令规则、git 规则、sleep 规则、background 使用、沙箱说明 |
| `ReadFileTool` | FileReadTool (~50行) | ~50行 | 编码、大文件 offset/limit、图片/PDF |
| `WriteFileTool` | FileWriteTool (~30行) | ~30行 | 必须先 read、模式选择 |
| `EditFileTool` | FileEditTool (~30行) | ~40行 | 必须先 read、唯一性、replace_all |
| `GlobTool` | GlobTool (~20行) | ~30行 | 查询构造、fallback |
| `SearchInFilesTool` | GrepTool (~30行) | ~40行 | content words not descriptions、fallback chain |
| `SubAgentTool` | AgentTool (~40行) | ~50行 | 何时委托 vs 直接做 |
| `TodoWriteTool` | TodoWriteTool (~20行) | ~20行 | 任务管理规范 |
| `SnipTool` | SnipTool (~20行) | ~20行 | 何时主动裁剪 |
| `ToolSearchTool` | ToolSearchTool (~20行) | ~20行 | 搜索 vs 直接 select |

### 11.5 工具截断重构：Persist-to-Disk 策略

> 对齐 CC 的 `toolResultStorage.ts`

**CC 的工具结果处理（3 层）：**

1. **Per-tool persistence threshold** — 每工具声明 `maxResultSizeChars`（默认 50K，ShellTool 30K，ReadFile ∞），超过阈值时**整个结果持久化到磁盘**，替换为 2KB preview + 文件路径引用
2. **Per-message aggregate budget** (200K) — 同一轮 N 个并行工具结果总和超过 200K 时，最大的结果被持久化
3. **MicroCompact** (渐进淡化) — 老结果随时间淡化

**FastClaw 当前问题：**
- 截断是 in-place 的（head 20% + tail 80%），截断后**信息丢失**
- 没有 per-message aggregate budget — 10 个并行工具各 1500 chars = 15K OK，但如果 limit 提高，可能爆
- 保存到 temp 文件的路径不稳定（含时间戳），重启后丢失

**重构方案：**

```rust
// crates/fastclaw-agent/src/runtime/tool_result_storage.rs (新文件)

/// 工具结果持久化策略常量
pub const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;
pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;
pub const PREVIEW_SIZE_BYTES: usize = 2000;

/// 持久化大工具结果到会话目录
pub struct ToolResultStorage {
    session_dir: PathBuf,       // ~/.fastclaw/sessions/<session_id>/tool-results/
    seen_ids: HashSet<String>,  // 已处理过的 tool_use_id（决策不可变）
    replacements: HashMap<String, String>,  // tool_use_id → 替换后的 preview 文本
}

impl ToolResultStorage {
    /// 处理单个工具结果：超过阈值时持久化到磁盘，返回 preview。
    pub async fn process_result(
        &mut self,
        tool_use_id: &str,
        tool_name: &str,
        content: &str,
        max_result_size: usize,
    ) -> String {
        let threshold = max_result_size.min(DEFAULT_MAX_RESULT_SIZE_CHARS);
        if content.len() <= threshold {
            return content.to_string();
        }
        // 持久化到 session_dir/tool-results/<tool_use_id>.txt
        let filepath = self.persist(tool_use_id, content).await;
        let preview = self.generate_preview(content);
        let message = format!(
            "<persisted-output>\n\
             Output too large ({} chars). Full output saved to: {}\n\n\
             Preview (first ~2KB):\n{}\n...\n\
             </persisted-output>",
            content.len(), filepath.display(), preview
        );
        self.replacements.insert(tool_use_id.to_string(), message.clone());
        message
    }

    /// 对一轮的所有工具结果执行 per-message aggregate budget 检查。
    /// 当总和超过 200K 时，最大的结果被持久化。
    pub async fn enforce_per_message_budget(
        &mut self,
        results: &mut [(String, String, String)], // (tool_use_id, tool_name, content)
    ) {
        let total: usize = results.iter().map(|(_, _, c)| c.len()).sum();
        if total <= MAX_TOOL_RESULTS_PER_MESSAGE_CHARS {
            return;
        }
        // 按 size 降序，持久化最大的直到总量 < budget
        let mut indices: Vec<usize> = (0..results.len()).collect();
        indices.sort_by(|a, b| results[*b].2.len().cmp(&results[*a].2.len()));
        let mut remaining = total;
        for idx in indices {
            if remaining <= MAX_TOOL_RESULTS_PER_MESSAGE_CHARS { break; }
            let (id, name, content) = &results[idx];
            let replaced = self.process_result(id, name, content, usize::MAX).await;
            remaining -= content.len();
            remaining += replaced.len();
            results[idx].2 = replaced;
        }
    }
}
```

**关键改进 vs 当前实现：**

| 维度 | 当前 (v5) | 重构后 (v6) |
|------|-----------|-------------|
| 截断策略 | head 20% + tail 80% in-place | 整体持久化到磁盘 + 2KB preview |
| 信息丢失 | 中间部分永久丢失 | 完整内容可通过 ReadFile 恢复 |
| 文件路径 | `/tmp/fastclaw_truncated/<ts>_<tool>.txt` | `~/.fastclaw/sessions/<sid>/tool-results/<tool_use_id>.txt` |
| 持久性 | 重启丢失 | 跟随 session 持久化 |
| Per-message budget | 无 | 200K aggregate limit |
| 空结果处理 | 无 | `(tool_name completed with no output)` marker |
| Cache 稳定性 | N/A | 决策一旦做出不可变（seenIds frozen） |

### 11.6 `build_messages` 重构

当前 `runtime/mod.rs` 的 `build_messages_with_subagent_ctx()` 是简单字符串拼接：

```rust
// 当前实现（仅 3 行核心逻辑）
let system_text = configured.unwrap_or(default_runtime_system_prompt_for_agent(id));
system_text.push_str(&subagent_block);
messages.push(ChatMessage::system(system_text));
```

**重构为：**

```rust
fn build_messages_with_subagent_ctx(...) -> Vec<ChatMessage> {
    let prompt_ctx = PromptContext::from_runtime(
        config, &self.tool_registry, &self.mcp_clients, ...
    );

    let system_parts = self.prompt_engine.build_effective_prompt(
        &prompt_ctx,
        None,                          // override
        config.system_prompt.as_deref(), // agent prompt
        None,                          // custom
        subagent_ctx.map(|c| build_subagent_prompt_block(c)).flatten().as_deref(),
    );

    let system_text = system_parts.join("\n\n");
    messages.push(ChatMessage::system(system_text));
    messages.extend_from_slice(user_messages);
    messages
}
```

### 11.7 修正后的时间线

| 阶段 | 状态 | 时长 |
|------|------|------|
| Phase 0: 基建清场 | ✅ DONE | - |
| Phase 1: 压缩流水线 | ✅ DONE | - |
| Phase 2: QueryEngine + P0 工具 | ✅ DONE | - |
| Phase 3: 前端 IM | ✅ DONE | - |
| Phase 4: Context Collapse + P1 工具 | 8/16 DONE | 1.5周（完成剩余） |
| **Phase 4.5: PromptEngine + 工具截断重构** | **新增** | **2.5~3 周** |
| Phase 5: 测试与优化 | PENDING | 1 周 |
| **修正后总剩余** | | **~5~5.5 周** |

---

## 十二、总结

### 方案核心差异（v6 vs v5）

| 维度 | v5 方案 | v6 方案 |
|------|---------|---------|
| Prompt 系统 | 静态 md 文件拼接 | **PromptEngine：21 个 section + 缓存 + 动态边界** |
| 工具行为指导 | 短 description 字符串 | **Tool.prompt() 返回完整行为规范（50~370行/工具）** |
| 工具截断 | head/tail in-place 截断 | **persist-to-disk + 2KB preview + ReadFile 恢复** |
| Per-message budget | 无 | **200K aggregate limit（防并行工具 context 爆炸）** |
| 空结果处理 | 无 | **marker 文本（防模型误判 stop sequence）** |
| 模式感知 | 无 | **Plan/Agent/Auto 影响 tool 可用性和 prompt** |

### 预期成果（在 v5 基础上新增）

- ✅ **Claude Code 级别 Prompt Engineering**：21 个 section 动态组装，对齐 CC 的 prompts.ts
- ✅ **工具行为指导**：10+ 核心工具有丰富的 prompt（不只是名字，而是教 Agent 怎么用）
- ✅ **Persist-to-disk 截断**：大结果不丢失，通过 ReadFile 可恢复
- ✅ **Per-message aggregate budget**：防止 N 个并行工具集体撑爆 context
- ✅ **Prompt cache 友好**：静态/动态分离 + section 缓存 + 决策不可变
- ✅ **模式感知 Prompt**：Plan 模式自动禁用 write 工具 + 调整行为指导
- ✅ **优先级层叠**：override > agent > custom > default + append
