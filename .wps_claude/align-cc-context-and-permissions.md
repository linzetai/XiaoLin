# 对齐 claude-code：上下文管理 + 权限边界 + 失败容错

> 分析日期：2026-04-30
> 对比基线：claude-code `src/services/compact/microCompact.ts`, `src/constants/toolLimits.ts`, `src/utils/shell/outputLimits.ts`, `src/utils/toolResultStorage.ts`

---

## 问题总结

| # | 问题 | 根因 | cc 的做法 |
|---|------|------|-----------|
| 1 | 1-2 次对话后上下文就满 | read_file 单次最大 32K chars 直接入 LLM 上下文（几次读文件 = 100K+ tokens）；旧 tool results **无渐进清除机制** | microCompact: 计数/时间触发清除旧结果 |
| 2 | 权限过严，工作目录外合法操作被拒 | `ensure_within_workspace` 只允许 workspace_root + state_dir | 白名单 + ask-user 确认（非硬拒） |
| 3 | 连续失败直接报错，不给模型恢复机会 | retry 只覆盖 API 层错误，工具层缺少方向引导 | 错误返回模型 + system-reminder + PromptTooLong 触发 compact |

> **校正说明（经三轮 code review）**：
> 
> Shell 工具已有激进压缩（800B 阈值 + `ok_split`）。XiaoLin 也已有：
> - `unified_pre_query_compact()` 每轮调用（含 `microcompact_tool_results(messages, 3)` + `time_based_microcompact()`）
> - `enforce_per_message_budget()` 每轮调用
> 
> **但实际根因是**：`read_file` 声明 `max_result_size_chars() = usize::MAX`，导致它被 `build_skip_tool_names()` 加入跳过列表，per-message budget 完全不约束 read_file 结果。同时 microcompact 只在迭代边界运行（下一轮开始前），**当前轮内**模型连续读多个文件时无任何限制。
>
> 典型场景：模型首轮读 4 个文件 → 4 × 32K = 128K → 上下文直接填满，还没到 microcompact 介入的时机。

---

## 方案一：工具结果上下文压控

### 1.1 ~~降低 Shell 输出默认限制~~ → 已确认无需修改

**结论**: Shell 已有效压缩。实际代码 (`shell.rs`):
- `TERMINAL_FILE_THRESHOLD = 800` — 输出 > 800 bytes 写文件，LLM 只收到 compact summary（15 行尾部 + 文件引用）
- 64KB `DEFAULT_MAX_OUTPUT_BYTES` 仅影响 `display_output`（UI 侧），不进入 LLM context
- 通过 `ToolResult::ok_split(llm_out, display)` 将 LLM 和 UI 输出分离

**真正的上下文膨胀来源**:
- `read_file`: `DEFAULT_READ_FILE_MAX_CHARS = 32_768`（32K chars 直接进入 LLM 上下文）
- `DEFAULT_READ_FILE_MAX_LINES = 2000` 行
- 多次 read_file 调用 → 几次就占满上下文
- 旧的 tool results 不会被清除

### 1.1b（替代）降低 read_file 进入 LLM 上下文的默认大小

**文件**: `crates/xiaolin-agent/src/builtin_tools/filesystem.rs`

**可选改动**（需评估影响）:
```rust
// 当前:
const DEFAULT_READ_FILE_MAX_CHARS: usize = 32_768;   // 32K — 较大

// 可选降至:
const DEFAULT_READ_FILE_MAX_CHARS: usize = 16_384;   // 16K — 减半
// 或采用 shell 的 ok_split 模式：LLM 收摘要 + 文件引用，UI 收全文
```

**注意**: 这个改动需要谨慎评估。read_file 返回更少内容可能影响模型理解代码的能力。

### 1.1c（根因修复）取消 read_file 的 budget 豁免

**根因**: `ReadFileTool::max_result_size_chars()` 返回 `usize::MAX`（`filesystem.rs` line 1283），导致：
1. `build_skip_tool_names()` 将 "read_file" 加入跳过集合
2. `enforce_per_message_budget()` 完全不约束 read_file 结果
3. 模型一轮内读 4 个文件 → 4 × 32K = 128K → 上下文直接满

**cc 的做法**: cc 也有 `read_file` 但它不豁免 budget。当多个 read_file 结果超出聚合限制时，最大的被持久化到磁盘。

**修复方案**（二选一）:

**方案 A（推荐）**: 给 read_file 设正常阈值，允许其参与 budget enforcement：
```rust
// filesystem.rs line 1283:
// Before:
fn max_result_size_chars(&self) -> usize { usize::MAX }
// After:
fn max_result_size_chars(&self) -> usize { 32_768 } // 32K — 与 DEFAULT_READ_FILE_MAX_CHARS 一致
```
这意味着当同一轮内 read_file 结果总量超过 200K 时，最大的会被持久化（preview + 磁盘文件），模型可通过 `read_file` 路径再次获取。

> **循环问题**：之前设为 usize::MAX 的原因是"持久化 read_file 结果后模型用 read_file 读持久文件 → 循环"。但实际上 `process_result()` 检查 `content.starts_with(PERSISTED_OUTPUT_TAG)` 后直接返回 `Ok(None)` 不再重复持久化，所以不会真正循环。

**方案 B**: 在 per-message budget 之外，对 read_file 单独做**轮内限制**：
```rust
// 在 tool 执行循环中，track 当前轮的 read_file 累积大小
// 超过阈值后注入提示让模型使用 offset/limit 分段读取
const MAX_READ_FILE_CHARS_PER_TURN: usize = 100_000; // 100K per turn
```

### 1.2 ~~Enforce per-message 聚合限制~~ → 已确认已实现

**结论**: 已完整实现且在运行时中激活。

`tool_result_storage.rs` 已有完整的 `enforce_per_message_budget()` 实现（第307行），包括：
- `ContentReplacementState` — 跟踪哪些 tool_use_id 已 seen/replaced
- `ToolResultEntry` 结构 + `BudgetEnforcementResult`
- 分区逻辑: must_reapply / frozen / fresh
- 选择最大的 fresh 结果持久化到磁盘
- `reconstruct_state()` 用于 session resume

**调用点** (`runtime/mod.rs` line 511, 988):
```rust
let newly_replaced = apply_message_budget(&tool_storage, &mut messages, &mut replacement_state, &skip_tool_names);
Self::persist_replacement_records(session_store, request.session_id.as_deref(), &newly_replaced).await;
```

每次 LLM 调用前都会执行，已完全生效。

### 1.3 MicroCompact: 渐进内容清除（cc 风格）

> **现状辨析**: XiaoLin 已有 ContextPipeline Layer 2 "micro_compact"（`pipeline.rs` line 289），但它是
> **importance-based message eviction** — 直接删除低分消息。cc 的 microcompact 是**内容替换** — 保留
> 消息结构（tool_use/tool_result 配对完整），只把旧工具结果的 content 替换为
> `"[Old tool result content cleared]"`。
>
> 两者的区别：
> - 删除消息 → 模型不知道曾经做过什么（可能重复操作）
> - 清除内容 → 模型知道做过但忘了细节（不会重复，需要时可重新获取）
>
> XiaoLin 已定义了 `TOOL_RESULT_CLEARED_MESSAGE`（`tool_result_storage.rs` line 18）但**从未使用**。

**前置条件（可选）** — 如果需要实现纯 cc-style 的基于时间戳的 microcompact，需扩展 `ChatMessage`。但**现有实现 `time_based_microcompact` 已通过 `iteration_boundaries`（运行时 `Instant`）解决了同样问题**，无需修改 ChatMessage 结构。以下代码仅供参考：

```rust
// 1. 添加时间戳字段（用于时间触发 microcompact）
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<serde_json::Value>,
    pub name: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<ToolCallId>,
    /// When this message was added (Unix ms). Used for time-based microcompact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_millis: Option<u64>,
}

// 2. 添加辅助方法
impl ChatMessage {
    /// Replace content with a plain-text string (used by microcompact).
    pub fn set_content_text(&mut self, text: String) {
        self.content = Some(serde_json::Value::String(text));
    }
}
```

**新文件**: `crates/xiaolin-agent/src/runtime/microcompact.rs`

```rust
use std::collections::HashSet;
use xiaolin_core::types::{ChatMessage, Role};

/// MicroCompact 配置
#[derive(Debug, Clone)]
pub struct MicroCompactConfig {
    /// 可压缩工具列表（这些工具的旧结果可以被清除）
    pub compactable_tools: HashSet<String>,
    /// 当可压缩 tool_result 总数达到此阈值时触发清除
    pub trigger_threshold: usize,
    /// 清除时保留最近 N 个 tool_result（不清除）
    pub keep_recent: usize,
    /// 时间触发：距上次 assistant 回复超过 N 分钟时清除
    pub gap_threshold_minutes: u64,
}

impl Default for MicroCompactConfig {
    fn default() -> Self {
        Self {
            compactable_tools: [
                "read_file", "shell_exec", "shell", "search_in_files", "grep",
                "glob", "list_directory", "web_search", "web_fetch",
                "edit_file", "write_file",
            ].iter().map(|s| s.to_string()).collect(),
            trigger_threshold: 20,
            keep_recent: 5,
            gap_threshold_minutes: 5,
        }
    }
}

pub const CLEARED_MESSAGE: &str = "[Old tool result content cleared]";

/// 已清除的 tool_result 信息
pub struct MicroCompactResult {
    pub cleared_count: usize,
    pub freed_chars: usize,
}

/// 在每次 API 调用前执行，渐进清除旧的 tool results。
///
/// 对应 cc 的 `microcompactMessages()`:
/// 1. 收集所有 compactable 工具的 tool_result 位置
/// 2. 如果总数超过 trigger_threshold，保留最近 keep_recent 个，清除其余
/// 3. 被清除的 tool_result 内容替换为 CLEARED_MESSAGE
///
/// 不删除消息本身（保持 tool_use/tool_result 的配对关系），只清空内容。
pub fn microcompact_messages(
    messages: &mut [ChatMessage],
    config: &MicroCompactConfig,
) -> MicroCompactResult {
    // Step 1: 收集所有 compactable tool results
    // 格式: (message_index, tool_use_id, content_len)
    let mut compactable: Vec<(usize, String, usize)> = Vec::new();

    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role != Role::Tool {
            continue;
        }
        // 检查对应的 tool_use 是否在 compactable_tools 中
        let tool_name = msg.name.as_deref().unwrap_or("");
        if !config.compactable_tools.contains(tool_name) {
            continue;
        }
        // 已经被清除的跳过
        let content_str = msg.text_content().unwrap_or_default();
        if content_str == CLEARED_MESSAGE {
            continue;
        }
        let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("").to_string();
        compactable.push((msg_idx, tool_call_id, content_str.len()));
    }

    // Step 2: 检查是否需要清除
    if compactable.len() <= config.trigger_threshold {
        return MicroCompactResult { cleared_count: 0, freed_chars: 0 };
    }

    // Step 3: 保留最近 keep_recent 个，清除其余
    let to_clear_count = compactable.len().saturating_sub(config.keep_recent);
    let mut cleared_count = 0;
    let mut freed_chars = 0;

    for (msg_idx, _tool_call_id, content_len) in compactable.iter().take(to_clear_count) {
        if let Some(msg) = messages.get_mut(*msg_idx) {
            msg.set_content_text(CLEARED_MESSAGE.to_string());
            cleared_count += 1;
            freed_chars += content_len.saturating_sub(CLEARED_MESSAGE.len());
        }
    }

    MicroCompactResult { cleared_count, freed_chars }
}

/// 时间触发的 microCompact: 当距上次 assistant 回复超过阈值时执行。
/// 对应 cc 的 `maybeTimeBasedMicrocompact()`。
///
/// 场景：用户离开 5 分钟后回来，API prompt cache 已经过期，
/// 此时旧 tool results 反正要重新发送，不如清除节省 token。
pub fn maybe_time_based_microcompact(
    messages: &mut [ChatMessage],
    config: &MicroCompactConfig,
    now_millis: u64,
) -> MicroCompactResult {
    // 找到最后一条 assistant 消息的时间戳
    let last_assistant_time = messages.iter().rev()
        .find(|m| m.role == Role::Assistant)
        .and_then(|m| m.timestamp_millis);

    let Some(last_time) = last_assistant_time else {
        return MicroCompactResult { cleared_count: 0, freed_chars: 0 };
    };

    let gap_minutes = (now_millis.saturating_sub(last_time)) / 60_000;
    if gap_minutes < config.gap_threshold_minutes {
        return MicroCompactResult { cleared_count: 0, freed_chars: 0 };
    }

    // 缓存过期，执行更激进的清除：只保留 keep_recent 个
    microcompact_messages(messages, config)
}
```

**集成点**: 在 `crates/xiaolin-agent/src/runtime/query_engine.rs` 的 `query_model()` 入口处调用：

```rust
// 在发送 API 请求前
let mc_result = microcompact::microcompact_messages(&mut messages, &self.microcompact_config);
if mc_result.cleared_count > 0 {
    tracing::info!(
        cleared = mc_result.cleared_count,
        freed_chars = mc_result.freed_chars,
        "microcompact cleared old tool results"
    );
}
```

---

## 方案二：权限边界优化

### 2.1 扩展白名单路径

**文件**: `crates/xiaolin-agent/src/builtin_tools/filesystem.rs`

**新增函数** `allowed_external_paths()`:

```rust
/// Workspace 模式下额外允许访问的路径列表。
/// 这些路径即使在工作目录外也被认为是安全的。
fn allowed_external_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(home) = dirs::home_dir() {
        // Skills 安装/管理目录
        paths.push(home.join(".cursor").join("skills"));
        paths.push(home.join(".cursor").join("rules"));
        paths.push(home.join(".agents").join("skills"));
        paths.push(home.join(".codex").join("skills"));

        // XiaoLin 自身数据目录（session、memory 等）
        paths.push(home.join(".xiaolin"));
        paths.push(home.join(".local").join("share").join("xiaolin"));

        // 只读：包管理器缓存/元数据（读取 Cargo.toml 查依赖信息等）
        paths.push(home.join(".cargo").join("registry"));
        paths.push(home.join(".rustup").join("toolchains"));
    }

    // 临时目录（用于保存截断输出等）
    paths.push(std::env::temp_dir().join("xiaolin_truncated"));

    paths
}
```

**修改 `ensure_within_workspace`** — 在现有 state_dir 检查之后增加白名单检查：

```rust
// 现有代码之后，return Err(...) 之前：
// 3. 检查扩展白名单
for allowed in allowed_external_paths() {
    if let Ok(canonical_allowed) = allowed.canonicalize() {
        if resolved.starts_with(&canonical_allowed) {
            return Ok(resolved);
        }
    }
    // 对于不存在的目录也检查前缀匹配（如 ~/.cursor/skills/ 还未创建时）
    if resolved.starts_with(&allowed) {
        return Ok(resolved);
    }
}
```

### 2.2 改进错误消息

**改动 `ensure_within_workspace` 的错误文本**:

```rust
Err(std::io::Error::new(
    std::io::ErrorKind::PermissionDenied,
    format!(
        "Access denied: '{}' is outside the workspace '{}'.\n\
         \n\
         Allowed locations in Workspace mode:\n\
         - The workspace directory and its subdirectories\n\
         - Skills directories (~/.cursor/skills/, ~/.agents/skills/)\n\
         - XiaoLin data directory\n\
         \n\
         To resolve this:\n\
         1. Use a relative path within the workspace, OR\n\
         2. If this is a legitimate operation, ask the user to enable Full access mode\n\
         \n\
         [Tip: For skill installation, the target should be ~/.cursor/skills/ \
         or ~/.agents/skills/ which are already allowed]",
        path.display(),
        root.display()
    ),
))
```

### 2.3 (可选) 配置化白名单

在 `config/default.json` 中支持用户自定义：

```json
{
  "security": {
    "file_access_mode": "workspace",
    "additional_allowed_paths": [
      "~/.cursor/skills",
      "~/.config/my-tool"
    ]
  }
}
```

---

## 方案三：连续失败容错

> **现状**: XiaoLin 已有三层错误恢复机制：
> 1. `self-iter` feature（feature-gated）: `SelfIterEngine.diagnose_tool_failure_streak()` + `inject_tool_recovery_guidance()` — 诊断失败模式并注入恢复指导
> 2. Grace turn: `check_error_limit()` → 给模型最后一次机会
> 3. Length truncation guidance: 检测 finish_reason=length 并注入提示
>
> **问题**: `self-iter` 是 feature-gated（`#[cfg(feature = "self-iter")]`），不启用时只有 grace turn（无具体指导）。
> 方案应确保恢复指导**始终可用**而非 feature-gated。

### 3.1 ToolFailureTracker — 连续失败检测与方向引导（补充现有机制）

**策略**: 不需要从头实现，而是将 `self-iter` 的恢复逻辑**解耦为独立模块**，始终启用。

**新文件**: `crates/xiaolin-agent/src/runtime/failure_tracker.rs`

```rust
use std::collections::HashMap;

/// 追踪工具连续失败，超过阈值后注入 system-reminder 引导模型换方向。
/// 对应 cc 的行为：错误返回模型 + 如果模型不调整则注入提示。
pub struct ToolFailureTracker {
    /// 工具名 → (连续失败次数, 最后一次错误摘要)
    failures: HashMap<String, (u32, String)>,
    /// 同一工具连续失败多少次后注入 system-reminder
    pub reminder_threshold: u32,
    /// 全局连续失败次数（任何工具的任何失败）
    global_consecutive: u32,
    /// 全局阈值：超过后建议模型使用 ask_question
    pub global_threshold: u32,
}

impl Default for ToolFailureTracker {
    fn default() -> Self {
        Self {
            failures: HashMap::new(),
            reminder_threshold: 3,
            global_consecutive: 0,
            global_threshold: 5,
        }
    }
}

impl ToolFailureTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次工具失败。
    /// 返回 Some(reminder_text) 如果需要注入 system-reminder。
    pub fn record_failure(&mut self, tool_name: &str, error_summary: &str) -> Option<String> {
        self.global_consecutive += 1;

        let entry = self.failures
            .entry(tool_name.to_string())
            .or_insert((0, String::new()));
        entry.0 += 1;
        entry.1 = error_summary.chars().take(200).collect();

        // 全局阈值 — 建议 ask_question
        if self.global_consecutive >= self.global_threshold {
            return Some(format!(
                "<system-reminder>\n\
                 You have encountered {} consecutive tool failures across different tools. \
                 This suggests a systemic issue (e.g., wrong working directory, missing \
                 permissions, or incorrect assumptions about the environment).\n\
                 \n\
                 STOP retrying and instead:\n\
                 1. Analyze the pattern of failures\n\
                 2. Use ask_question to tell the user what's happening and ask for guidance\n\
                 3. Do NOT continue making failing tool calls\n\
                 </system-reminder>",
                self.global_consecutive
            ));
        }

        // 单工具阈值 — 建议换方向
        if entry.0 >= self.reminder_threshold {
            return Some(format!(
                "<system-reminder>\n\
                 The tool '{}' has failed {} consecutive times.\n\
                 Last error: {}\n\
                 \n\
                 You MUST try a different approach:\n\
                 - Use an alternative tool or method\n\
                 - If this is a path/permission issue, try a different path\n\
                 - If this is a search issue, broaden or change your query\n\
                 - As a last resort, use ask_question to ask the user for help\n\
                 \n\
                 Do NOT call '{}' again with the same or similar arguments.\n\
                 </system-reminder>",
                tool_name, entry.0, entry.1, tool_name
            ));
        }

        None
    }

    /// 记录一次工具成功。重置该工具和全局的计数。
    pub fn record_success(&mut self, tool_name: &str) {
        self.failures.remove(tool_name);
        self.global_consecutive = 0;
    }

    /// 重置所有状态（新会话或 compact 后调用）。
    pub fn reset(&mut self) {
        self.failures.clear();
        self.global_consecutive = 0;
    }
}
```

**集成点**: 在运行时工具执行循环中 (`runtime/mod.rs` line 596-598):

当前代码已有 `state.record_tool_error()` 和 `state.clear_error_streak()`。
ToolFailureTracker 可以作为 `QueryLoopState` 的增强，或者替代当前简单的计数器：

```rust
// 已有代码（runtime/mod.rs line 596）:
if !result.success {
    state.record_tool_error(&tool_name, &result.output);
    // 增强：当返回 Some(reminder) 时，注入 system 消息
    if let Some(reminder) = state.check_failure_guidance(&tool_name) {
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(reminder)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }
} else {
    state.clear_error_streak();
}
```

**注意**: 当前已有 `self-iter` feature 提供类似功能（`inject_tool_recovery_guidance`）。
可选方案：将 `self-iter` 的恢复指导逻辑 un-feature-gate，始终可用。

### 3.2 ~~PromptTooLong 自动触发 Compact~~ → 已确认已实现

**结论**: 已在 runtime loop 中完整实现 (`runtime/mod.rs` line 1029-1185):

1. **API 错误时** (line 1029): `is_prompt_too_long_error()` → `deps.reactive_compact(&messages)` → 恢复则 `continue` 重试
2. **流式中遇到** (line 1060): 暂存到 `withheld_prompt_too_long` → 流结束后 reactive compact → 恢复则 `continue`
3. **防无限循环**: `state.has_attempted_reactive_compact` flag 确保只尝试一次
4. **恢复失败**: 返回错误给客户端 `"prompt_too_long: recovery failed"`

底层由 `xiaolin_context::ContextPipeline::reactive_compact()` 实现（多级渐进压缩）。

**与方案设计的区别**: 当前实现只尝试一次 reactive compact。如果需要更强的恢复能力（如方案中的多级尝试 ImportanceBased → Aggressive → GiveUp），可扩展 `has_attempted_reactive_compact` 为计数器，但**优先级较低**因为当前单次恢复在实践中已足够。

---

## 实施计划（终版 — 经代码 review 修正）

| Day | 改动 | 文件 | 验证方式 |
|-----|------|------|----------|
| 1 | **read_file budget 豁免修复（1.1c 根因）** | `filesystem.rs` line 1283 | 一轮读 5 个大文件 → 验证 budget 生效、结果被持久化 |
| 1 | 权限白名单 + 错误消息改进 | `filesystem.rs` | 手动测试 `~/.cursor/skills/` 路径访问 |
| ~~1~~ | ~~ChatMessage 添加 `timestamp_millis` + `set_content_text()`~~ | ~~`xiaolin-core/src/types.rs`~~ | **已确认不需要**：现有 `time_based_microcompact` 使用 `iteration_boundaries`(Instant)，不依赖消息时间戳 |
| 2 | cc-style MicroCompact（如果 unified_compact 现有的不够强） | 评估 `microcompact_tool_results(messages, 3)` 是否足够 | 对比清除前后 token 占用 |
| 2 | SUMMARIZE_TOOL_RESULTS 提示词 + 行为心理学提示词 | `prompt_sections/mod.rs` | review prompt 输出含新指导项 |
| 3 | ToolFailureTracker：un-feature-gate `self-iter` 恢复指导 | `runtime/mod.rs`, `query_state.rs` | 不启用 self-iter 时也注入恢复指导 |

**不需要做的（已确认已实现）**:
- ~~Shell 限制降低~~ → 已有 800B 阈值 + `ok_split` terminal file 压缩，比 cc 的 30K 更激进
- ~~Per-message budget enforcement~~ → `enforce_per_message_budget()` 已在 runtime loop 调用（line 511, 988）
- ~~PromptTooLong auto-compact~~ → `reactive_compact()` 已自动触发（line 1029-1185）
- ~~ContextPipeline~~ → 4 层流水线（snip + importance micro + collapse + auto_compact）已完整运行
- ~~基本错误追踪~~ → `record_tool_error()` + grace turn 已存在

---

## 设计决策记录

1. **为什么不用 cc 的 cache_edits 机制？** cc 的 cached microcompact 依赖 Anthropic API 的 `cache_edits` 特性（服务端缓存编辑），这是 Anthropic 私有 API。XiaoLin 作为多 provider 网关，采用客户端侧清除（修改 message content）更通用。

2. **为什么 keep_recent = 5？** cc 的默认配置（通过 GrowthBook A/B 测试得出），保留最近 5 个 tool results 让模型保持对最新操作的记忆，同时大幅减少旧结果的 token 消耗。

3. **为什么不用 ask-user 确认替代硬拒？** 当前 XiaoLin 架构中 tool 执行是同步的，引入 ask-user 需要中断-恢复机制。v1 先用白名单方案，v2 考虑加入确认流程。

4. **时间触发清除的依据？** Anthropic 的 prompt cache 通常 5 分钟过期。过期后整个前缀要重新计算 token，此时清除旧内容不会额外损失 cache 命中率。

5. **cc-style microcompact 与现有 pipeline Layer 2 的关系？** 两者互补而非替代：
   - Pipeline Layer 2（importance-based eviction）在**消息数过多**时删除低分消息 → 适合极长会话
   - cc-style microcompact 在**工具结果过多**时清空旧内容保留结构 → 适合工具密集型会话
   - 执行顺序：先 cc-style microcompact（保留结构），再 pipeline（如果仍超限则删除消息）

6. **为什么 Shell 不需要进一步压缩？** XiaoLin 的 `TERMINAL_FILE_THRESHOLD = 800` + `ok_split` 机制已比 cc 的 30K 限制**更激进**。LLM 上下文中 shell 输出只有末尾 15 行摘要（~500 bytes），实际上下文膨胀来自 read_file（32K/次）和 search 结果。

---

## 附录：Prompt 内容层面的剩余差距

> 以上方案解决了"运行时机制"层面的问题。以下是"提示词内容"层面 cc 有而 XiaoLin 仍缺少的要素。

### A. 行为心理学指导（cc 已验证有效的模型行为调优）

XiaoLin 的 `doing_tasks` section 已有基本规则，但缺少 cc 经过大量 A/B 实验后加入的高阶行为指导：

| 指导项 | cc 原文摘要 | XiaoLin 现状 |
|--------|------------|---------------|
| **Assertiveness counterweight** | "If you notice the user's request is based on a misconception, or spot a bug adjacent to what they asked about, say so. You're a collaborator, not just an executor" | 缺失 |
| **False-claims mitigation** | "Report outcomes faithfully: if tests fail, say so... Never claim 'all tests pass' when output shows failures" | 缺失 |
| **Accountability without collapse** | "Take accountability for mistakes without collapsing into over-apology, self-abasement, or surrender. If the user pushes back repeatedly, stay steady and honest" | 只有 "Don't apologize excessively" |
| **Thoroughness counterweight** | "Before reporting a task complete, verify it actually works: run the test, execute the script, check the output" | 只有 "VERIFY YOUR WORK" 但更弱 |
| **Knowledge cutoff suppression** | "Don't proactively mention your knowledge cutoff date unless the user's message makes it directly relevant" | 缺失 |
| **SUMMARIZE_TOOL_RESULTS** | "When working with tool results, write down any important information you might need later in your response, as the original tool result may be cleared later" | 缺失 — 这对 microcompact 至关重要 |

**建议改动文件**: `crates/xiaolin-agent/src/runtime/prompt_sections/mod.rs`

在 `doing_tasks_en()` 末尾追加：

```rust
// 追加到 doing_tasks_en() 的 </making_code_changes> 之前

7. REPORT FAITHFULLY: Report outcomes honestly. If tests fail, say so with the relevant output. \
If you did not run a verification step, say that rather than implying it succeeded. Never claim \
\"all tests pass\" when output shows failures. Equally, when something works, state it plainly \
without unnecessary hedging.

8. BE A COLLABORATOR: If you notice the user's request is based on a misconception, or spot a \
bug adjacent to what they asked about, say so. You are a collaborator, not just an executor. \
Users benefit from your judgment, not just your compliance.

9. STAY STEADY: Take accountability for mistakes without excessive apology or surrender. If the \
user pushes back, acknowledge what went wrong and stay focused on solving the problem. Don't \
abandon a correct position just because the user is frustrated.

10. SAVE CONTEXT: When working with tool results, write down any important information you might \
need later in your response text, as old tool results may be automatically cleared to save \
context space.
```

中文版对应添加到 `doing_tasks_zh()`.

### B. Compact Prompt 模板质量

XiaoLin 当前的 `COMPRESSION_SYSTEM_PROMPT`（`context_compressor.rs` line 18）是一个简短的结构化模板（~160 words），只有 5 个字段：goal/facts/files/progress/errors。

cc 的 compact prompt（`src/services/compact/prompt.ts`）是**经过深度调优的 9 段结构**，有几个关键差异：

| 特性 | cc | XiaoLin |
|------|-------|----------|
| 段落数量 | 9 个详细段落 | 5 个简短字段 |
| `<analysis>` 中间推理 | 有（生成后 strip，提升摘要质量） | 无 |
| 用户消息保留 | "List ALL user messages that are not tool results" | 无单独要求 |
| 代码片段保留 | "include full code snippets where applicable" | 仅 "no code blocks" |
| Next Step 引用 | "include direct quotes from the most recent conversation" | 无 |
| NO_TOOLS 约束 | 硬性禁止摘要模型调用工具 | 无（可能浪费 turn） |
| Partial compact | 三种变体：Base / Partial(from) / Partial(up_to) | 仅一种全量模式 |

**建议**: 将 `COMPRESSION_SYSTEM_PROMPT` 升级为更详细的模板，参考 cc 的 9 段结构。特别重要的是：
1. 加入 `<analysis>` 中间推理步骤
2. 显式保留用户消息列表
3. 加入 "Next Step" + 原文引用防漂移
4. 加入 NO_TOOLS 约束防止摘要模型调用工具

### C. OutputStyle / Persona Overlay

cc 支持 `OutputStyleConfig`：用户可以自定义一个 "output style"（如 "terse"、"verbose"、"formal"），overlay 到系统 prompt 上。这允许：
- 不同用户有不同的交互风格偏好
- `keepCodingInstructions` 开关决定是否保留编程指导
- 可以完全替换 `doing_tasks` section 用自定义的

XiaoLin 当前仅有 `language_preference`（中/英切换）。

**建议**: 在 `AgentConfig` 中新增 `output_style` 字段：

```rust
pub struct OutputStyleConfig {
    pub name: String,
    pub prompt: String,            // 附加到系统 prompt 的自定义指令
    pub keep_coding_instructions: bool,  // 是否保留 doing_tasks section
}
```

### D. Scratchpad Directory 机制

cc 有 `getScratchpadInstructions()` — 为每个 session 创建一个临时目录，引导模型使用该目录存放中间文件（而非 /tmp 或用户项目目录），避免污染工作区。

XiaoLin 当前使用 `std::env::temp_dir().join("xiaolin_truncated")` 做截断输出保存，但没有系统性的 scratchpad 机制告知模型。

**建议**: 低优先级。可在 `environment_section` 中追加 scratchpad 路径说明。

### E. DiscoverSkills 自动发现

cc 有 `EXPERIMENTAL_SKILL_SEARCH` feature — 每轮自动浮出相关 skills 作为 "Skills relevant to your task:" 提示，并提供 `DiscoverSkills` 工具让模型主动搜索更多。

XiaoLin 的 skill 系统（`xiaolin-evolution`）已有 `skill_store.rs`，但缺少：
- 每轮自动浮出相关 skills 的机制
- 给模型的 skill discovery 指导提示词

**建议**: 中优先级。在 `session_guidance` 中加入：
```
If a skill file was surfaced as relevant to your task, follow its instructions.
Use tool_search with "skill:" prefix to find specialized skills for unfamiliar workflows.
```

---

## 优先级总排序（修正版）

| 优先级 | 项目 | 影响 |
|--------|------|------|
| **P0** | **方案一：read_file budget 豁免修复（1.1c）** | **真正根因** — 一行改动 `usize::MAX` → `32_768` 即可让 read_file 参与 budget |
| P0 | 方案二：权限白名单 | 用户体验直接卡住 |
| P0 | 附录A：SUMMARIZE_TOOL_RESULTS 指导 | 搭配已有 microcompact 防信息丢失 |
| P1 | 方案一：评估现有 microcompact 是否足够（1.3） | unified_compact 已有 `microcompact_tool_results(messages, 3)`，可能只需调参 |
| P2 | 方案三：ToolFailureTracker（un-feature-gate） | 改善错误恢复（基础已有，需解除 feature gate） |
| P2 | 附录A：行为心理学提示词 | 提升模型行为质量 |
| P2 | 附录B：Compact Prompt 升级 | 提升摘要质量 |
| P3 | 附录C：OutputStyle | 用户定制性 |
| P3 | 附录D：Scratchpad | 文件管理规范 |
| P3 | 附录E：DiscoverSkills | skill 生态 |
| ✅ | ~~方案一：Shell 限制降低（1.1）~~ | **已确认无需修改** — shell 已有 800B 阈值压缩 |
| ✅ | ~~方案一：per-message enforce（1.2）~~ | **已实现** — `enforce_per_message_budget()` 已在 runtime 调用 |
| ✅ | ~~方案三：PromptTooLong auto-compact（3.2）~~ | **已实现** — `reactive_compact()` 已自动触发 |
