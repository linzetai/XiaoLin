## Why

Bootstrap 机制（BOOTSTRAP.md 初始化仪式）在实践中带来了上下文污染问题：当 BOOTSTRAP.md 未被删除时，每个新会话都会注入"Bootstrap Pending"指令，导致 agent 回复中掺杂无关的身份初始化内容。该机制设计过于复杂且脆弱——依赖文件存在性来判断状态，且无可靠的自动清理机制。

## What Changes

- **BREAKING**: 移除 `ensure_bootstrap()` 方法及其创建 BOOTSTRAP.md 的逻辑
- **BREAKING**: 移除 `engine.rs` 中检测 BOOTSTRAP.md 并注入"Bootstrap Pending"消息的逻辑
- 移除 `DEFAULT_BOOTSTRAP_FILENAME` 常量及 `WorkspaceBootstrap.bootstrap` 字段
- 移除 `DEFAULT_BOOTSTRAP_TEMPLATE` 模板内容
- 保留 identity 文件系统（SOUL.md, IDENTITY.md, USER.md, AGENTS.md, TOOLS.md）— 这些不依赖 bootstrap
- `ensure_bootstrap()` 重命名为 `ensure_workspace_files()` 或类似名称，仅创建 identity 模板文件（不含 BOOTSTRAP.md）
- 移除刚刚在 `engine.rs` 中添加的 `identity_already_configured` 防御逻辑（不再需要）

## Capabilities

### New Capabilities

- `workspace-init-without-bootstrap`: workspace 初始化不再依赖 BOOTSTRAP.md 文件，直接创建 identity 模板文件

### Modified Capabilities


## Impact

- `crates/xiaolin-core/src/workspace.rs` — 移除 bootstrap 相关常量、模板、`ensure_bootstrap` 方法中创建 BOOTSTRAP.md 的部分
- `crates/xiaolin-context/src/engine.rs` — 移除 BOOTSTRAP.md 检测和注入逻辑
- `crates/xiaolin-gateway/src/state/builder.rs` — 调用点从 `ensure_bootstrap()` 改为新方法名
- `crates/xiaolin-app/src-tauri/src/commands/agent.rs` — 同上
- 现有 workspace 中的 BOOTSTRAP.md 文件将被忽略（不再读取）
