## Phase 1: 修复 Session work_dir 数据流

- [x] 1.1 在 `crates/fastclaw-gateway/src/ws/` 中实现 `sessions.set_work_dir` handler，调用 `SessionStore::update_work_dir` 并广播 `sessions.changed`
- [x] 1.2 修改 WS `sessions.list` 响应，包含 `work_dir` 和 `source` 字段
- [x] 1.3 修改 WS `sessions.get` 响应，包含 `work_dir` 和 `source` 字段
- [x] 1.4 在 `generate_smart_title` 成功后发送 `sessions.changed` 事件
- [x] 1.5 前端 `syncSessionsForAgent` 从 WS 响应中正确恢复 `workDir` 和 `source`
- [x] 1.6 前端 `setSessionWorkDir` 调用新实现的 WS `sessions.set_work_dir`
- [x] 1.7 前端 `onSessionChanged` handler 更新 `workDir`（不仅仅是 title）

## Phase 2: Workspace Root 自动检测

- [x] 2.1 在 `crates/fastclaw-core/src/workspace.rs` 中实现 `detect_workspace_root(start: &Path) -> PathBuf`，按 `.fastclaw/` > `.git/` > 语言标记优先级向上遍历
- [x] 2.2 在 session 创建时（`resolve_session_context`）使用 `detect_workspace_root` 结果作为默认 `work_dir`（而非 agent workspace path）
- [x] 2.3 在 gateway 启动时使用 `detect_workspace_root` 确定项目级配置搜索根
- [x] 2.4 修改 `load_config` 增加 `<workspace_root>/.fastclaw/config.json` 作为最高优先级配置层

## Phase 2.5: 项目级 Skills 动态发现

- [x] 2.5.1 扩展 `SkillLayer` 枚举，增加 `ProjectFastclaw`、`ProjectCursor`、`UserCursor`、`UserCodex`、`SharedAgents` 层级
- [x] 2.5.2 修改 `state/builder.rs` 中 skill 加载逻辑，按完整扫描路径发现 skills：`~/.agents/skills/` → `~/.codex/skills/` → `~/.cursor/skills/` → `~/.fastclaw/skills/` → `<root>/skills/` → `<root>/.cursor/skills/` → `<root>/.fastclaw/skills/`
- [x] 2.5.3 为每个 skill 附加 `SkillSource { origin, layer, path }` 元信息
- [x] 2.5.4 确保跨工具目录为只读：agent 创建 skill 时只写入 `.fastclaw/skills/`

## Phase 2.6: 项目级 MCP 配置

- [x] 2.6.1 定义 `ProjectMcpConfig` 结构体，与 Cursor `.cursor/mcp.json` 格式兼容
- [x] 2.6.2 在 gateway 启动时加载 `<workspace_root>/.fastclaw/mcp.json`，与用户级 MCP 合并（项目级优先，`enabled: false` 可屏蔽用户级）
- [x] 2.6.3 MCP 热重载时也刷新项目级配置

## Phase 2.7: 项目级 Rules

- [x] 2.7.1 实现 `<workspace_root>/.fastclaw/rules/*.md` 扫描加载
- [x] 2.7.2 解析 YAML frontmatter（`alwaysApply`、`globs` 字段）
- [x] 2.7.3 将 `alwaysApply: true` 的 rules 注入 system prompt
- [x] 2.7.4 将带 `globs` 的 rules 在匹配文件操作时动态注入

## Phase 3: 按工作区分组会话

- [x] 3.1 前端 `SessionList` 实现按 `workDir` 分组，组标题显示项目名 + 会话数
- [x] 3.2 无 `workDir` 的会话归入"未关联项目"组
- [x] 3.3 组内按 `updatedAt` 降序排列
- [x] 3.4 新建会话自动继承当前 workspace root

## Phase 3.5: Agent 元能力

- [x] 3.5.1 创建内置 skill `fastclaw-config-manager/SKILL.md`，描述 `.fastclaw/` 结构和操作约定
- [x] 3.5.2 实现 `list_project_config` 工具，展示当前项目 skills/MCP/rules 及来源
- [x] 3.5.3 启动时将项目配置摘要注入 system prompt 上下文

## Phase 3.6: fastclaw init 便利命令

- [x] 3.6.1 实现 `fastclaw init` CLI 命令：检测项目类型，创建 `.fastclaw/` 目录结构
- [x] 3.6.2 交互式询问是否添加 `.fastclaw/` 到 `.gitignore`
- [x] 3.6.3 生成带注释的模板 `config.json`
