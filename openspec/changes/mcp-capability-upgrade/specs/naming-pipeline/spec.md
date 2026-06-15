# MCP 工具名规范化管线 — 详细技术方案

## 现状分析

### 当前命名格式

```
mcp_{server_id}_{tool_name}
```

Prefix 由调用方构造：`format!("mcp_{}_", cfg.id)`

### 当前已知 Bug

**单下划线分隔导致歧义**：当 `server_id` 包含 `_` 时（如 `chrome_devtools`），解析失败。

前端 `StepIndicator.tsx` / `ToolCallCard.tsx` 的解析逻辑：

```typescript
const rest = name.slice(4);           // 去掉 "mcp_"
const idx = rest.indexOf("_");        // 找第一个 _
const serverId = idx >= 0 ? rest.slice(0, idx) : rest;
const toolName = idx >= 0 ? rest.slice(idx + 1) : "";
```

示例：`mcp_chrome_devtools_read_console`
- 期望：serverId = `chrome_devtools`, toolName = `read_console`
- 实际：serverId = `chrome`, toolName = `devtools_read_console` ❌

后端 `chat_pipeline.rs` 有同样的 Bug：

```rust
let after_prefix = name.strip_prefix("mcp_").unwrap_or(name);
let server_id = if let Some(idx) = after_prefix.find('_') {
    &after_prefix[..idx]
} else {
    after_prefix
};
```

### 所有受影响的代码点

| # | 文件 | 位置 | 用途 | 改动 |
|---|------|------|------|------|
| 1 | `xiaolin-gateway/src/state/mod.rs` | `format!("mcp_{}_", id)` (5 处) | prefix 构造 | → `mcp__{id}__` |
| 2 | `xiaolin-gateway/src/mcp_tool.rs` | `format!("mcp_{}_", cfg.id)` (4 处) | prefix 构造 | → `mcp__{id}__` |
| 3 | `xiaolin-mcp/src/lib.rs` | `format!("{server_prefix}{}", tool.name)` | 工具名组装 | 增加 sanitize |
| 4 | `xiaolin-core/src/tool.rs` | `starts_with("mcp_")` in `mcp_definitions` | 过滤 MCP 工具 | → `starts_with("mcp__")` |
| 5 | `xiaolin-gateway/src/chat_pipeline.rs` | `strip_prefix("mcp_")` + `find('_')` | 解析 server_id | → `strip_prefix("mcp__")` + `split("__")` |
| 6 | `xiaolin-agent/src/subagent.rs` | `starts_with("mcp_")` | 子代理工具过滤 | → `starts_with("mcp__")` |
| 7 | `xiaolin-agent/src/runtime/tool_executor.rs` | `starts_with("mcp_")` | retention tier | → `starts_with("mcp__")` |
| 8 | `xiaolin-core/src/agent_config.rs` | `mcp_*` glob in tool_pattern_matches | 权限匹配 | → `mcp__*` |
| 9 | `xiaolin-app/.../StepIndicator.tsx` | `getMcpMeta()` + `startsWith("mcp_")` | UI 显示 | → `mcp__` + `split("__")` |
| 10 | `xiaolin-app/.../ToolCallCard.tsx` | `getMcpMeta()` + `startsWith("mcp_")` | UI 显示 | → `mcp__` + `split("__")` |
| 11 | `xiaolin-app/.../ToolCallCard.test.tsx` | 测试 mock | 数据 | → `mcp__` 格式 |

## 新命名格式

### 格式

```
mcp__{sanitized_server_id}__{sanitized_tool_name}
```

双下划线 `__` 分隔，与 Claude Code 和 Codex 完全一致。

### 为什么用双下划线

1. **消除歧义**：server_id 和 tool_name 内部允许单下划线 `_`，双下划线 `__` 作为唯一分隔符
2. **行业标准**：Claude Code 和 Codex 均采用 `mcp__server__tool` 格式
3. **向后兼容**：用户极少手动输入工具名，迁移成本低

### Sanitization 规则

参考 Codex 和 Claude Code，工具名需要匹配 LLM API 的 `^[a-zA-Z0-9_-]+$` 约束。

```rust
// xiaolin-mcp/src/naming.rs（新文件）

/// MCP 工具名分隔符
pub const MCP_DELIMITER: &str = "__";
pub const MCP_PREFIX: &str = "mcp";

/// Sanitize name for LLM API compatibility.
/// Replaces any character not in [a-zA-Z0-9_-] with '_'.
/// Matches Claude Code's `normalizeNameForMCP` and Codex's `sanitize_responses_api_tool_name`.
pub fn sanitize_for_api(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            result.push(c);
        } else {
            result.push('_');
        }
    }
    if result.is_empty() {
        "_".to_string()
    } else {
        result
    }
}

/// Build the full MCP prefix for a server: `mcp__{sanitized_id}__`
pub fn mcp_server_prefix(server_id: &str) -> String {
    format!(
        "{}{}{}{}{}",
        MCP_PREFIX,
        MCP_DELIMITER,
        sanitize_for_api(server_id),
        MCP_DELIMITER,
        ""
    )
}

/// Build fully qualified MCP tool name: `mcp__{server_id}__{tool_name}`
pub fn mcp_tool_name(server_id: &str, tool_name: &str) -> String {
    format!(
        "{}{}",
        mcp_server_prefix(server_id),
        sanitize_for_api(tool_name)
    )
}

/// Parse a fully qualified MCP tool name back into (server_id, tool_name).
/// Returns None if the name doesn't match the MCP format.
pub fn parse_mcp_tool_name(full_name: &str) -> Option<(&str, &str)> {
    let rest = full_name.strip_prefix("mcp__")?;
    let idx = rest.find("__")?;
    let server_id = &rest[..idx];
    let tool_name = &rest[idx + 2..];
    if server_id.is_empty() || tool_name.is_empty() {
        return None;
    }
    Some((server_id, tool_name))
}

/// Check if a name is an MCP tool name
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp__")
}
```

### 前端对应工具函数

```typescript
// lib/mcpNaming.ts（新文件）

export const MCP_DELIMITER = "__";
export const MCP_PREFIX = "mcp";

/** Sanitize name for API compatibility: [a-zA-Z0-9_-] */
export function sanitizeForApi(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_") || "_";
}

/** Build full prefix: `mcp__{sanitizedId}__` */
export function mcpServerPrefix(serverId: string): string {
  return `${MCP_PREFIX}${MCP_DELIMITER}${sanitizeForApi(serverId)}${MCP_DELIMITER}`;
}

/** Parse `mcp__{serverId}__{toolName}` → { serverId, toolName } | null */
export function parseMcpToolName(fullName: string): { serverId: string; toolName: string } | null {
  if (!fullName.startsWith("mcp__")) return null;
  const rest = fullName.slice(5); // "mcp__".length
  const idx = rest.indexOf("__");
  if (idx < 0) return null;
  const serverId = rest.slice(0, idx);
  const toolName = rest.slice(idx + 2);
  if (!serverId || !toolName) return null;
  return { serverId, toolName };
}

/** Check if name is an MCP tool */
export function isMcpTool(name: string): boolean {
  return name.startsWith("mcp__");
}
```

## 三方对比

| 特性 | XiaoLin (当前) | XiaoLin (新) | Claude Code | Codex |
|------|---------------|-------------|-------------|-------|
| 分隔符 | `_` (单) | `__` (双) | `__` (双) | `__` (双) |
| 格式 | `mcp_{id}_{tool}` | `mcp__{id}__{tool}` | `mcp__{id}__{tool}` | `mcp__{id}__{tool}` |
| Sanitize | ❌ 无 | ✅ `[^a-zA-Z0-9_-]` → `_` | ✅ `[^a-zA-Z0-9_-]` → `_` | ✅ `[^a-zA-Z0-9_]` → `_` |
| Hash 去重 | ❌ 仅 skip | 🔜 Phase 2 | ❌ 无 | ✅ SHA1 12-char suffix |
| 长度限制 | ❌ 无 | 🔜 Phase 2 (64 bytes) | ❌ 无 | ✅ 64 bytes |
| 集中定义 | ❌ 散落各处 | ✅ `naming.rs` + `lib/mcpNaming.ts` | ✅ `mcpStringUtils.ts` | ✅ `tools.rs` |

### P0 范围（本次）

1. 双下划线分隔 + sanitize（修复解析 Bug）
2. 集中到 `naming.rs` / `mcpNaming.ts`
3. 更新所有 11 个代码点

### P2 范围（未来）

1. Hash 去重（当 sanitize 后两个不同原名产生相同结果时）
2. 64 字节长度限制（当 server_id + tool_name 超长时截断 + hash）

## 数据流

```
用户配置:
  id: "chrome-devtools"
  
        ↓ sanitize_for_api
        
  sanitized: "chrome-devtools"   (已合法，不变)
  
        ↓ mcp_server_prefix
        
  prefix: "mcp__chrome-devtools__"
  
        ↓ + tool.name ("read_console")
        
  full_name: "mcp__chrome-devtools__read_console"
```

```
用户配置:
  id: "my.server/v2"
  
        ↓ sanitize_for_api
        
  sanitized: "my_server_v2"   (. 和 / → _)
  
        ↓ mcp_server_prefix
        
  prefix: "mcp__my_server_v2__"
  
        ↓ + tool.name ("query.data")
                         ↓ sanitize_for_api: "query_data"
        
  full_name: "mcp__my_server_v2__query_data"
```

## 迁移策略

### 向后兼容

工具名变更不需要迁移用户数据，因为：
1. 工具名是 **运行时生成** 的（非持久化到配置）
2. 每次启动/重连都会重新构造工具名
3. 用户在 `tools_allow`/`tools_deny` 中的 glob 规则需要更新

### 权限规则迁移

`agent_config.rs` 中 `tools_ask` 等规则从 `mcp_*` 改为 `mcp__*`。

这是**自动兼容**的：
- 旧规则 `mcp_*` 会匹配 `mcp__server__tool`（`mcp_` 是 `mcp__` 的前缀）
- 但新规则 `mcp__*` 更精确，避免误匹配非 MCP 工具

建议：在 `tool_pattern_matches` 中加兼容层，`mcp_*` 自动视为 `mcp__*`，并发出 deprecation warning。

## 变更详情

### 变更 1: 新增 `naming.rs`

在 `xiaolin-mcp/src/` 中新增 `naming.rs`，包含上述所有 naming 函数。
导出路径：`xiaolin_mcp::naming::*`

### 变更 2: `McpToolBridge::new` 使用 sanitize

```rust
impl McpToolBridge {
    fn new(mcp_tool: &McpTool, client: SharedMcpClient, server_prefix: &str) -> Self {
        let sanitized_name = naming::sanitize_for_api(&mcp_tool.name);
        Self {
            tool_name: format!("{server_prefix}{sanitized_name}"),
            // ...
        }
    }
}
```

### 变更 3: 所有 prefix 构造统一

将 11 个 `format!("mcp_{}_", id)` 全部替换为 `xiaolin_mcp::naming::mcp_server_prefix(&id)`。

### 变更 4: 后端解析统一

`chat_pipeline.rs` 的 `inject_mcp_tools_prompt`：

```rust
// Before:
let after_prefix = name.strip_prefix("mcp_").unwrap_or(name);
let server_id = if let Some(idx) = after_prefix.find('_') { ... };

// After:
if let Some((server_id, _tool_name)) = xiaolin_mcp::naming::parse_mcp_tool_name(name) {
    servers.entry(server_id.to_string()).or_default().push(...);
}
```

### 变更 5: 前端解析统一

`StepIndicator.tsx` / `ToolCallCard.tsx` 的 `getMcpMeta`：

```typescript
// Before:
function getMcpMeta(name: string) {
  if (!name.startsWith("mcp_")) return null;
  const rest = name.slice(4);
  const idx = rest.indexOf("_");
  // ... buggy parsing ...
}

// After:
import { parseMcpToolName } from "@/lib/mcpNaming";

function getMcpMeta(name: string) {
  const parsed = parseMcpToolName(name);
  if (!parsed) return null;
  return { icon: <Plug />, label: `${parsed.serverId}/${parsed.toolName}` };
}
```

### 变更 6: `mcp_definitions` 过滤更新

```rust
// Before:
pub fn mcp_definitions(&self) -> Vec<ToolDefinition> {
    all.iter().filter(|td| td.function.name.starts_with("mcp_")).cloned().collect()
}

// After:
pub fn mcp_definitions(&self) -> Vec<ToolDefinition> {
    all.iter().filter(|td| td.function.name.starts_with("mcp__")).cloned().collect()
}
```

### 变更 7: subagent/tool_executor 过滤更新

```rust
// subagent.rs: name.starts_with("mcp_")  → name.starts_with("mcp__")
// tool_executor.rs: tool_name.starts_with("mcp_")  → tool_name.starts_with("mcp__")
```

## 测试计划

### 单测（naming.rs）

```rust
#[test]
fn sanitize_preserves_valid_chars() {
    assert_eq!(sanitize_for_api("hello_world-123"), "hello_world-123");
}

#[test]
fn sanitize_replaces_invalid_chars() {
    assert_eq!(sanitize_for_api("my.server/v2"), "my_server_v2");
    assert_eq!(sanitize_for_api("name with spaces"), "name_with_spaces");
}

#[test]
fn sanitize_empty_string() {
    assert_eq!(sanitize_for_api(""), "_");
}

#[test]
fn mcp_server_prefix_format() {
    assert_eq!(mcp_server_prefix("chrome-devtools"), "mcp__chrome-devtools__");
    assert_eq!(mcp_server_prefix("my.server"), "mcp__my_server__");
}

#[test]
fn mcp_tool_name_format() {
    assert_eq!(
        mcp_tool_name("chrome-devtools", "read_console"),
        "mcp__chrome-devtools__read_console"
    );
}

#[test]
fn parse_roundtrip() {
    let full = mcp_tool_name("server", "tool");
    let (s, t) = parse_mcp_tool_name(&full).unwrap();
    assert_eq!(s, "server");
    assert_eq!(t, "tool");
}

#[test]
fn parse_with_underscores_in_ids() {
    let full = "mcp__chrome_devtools__read_console";
    let (s, t) = parse_mcp_tool_name(full).unwrap();
    assert_eq!(s, "chrome_devtools");
    assert_eq!(t, "read_console");
}

#[test]
fn parse_rejects_old_format() {
    assert!(parse_mcp_tool_name("mcp_server_tool").is_none());
}

#[test]
fn is_mcp_tool_checks() {
    assert!(is_mcp_tool("mcp__server__tool"));
    assert!(!is_mcp_tool("mcp_server_tool"));
    assert!(!is_mcp_tool("read_file"));
}
```

### 集成测试

1. 配置 server_id 含特殊字符（`.`, `/`, 空格），验证 sanitize 正确
2. 配置 server_id 含单下划线（`chrome_devtools`），验证解析正确
3. `inject_mcp_tools_prompt` 正确按 server 分组显示
4. `subagent` 正确过滤包含 MCP 工具
5. `tool_executor` retention tier 正确识别 MCP 工具

### E2E 测试

1. 添加名为 `my.test-server` 的 MCP server
2. 在 PluginsView 中确认显示正确
3. 在聊天中使用 MCP 工具，确认 StepIndicator 正确显示 server/tool 分离
