## Why

FastClaw 缺少统一的项目级配置基建。与 Cursor（`.cursor/`）、Codex（`.codex/`）、Claude Code（`CLAUDE.md`）等工具相比：

1. **无 Workspace Root 检测** — 所有路径基于 cwd，从子目录启动丢失项目配置
2. **Session work_dir 数据流断裂** — 前端 `setWorkDir` 调用了后端未实现的 WS 方法，WS list 不返回 workDir，刷新后丢失
3. **无项目级 MCP/Skills/Rules 发现** — 用户只能通过全局配置管理，无法按项目隔离
4. **会话无工作区分组** — Cursor 按项目分组会话，FastClaw 只按日期
5. **不读取其他工具的 Skills** — `.cursor/skills/` 中的 skills 无法被 FastClaw 利用
6. **Agent 不理解配置结构** — 无法帮用户创建 skill、添加 MCP 等

## What Changes

### Phase 1: 修复 Session work_dir 数据流
- 实现 `sessions.set_work_dir` WS 方法
- WS `sessions.list` / `sessions.get` 返回 `workDir` 和 `source`
- 智能标题生成后发送 `sessions.changed` 事件
- 前端正确恢复和同步 workDir

### Phase 2: Workspace Root 自动检测
- 实现向上遍历检测：`.fastclaw/` > `.git/` > 语言标记（Cargo.toml, package.json 等）
- 新建 session 的 work_dir 基于检测到的项目根
- 项目级配置从检测到的根加载：`.fastclaw/skills/`、`.fastclaw/mcp.json`、`.fastclaw/rules/`
- Skills 动态发现：目录含 `SKILL.md` 即识别，无需 manifest 注册

### Phase 3: 工作区分组与生态互操作
- SessionList 按 workDir 分组（类 Cursor）
- 跨工具 Skill 复用：只读扫描 `.cursor/skills/`、`.codex/skills/`
- Agent 元能力：内置 skill + 工具支持 list/add skill、mcp、rule
- `fastclaw init` 便利命令（非必须）

## Capabilities

### New Capabilities
- `workspace-root-detection`: 向上遍历检测项目根目录
- `project-config-discovery`: 项目级 skills/MCP/rules 动态发现
- `session-workspace-sync`: Session work_dir 前后端完整数据流
- `workspace-grouped-sessions`: 按工作区分组的会话列表
- `cross-tool-skills`: 跨工具 skill 只读复用
- `agent-config-tools`: Agent 管理项目配置的元能力

### Modified Capabilities
- `session-management`: 修复 work_dir 数据流断裂
- `skill-discovery`: 扩展扫描路径支持项目级和跨工具
- `mcp-client`: 支持项目级 MCP 配置

## Impact

### Backend (Rust)
- `crates/fastclaw-core/src/workspace.rs` — 新增 workspace root 检测
- `crates/fastclaw-core/src/config.rs` — 项目级配置层加载
- `crates/fastclaw-core/src/skill.rs` — 扩展扫描路径
- `crates/fastclaw-gateway/src/ws/` — 实现 `sessions.set_work_dir`
- `crates/fastclaw-gateway/src/ws/` — WS list/get 返回完整字段
- `crates/fastclaw-gateway/src/state/builder.rs` — 项目级 skill/MCP 加载
- `crates/fastclaw-session/src/store.rs` — work_dir 更新方法接入

### Frontend (Tauri/React)
- `src/lib/stores/session-store.ts` — workDir 持久化和恢复
- `src/components/session-list/SessionList.tsx` — 工作区分组
- `src/lib/transport.ts` — WS 方法对接
- `src/lib/store.ts` — syncSessionsForAgent 补全 workDir

### New Files
- `.fastclaw/mcp.json` schema 定义
- `.fastclaw/rules/` 格式定义
- 内置 skill: `fastclaw-config-manager`
