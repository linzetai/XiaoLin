## Why

XiaoLin 的 skill 系统虽已有跨工具扫描（7 层优先级）、三种 prompt 注入模式（full/compact/lazy）和 evolution 自学习能力，但与 Codex 和 Claude Code 相比，在**渐进式披露**、**智能发现**、**前端管理体验**和**工具链完整度**上存在显著差距。

当前存在的具体问题：
- 默认 `full` 模式每轮注入所有 SKILL.md 全文，300+ skills 时 token 消耗极高（compact 可节省 ~90%+）
- `SearchSkillTool` 已实现但标记 `#[allow(dead_code)]`，未注册到 ToolRegistry
- `upload_skill` Tauri 命令写入 `{state_dir}/config/skills/`，但 scan 路径为 `{state_dir}/skills/`，导致上传后不可见
- `ext_registry` 始终为空（`SkillRegistry::new()`），extension 插件 skills 未被加载
- `reload_skills()` 与初始化不一致——缺少 `register_builtin_skills()` 和 `ext_registry` merge，hot-reload 会丢失内置 skill
- `SKILL_AUTHORING_PROMPT` 含虚假「已实现」描述（semantic search、usage tracking），误导 agent
- `full` 模式不注册任何 skill 工具（`UnifiedSkillTool` 仅 compact/lazy 注册）
- 前端已有 skill 列表（Global/Agent 分组）、上传（folder/zip）、refresh，以及 deny list API（`getSkillsDenyList`/`updateSkillsDenyList`），但缺少查看全文、编辑、enable/disable UI（deny API 未接入 UI）
- `frontmatter.tools` 字段被解析但未用于限制工具集

参考 Codex 的 metadata-only 注入（默认 2% context window token budget，超出则 shorten description → omit skill）与 mention/任务匹配后从磁盘读取 SKILL.md；参考 Claude Code 的 frontmatter 索引（char budget 1% context）+ Skill tool 按需读取 + 文件操作时 `paths:` 条件激活。XiaoLin 需要系统性升级 skill 的发现、注入、管理和创建全链路。

## What Changes

### Phase 1: 修 Bug + 激活死代码（30→50 分）
- 修复 `upload_skill` 路径：写入 `{state_dir}/skills/` 对齐 scan 路径；迁移 `config/skills/` 下已有数据
- 注册 `search_skills`：扩展 `UnifiedSkillTool` 增加 `search` action，或独立注册 `SearchSkillTool`
- 所有 prompt 模式注册 `UnifiedSkillTool`（当前仅 compact/lazy 注册）
- 默认 `prompt_mode` 从 `full` 切换为 `compact`
- 修复 `reload_skills()`：与初始化一致，保留 builtin skills 和 extension skills
- 实现 `write_skill` → `reload_skills` 回调（需设计 gateway 层 post-tool hook）
- 加载 extension skills 到 `ext_registry`
- 清理 `SKILL_AUTHORING_PROMPT` 中与现状不符的描述

### Phase 2: 渐进式披露 + 前端管理（50→70 分）
- 实现 context budget 机制：skill 注入量按 context window 比例控制（默认 2-5%），渐进截断（先缩短 description → 再 omit skill）
- 前端 Skill 管理面板：查看全文、编辑内容、单 skill 开关（接入现有 deny list API）、deny list 管理
- `frontmatter.tools` 字段生效：当 skill 激活时限制可用工具集
- Skill 详情 modal：展示 name、description、source、layer、frontmatter 信息
- 扩展 `skills.list` WS API 返回 source/layer/enabled 字段（基于现有 `ws/skills.rs`）

### Phase 3: 智能发现 + Marketplace（70→85 分）
- 条件激活机制：frontmatter `paths:` 字段，文件操作时动态激活（对齐 Claude Code touch-triggered 模式）
- Skill marketplace UI：内置推荐 + GitHub 目录安装（参考 Codex `skill-installer`）
- `/skillify` 元技能：将当前会话片段转换为可复用 skill（参考 Claude Code 内置 `/skillify` bundled skill）
- MCP `skill://` 资源协议支持：MCP server 可暴露 skill 资源（对齐 Claude Code 已有能力）

### Phase 4: 语义搜索 + 闭环反馈（85→100 分）
- 语义搜索：复用 `xiaolin-memory::EmbeddingProvider`（hypembed，all-MiniLM-L6-v2），不引入 fastembed-rs
- Evolution skill 与静态 skill 统一视图：前端合并展示，支持 promote（evolution→static）
- 用量追踪排序：记录 skill 使用频率，高频 skill 优先展示和注入
- 团队 skill 同步：通过云端/Git 仓库实现团队间 skill 共享（仅设计接口）

## Capabilities

### New Capabilities
- `skill-context-budget`: Context window 预算机制，skill 注入量按比例控制，渐进截断（description → omit）
- `skill-management-ui`: 前端 Skill 管理面板，查看全文、编辑、开关、deny list、详情展示
- `skill-conditional-activation`: 基于 frontmatter `paths:` 的条件激活，文件操作时动态激活
- `skill-marketplace`: Skill 市场 UI，内置推荐目录 + GitHub 安装 + 社区分享
- `skill-semantic-search`: 复用 hypembed 的语义 skill 搜索，替代关键词匹配
- `skill-usage-tracking`: Skill 使用频率追踪和基于用量的排序优化
- `skillify-meta-skill`: 将会话片段转换为可复用 skill 的元技能
- `skill-mcp-resource`: 通过 MCP skill:// 资源协议暴露和消费 skill

### Modified Capabilities
- `cross-tool-skills`: 修复路径 bug，注册 search_skills，默认 compact，所有模式注册 UnifiedSkillTool，修复 reload_skills parity，加载 extension skills，清理 SKILL_AUTHORING_PROMPT

## Impact

### 后端（Rust crates）
- `xiaolin-core/src/skill.rs` — SkillRegistry 增加 context budget 截断、条件激活、用量追踪
- `xiaolin-core/src/config.rs` — SkillsConfig 默认值 full→compact，新增 budget/条件激活配置
- `xiaolin-agent/src/builtin_tools/skill.rs` — 扩展 UnifiedSkillTool 增加 search action，清理 dead code
- `xiaolin-agent/src/builtin_tools/mod.rs` — 所有 prompt 模式注册 skill 工具
- `xiaolin-gateway/src/chat_pipeline.rs` — `inject_skills_prompt` 适配 context budget
- `xiaolin-gateway/src/state/builder.rs` — ext_registry 加载、upload 路径修复、full 模式注册
- `xiaolin-gateway/src/state/mod.rs` — 修复 reload_skills() 保留 builtin/ext
- `xiaolin-gateway/src/ws/skills.rs` — 扩展 skills.list 返回 source/layer/enabled；新增 skills.read/update/delete
- `xiaolin-app/src-tauri/src/commands/skill.rs` — upload 路径修复
- `xiaolin-protocol/src/op.rs` — 新增 SkillsRead/SkillsUpdate/SkillsDelete op 类型
- `xiaolin-memory/` — 复用 EmbeddingProvider 为 skill 语义搜索提供 embedding

### 前端（React/TypeScript）
- `src/components/plugins/` — Skill 管理面板（列表增强、详情、编辑、开关）
- `src/components/plugins/` — Skill marketplace 面板
- `src/lib/stores/` — Skill store 扩展（source/layer/enabled 字段）
- `src/lib/api.ts` — 复用已有 deny list API，新增 read/update/delete
