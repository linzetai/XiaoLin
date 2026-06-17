# MCP 工具接入 Deferred 管线 — 详细技术方案

> 更新于 2026-06-15，基于 XiaoLin / Codex / Claude Code 三方深度对比分析。

## 三方 Deferred Loading 对比

| 维度 | XiaoLin (当前) | Codex | Claude Code |
|------|---------------|-------|-------------|
| **默认策略** | 全量 eager ❌ | 阈值 ≥100 工具 defer | MCP 100% defer |
| **基础设施** | ✅ 完整（BM25 + activate） | ✅ 完整 | ✅ 完整 + API `tool_reference` |
| **MCP 接入** | ❌ 未接入 | ✅ 全面 | ✅ 全面 |
| **Opt-out** | N/A | 显式启用 connector direct | `_meta['anthropic/alwaysLoad']` |
| **搜索索引字段** | name+desc+hint | name+desc+title+schema+connector | name+searchHint+desc |
| **名称通知** | `<deferred_tools>` 仅数量 | ToolSearch prompt | Delta attachment 或 prepend 列表 |
| **动态激活** | `activate_deferred` | `defer_loading:true` spec | `tool_reference` block |
| **双重注入** | ❌ 有浪费 | 无 | 无 |

### 关键设计差异

**Codex** 采用**阈值触发**：`DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD = 100`，低于阈值全部 direct，高于阈值除显式启用的 connector 外全部 deferred。Feature flag `ToolSearchAlwaysDeferMcpTools`（默认 off）可强制全部 defer。

**Claude Code** 采用**类型判定**：`isMcp === true → defer`（无阈值），`alwaysLoad === true → 不 defer`（优先级最高）。额外 `tst-auto` 模式按 token 阈值（context window × 10%）自动开关。`tool_reference` API 让激活后的工具 schema 由 Anthropic 服务端注入，无需客户端管理。

**XiaoLin 推荐策略**：结合两家做法，阈值触发 + alwaysLoad opt-out + Delta 名称通知。

## 现有基础设施盘点

XiaoLin 已有**完整的 deferred tool loading 基础设施**（~70%），只是 MCP 工具没有接入。

### ToolRegistry (xiaolin-core/src/tool.rs)

| API | 功能 | MCP 是否使用 |
|-----|------|-------------|
| `register(tool)` | 注册为 eager（LLM 直接可见） | ✅ 当前用这个 |
| `register_deferred(tool)` | 注册到 deferred set（LLM 不可见，需搜索） | ❌ 应改用这个 |
| `eager_definitions()` | 返回非 deferred 工具定义列表 | 间接使用 |
| `search_deferred(query)` | BM25 搜索 deferred 工具（匹配 name+desc+hint） | 不涉及 |
| `activate_deferred(name)` | 将 deferred 工具提升为 eager | 不涉及 |
| `deferred_count()` | deferred 工具总数 | 不涉及 |

### ToolSearchTool (xiaolin-agent/src/builtin_tools/tool_search.rs)

- 注册为 **eager**（始终可见）
- 两种模式：`{"query": "keyword"}` 搜索 / `{"query": "select:tool_name"}` 激活
- 搜索使用 BM25 算法（name + description + search_hint）
- 激活后工具立即可用（同一轮对话即可调用）

### Prompt Engine (xiaolin-agent/src/runtime/prompt_sections/)

已有完整的 deferred 提示：
- `system_section`：当 `deferred_tool_count > 0` 时注入 `<deferred_tools>` 标签
- `using_tools_section`：提示 LLM 使用 `tool_search` 发现额外工具

### 当前 MCP 双重 token 浪费

```
路径 1: register → eager_definitions() → 发给 LLM 的 tools 列表
路径 2: inject_mcp_tools_prompt → system prompt [MCP Extensions] 段落

= 同一批 MCP 工具的 schema 被发送两次
```

### 隐性 Bug（接入前必须修复）

| Bug | 位置 | 影响 |
|-----|------|------|
| `unregister_by_prefix` 不清理 `deferred` set | `tool.rs` | MCP 热重载后 deferred set 残留幽灵条目，search 返回已断开 server 的工具 |
| `activate_deferred` 后 `tool_defs` 不刷新 | `runtime/turn_setup.rs` | 模型 activate 后下一轮 LLM 调用仍用旧 tool list，工具不可见 |
| `filtered_tool_definitions` 用 `definitions()` 不过滤 deferred | `routes/common.rs` | token 预算统计偏高，前端展示含 deferred 工具 |

## 接入方案

### 变更 0 (前置): Bug 修复

**0a. `unregister_by_prefix` 同步清理 deferred set**

```rust
// xiaolin-core/src/tool.rs — unregister_by_prefix
pub fn unregister_by_prefix(&self, prefix: &str) {
    let mut tools = self.tools.write().unwrap();
    tools.retain(|name, _| !name.starts_with(prefix));
    // 新增：同步清理 deferred set
    let mut deferred = self.deferred.write().unwrap();
    deferred.retain(|name| !name.starts_with(prefix));
    self.bump_version();
}
```

**0b. `activate_deferred` 后刷新 tool_defs**

```rust
// xiaolin-agent/src/runtime/ — turn iteration 或 post_tool_processing
if registry.version_changed_since(last_version) {
    svc.tool_defs = rebuild_tool_defs(registry, profile, config);
    last_version = registry.version();
}
```

**0c. `filtered_tool_definitions` 改用 `eager_definitions`**

```rust
// xiaolin-gateway/src/routes/common.rs
let tool_defs = registry.eager_definitions();  // 替换 definitions()
```

### 变更 1: `register_mcp_tools` 改为条件 deferred 注册

```rust
// xiaolin-mcp/src/lib.rs — connect_mcp_server（统一入口，T4 后）
//
// 签名约定（跨 spec 统一）：
//   prefix 由函数内部通过 naming::mcp_server_prefix(&cfg.id) 派生，调用方无需传入。
//   deferred 参数控制工具注册到 eager 还是 deferred set。
pub async fn connect_mcp_server(
    cfg: &McpServerConfig,
    registry: &ToolRegistry,
    deferred: bool,
) -> anyhow::Result<SharedMcpClient> {
    let prefix = naming::mcp_server_prefix(&cfg.id);
    // ... connect + tools/list ...
    for tool in &tools {
        let bridge = McpToolBridge::new(tool, shared.clone(), &prefix);
        let always_load = bridge.always_load;
        if deferred && !always_load {
            registry.register_deferred(Arc::new(bridge));
        } else {
            registry.register(Arc::new(bridge));
        }
    }
    // ...
}
```

调用方决定 deferred 时机（见变更 3）。

### 变更 2: McpToolBridge 增加 search_hint

```rust
// xiaolin-mcp/src/lib.rs
struct McpToolBridge {
    // ... existing fields ...
    search_hint: String,  // 新增：server_id + 原始 tool name + 可能的 annotations
}

impl Tool for McpToolBridge {
    fn search_hint(&self) -> &str {
        &self.search_hint
    }
}
```

`search_hint` 组成：`"{server_id} {original_tool_name} {annotation_keywords}"`

这让 BM25 搜索能通过 server 名或原始工具名（不带前缀）匹配。

### 变更 3: Gateway 层阈值决策（更新：分层策略）

综合 Codex（阈值 100）和 Claude Code（10% context window + 默认 defer MCP）的做法：

```rust
// xiaolin-gateway/src/state/mod.rs

/// 分层阈值策略
fn should_defer_mcp_tools(
    total_mcp_tools: usize,
    total_mcp_desc_tokens: usize,
    context_window: usize,
) -> bool {
    // Layer 1: 工具数阈值（对标 Codex DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD）
    if total_mcp_tools > MCP_DEFERRED_THRESHOLD {
        return true;
    }
    // Layer 2: token 占比阈值（对标 Claude Code getAutoToolSearchTokenThreshold）
    if context_window > 0 {
        let threshold = context_window / 10;  // 10%
        if total_mcp_desc_tokens > threshold {
            return true;
        }
    }
    false
}

const MCP_DEFERRED_THRESHOLD: usize = 100;
const CHARS_PER_TOKEN: f64 = 2.5;  // Claude Code 同款估算
```

```rust
// register_mcp_and_subagent_tools — 两阶段策略

// 阶段 1：并行连接所有 MCP server（deferred=false 先 eager 注册），同时收集统计
let all_clients = connect_all_mcp_servers(&to_connect, &registry, /* deferred */ false).await;

let total_mcp_tools: usize = /* 从 registry 统计 mcp__ 前缀工具数 */;
let total_mcp_desc_chars: usize = /* 遍历工具 description 总字符数 */;
let total_mcp_desc_tokens = (total_mcp_desc_chars as f64 / CHARS_PER_TOKEN) as usize;

let should_defer = should_defer_mcp_tools(total_mcp_tools, total_mcp_desc_tokens, context_window);

// 阶段 2：如果需要 defer，将已注册的 MCP 工具降级为 deferred（不重新连接）
if should_defer {
    for (id, _) in &all_clients {
        let prefix = naming::mcp_server_prefix(id);
        // 将 eager MCP 工具移入 deferred set（always_load 的除外）
        registry.demote_to_deferred_by_prefix(&prefix, /* except_always_load */ true);
    }
}
```

> **注意**：阶段 2 不重新连接 server，只是将已注册工具从 eager 移到 deferred set。
> `demote_to_deferred_by_prefix` 是需要新增的 `ToolRegistry` 方法。

### 变更 4: `inject_mcp_tools_prompt` 条件化

```rust
// xiaolin-gateway/src/chat_pipeline.rs

fn inject_mcp_tools_prompt(state: &AppState, messages: &mut Vec<ChatMessage>) {
    // 当 deferred 管线启用时，不注入 [MCP Extensions] prompt
    // 因为 prompt engine 已经有 <deferred_tools> 标签 + tool_search 指引
    if state.rt.tool_registry.deferred_count() > 0 {
        // deferred 模式：仅注入少量 eager MCP 工具（alwaysLoad）
        let eager_mcp = state.rt.tool_registry.eager_mcp_definitions();
        if eager_mcp.is_empty() {
            return;  // 全部 deferred，prompt engine 会处理
        }
        // 只注入 eager 部分（数量很少）
        inject_partial_mcp_prompt(eager_mcp, messages);
        return;
    }

    // 非 deferred 模式：走现有全量注入逻辑
    let mcp_tools = state.rt.tool_registry.mcp_definitions();
    // ... 现有逻辑 ...
}
```

### 变更 5: `alwaysLoad` 元数据支持

MCP spec 允许 `tools/list` 响应中携带 `_meta` 字段。但当前 XiaoLin 的 `McpTool` 结构体
没有 `_meta` 字段，需要先扩展：

```rust
// xiaolin-mcp/src/lib.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,  // 新增
}
```

MCP server 可在响应中标记关键工具：

```json
{
  "name": "run_query",
  "description": "Execute SQL query",
  "_meta": {
    "anthropic/alwaysLoad": true
  }
}
```

`McpToolBridge` 解析此字段：
```rust
struct McpToolBridge {
    // ... existing fields ...
    always_load: bool,  // 新增
}

impl McpToolBridge {
    fn new(mcp_tool: &McpTool, client: SharedMcpClient, server_prefix: &str) -> Self {
        let always_load = mcp_tool.meta
            .as_ref()
            .and_then(|m| m.get("anthropic/alwaysLoad"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Self { /* ..., */ always_load }
    }
}
```

阈值决策时 `always_load` 工具保持 eager。

## 数据流图

```
MCP Server A (5 tools)  ─┐
MCP Server B (3 tools)   ├── connect + tools/list ──→ total = 8 < 100
MCP Server C (2 tools)  ─┘                              ↓
                                                   all eager (register)
                                                        ↓
                                            inject_mcp_tools_prompt ✅
                                            <deferred_tools> 段落 ❌ (count=0)


MCP Server A (50 tools) ─┐
MCP Server B (40 tools)  ├── connect + tools/list ──→ total = 110 > 100
MCP Server C (20 tools) ─┘                              ↓
                                                   ╔══════════════════╗
                                                   ║ alwaysLoad? → eager ║
                                                   ║ 其他全部 → deferred ║
                                                   ╚══════════════════╝
                                                        ↓
                                            inject_mcp_tools_prompt → 仅 eager 部分
                                            <deferred_tools> 段落 ✅ (count=107)
                                            tool_search 可搜索 deferred MCP 工具
```

## LLM 交互示例

### 场景：用户配了 20 个 MCP server，共 150 个工具

**System prompt 包含：**
```
<deferred_tools>
There are 148 additional tools not listed in your current tool set.
These are specialized tools available on demand. Use the `tool_search` tool
with a descriptive query to discover and access them when needed.
</deferred_tools>
```

**LLM 需要查数据库时：**
```json
{"name": "tool_search", "arguments": {"query": "database query sql"}}
```

**ToolSearch 返回：**
```json
{
  "matches": [
    {"name": "mcp__postgres__run_query", "description": "Execute a SQL query against the database"},
    {"name": "mcp__postgres__list_tables", "description": "List all tables in the schema"}
  ],
  "match_count": 2,
  "total_deferred_tools": 148
}
```

**LLM 激活：**
```json
{"name": "tool_search", "arguments": {"query": "select:mcp__postgres__run_query"}}
```

**激活确认后，LLM 直接调用：**
```json
{"name": "mcp__postgres__run_query", "arguments": {"query": "SELECT * FROM users LIMIT 10"}}
```

### 变更 6 (增强): `<deferred_tools>` 增加工具名列表

当前 XiaoLin 的 `<deferred_tools>` 仅提示数量：

```
There are 148 additional tools not listed...
```

对标 Claude Code 的 `<available-deferred-tools>` / Delta attachment，应增加工具名列表，让模型知道具体有哪些工具可搜索：

```rust
// xiaolin-agent/src/runtime/prompt_sections/mod.rs

let deferred_note = if ctx.deferred_tool_count > 0 {
    let deferred_names = ctx.tool_registry.deferred_tool_names();
    let names_list = deferred_names.iter()
        .take(200)  // 安全上限
        .map(|n| format!("  - {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\n<deferred_tools>\n\
         There are {} additional tools available via `tool_search`:\n\
         {names_list}\n\
         Use `tool_search` with a keyword query or `select:tool_name` to activate.\n\
         </deferred_tools>",
        ctx.deferred_tool_count
    )
} else { String::new() };
```

需要 `ToolRegistry` 新增 `deferred_tool_names() -> Vec<String>` 方法。

> **未来优化（T32 Instructions Delta）**：改为 Delta 方式增量注入，避免每轮都发送完整列表破坏 prompt cache。

## 不变量

1. **Deferred 工具仍可通过 `registry.get()` 执行** — 即使未激活，dispatcher 也能找到并执行（ToolRegistry 设计如此）
2. **ToolSearchTool 始终 eager** — 它自己永远不会被 deferred
3. **阈值看 MCP 工具总数 + description token 占比** — 内置工具不受影响
4. **Session 内状态** — `activate_deferred` 只对当前 session 有效（registry 是 session 级的）
5. **`always_load` 工具始终 eager** — 即使超过阈值也不 defer（对标 Claude Code）

## 影响的文件

| 文件 | 变更 | 估算行数 |
|------|------|---------|
| `xiaolin-mcp/src/lib.rs` | `McpToolBridge` 增加 `search_hint` + `always_load`；`connect_mcp_server` 增加 `deferred` 参数 | ~80 |
| `xiaolin-gateway/src/state/mod.rs` | 分层阈值决策逻辑 + 传入 deferred flag | ~40 |
| `xiaolin-gateway/src/chat_pipeline.rs` | `inject_mcp_tools_prompt` 条件化 | ~30 |
| `xiaolin-core/src/tool.rs` | `unregister_by_prefix` 清理 deferred set（Bug 修复）+ `eager_mcp_definitions()` + `deferred_tool_names()` | ~25 |
| `xiaolin-agent/src/runtime/` | `activate_deferred` 后刷新 `tool_defs`（Bug 修复） | ~25 |
| `xiaolin-agent/src/runtime/prompt_sections/` | `<deferred_tools>` 增加工具名列表 | ~20 |
| `xiaolin-gateway/src/mcp_tool.rs` | `do_reload` 传 deferred flag | ~15 |
| `xiaolin-gateway/src/routes/common.rs` | `filtered_tool_definitions` 改用 `eager_definitions`（Bug 修复） | ~10 |
| 测试 | 单元 + 集成 | ~120 |
| **合计** | | **~365** |

## 测试计划

1. **单测**：`register_deferred` 后 MCP 工具不出现在 `eager_definitions()`
2. **单测**：`search_deferred("server_name")` 能通过 search_hint 匹配 MCP 工具
3. **单测**：`activate_deferred("mcp__server__tool")` 后工具出现在 eager 列表
4. **单测**：阈值 100 以下全 eager，100 以上全 deferred（除 alwaysLoad）
5. **单测**：10% context window token 阈值触发 defer
6. **单测**：`unregister_by_prefix` 同步清理 deferred set（Bug 修复验证）
7. **集成测试**：deferred 模式下 `inject_mcp_tools_prompt` 不注入全量列表
8. **集成测试**：`<deferred_tools>` 包含工具名列表
9. **E2E**：配 100+ MCP 工具，验证 LLM 能通过 tool_search 发现并使用
10. **E2E**：MCP server 热重载后 deferred set 无残留幽灵条目
