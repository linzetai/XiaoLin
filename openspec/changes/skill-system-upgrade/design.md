## Context

XiaoLin 当前的 skill 系统架构：
- **核心数据**: `xiaolin-core/src/skill.rs` — `SkillRegistry`/`SkillEntry`/`SkillFrontmatter`，支持跨工具目录扫描（Extension → SharedAgents → UserCodex → UserCursor → Global → ProjectCursor → ProjectFastclaw → AgentWorkspace 共 8 层优先级）
- **Agent 工具**: `xiaolin-agent/src/builtin_tools/skill.rs` — `UnifiedSkillTool`（统一 `skill` 工具，action: list/read/write），仅在 compact/lazy 模式注册；`SearchSkillTool` 已实现但 `#[allow(dead_code)]` 未注册
- **Prompt 注入**: `xiaolin-gateway/src/chat_pipeline.rs` — `inject_skills_prompt()` 根据 prompt_mode 注入 full/compact/lazy 格式
- **WS API**: `xiaolin-gateway/src/ws/skills.rs` — 已有 `skills.list`/`skills.refresh`
- **前端 API**: `api.ts` — 已有 `getSkillsDenyList`/`updateSkillsDenyList`（未接入 UI）
- **Evolution**: `xiaolin-evolution` — 从会话轨迹自动提取 skill 模式（SQLite 存储）
- **完整加载链**: Extension(builtin) → legacy `{state}/skills` → cross-tool 7 dirs → per-agent workspace overlay

当前存在的 6 个 Bug/死代码：
1. `upload_skill` 写入 `{state_dir}/config/skills/`，scan 用 `{state_dir}/skills/`
2. `SearchSkillTool` 实现完整但 `#[allow(dead_code)]`，未注册
3. `ext_registry = SkillRegistry::new()` 始终为空
4. `reload_skills()` 缺少 `register_builtin_skills()` 和 `ext_registry` merge
5. `full` 模式不注册 `UnifiedSkillTool`
6. `SKILL_AUTHORING_PROMPT` 含虚假「已实现」描述

竞品对比关键启发：
- **Codex**: 2% context window **token budget**（`SKILL_METADATA_CONTEXT_WINDOW_PERCENT=2`）；渐进截断：shorten description → omit skill → path alias 压缩；`$SkillName` mention 触发全文读取；skill-installer 从 GitHub 目录安装
- **Claude Code**: 1% context × 4 chars/token **char budget**（`SKILL_BUDGET_CONTEXT_PERCENT=0.01`）；frontmatter-only 索引；Skill tool 按需读取；`paths:` 字段 **touch-triggered** 条件激活（文件操作时触发 `activateConditionalSkillsForPaths`，非 session 全量索引）；`/skillify` 会话转 skill（ant-only bundled）；MCP `skill://` 资源（`mcp__<server>__<name>` 命名）

## Goals / Non-Goals

**Goals:**
- 将默认 token 消耗从 O(N×全文) 降低到 O(N×单行摘要)（~90%+ 节省），N = skill 数量
- 激活已有但未生效的能力（SearchSkillTool、extension skills、frontmatter.tools）
- 修复所有已知 bug（upload 路径、reload parity、SKILL_AUTHORING_PROMPT）
- 提供完整的前端 skill 管理体验（CRUD、开关、详情、搜索，复用已有 deny API）
- 实现智能 skill 发现（touch-triggered 条件激活、语义搜索）
- 建立 skill 生态闭环（创建 → 发现 → 使用 → 反馈 → 改进）

**Non-Goals:**
- 不改变 SKILL.md 文件格式和 frontmatter schema（保持与 Cursor/Codex 兼容）
- 不引入 fastembed-rs（复用已有 `xiaolin-memory` 的 `EmbeddingProvider` + hypembed）
- 不实现多租户/云端 skill 同步（Phase 4 仅设计接口，实现推迟）
- 不重写 evolution skill 系统（仅添加统一视图接口和 promote 能力）

## Decisions

### D1: 默认 prompt_mode 切换为 compact

**选择**: `compact`（name + one-line description 列表）

**备选方案**:
- `lazy`（仅 ID 列表 + count）：太少信息，LLM 难以判断何时使用哪个 skill
- 保持 `full`：300 skill 约消耗 200K+ token/轮，不可接受

**理由**: compact 已实现，full→compact 在大 skill 集场景下节省 ~90%+ token（full 注入完整 SKILL.md body，compact 仅 name + 一行 description）。配合 context budget 进一步控制总量。LLM 通过 `read_skill` 按需获取全文。

### D2: Context budget 基于 token 估算（1 token ≈ 4 chars）

**选择**: 按 context window 百分比计算 token budget，用 4 chars/token 近似转换为 char budget

**公式**: `char_budget = context_window_tokens × (percent / 100) × 4`

**默认值**: 5%（比 Codex 2% 宽松，因为 compact 模式已大幅节省）

**渐进截断策略**（对齐 Codex）:
1. 正常注入 name + description
2. Budget 不足时：先 **缩短 description**（截取首行）
3. 仍不足：**omit 低优先级 skill**（从 Extension 层开始）
4. 截断 warning 发送到 session 状态/UI 通道，**不追加到 system prompt**

**备选方案**:
- 精确 token 计数（需 tiktoken）：增加延迟和依赖
- 固定行数限制：不同 skill 长度差异大，不灵活
- 整 skill 直接 omit：比 Codex 粗糙，不够优雅

### D3: Skill 开关存储在 SkillsConfig.deny 列表

**选择**: 在 `SkillsConfig.deny` 管理禁用，不修改 SKILL.md

**优先级**: `deny` 列表优先于 `frontmatter.enabled`（deny 覆盖 enabled）

**备选方案**:
- 修改 SKILL.md `enabled: false`：会改动跨工具共享文件
- 独立 `.xiaolin/skill-overrides.json`：增加配置文件

**理由**: `filtered()` 已实现并接入。前端已有 `getSkillsDenyList`/`updateSkillsDenyList` API（`api.ts`），仅需在 UI 接入。deny 变更后触发 `skills.refresh` 或 config hot-reload。

### D4: 条件激活使用 touch-triggered 模式（对齐 Claude Code）

**选择**: 在 `SkillFrontmatter` 新增 `paths: Vec<String>` 字段，**文件操作时**（而非 session 初始化全量索引）检查匹配

**激活时机**:
- 初始化：分离 conditional（有 `paths:`）和 unconditional（无 `paths:`）skill
- Unconditional skill 始终注入
- Conditional skill 仅当 tool 操作的文件路径匹配 `paths:` glob 时激活
- `paths: []` 或 `paths: ["**"]` 等同于 unconditional

**glob 语义**: 使用 `globset` crate，gitignore 风格相对路径匹配

**备选方案**:
- Session 初始化全量索引：大 repo 上耗时，且不如 touch-triggered 精确
- 基于 tag 匹配：需预定义 tag 体系

### D5: Marketplace 先实现 GitHub raw fetch

**选择**: 内置推荐目录（JSON 配置 + GitHub raw URL）→ 下载**整个 skill 目录**到 `~/.xiaolin/skills/`

**注意**: 区别于 Codex（仅下载 SKILL.md + restart 生效），XiaoLin 下载目录树 + hot-reload 立即生效

**备选方案**:
- 复用已有 `HubClient`（`xiaolin-core/hub.rs` 已实现但未接入）：可作为 marketplace 后端选项
- npm/cargo 式包管理：对 markdown skill 过重

### D6: 语义搜索复用 hypembed（不引入 fastembed-rs）

**选择**: 扩展 `xiaolin-memory::EmbeddingProvider` trait，新增 skill embedding 表，复用 hypembed（all-MiniLM-L6-v2）

**备选方案**:
- 引入 fastembed-rs（ONNX Runtime）：造成双 embedding 栈，打包体积增大
- 调用 LLM API 生成 embedding：网络依赖，成本

**理由**: `xiaolin-memory` 已有 `LocalEmbeddingProvider` + SQLite BLOB 存储 + cosine 检索。Claude Code 使用 TF-IDF 式 local search，Codex 使用关键词 relevance——语义搜索是 XiaoLin 的增量创新。

### D7: hot-reload 回调设计

**问题**: `WriteSkillTool` 在 `xiaolin-agent` crate，无法直接访问 `AppState::reload_skills()`

**选择**: 在 tool 注册时注入 `Arc<dyn Fn() -> Result<()> + Send + Sync>` 回调闭包，由 gateway 层提供 reload 实现

**备选方案**:
- 全局 channel（`tokio::sync::broadcast`）：更灵活但复杂度高
- Agent tool executor post-hook：需改 runtime 框架

## Risks / Trade-offs

**[Risk] compact 模式下 LLM 可能不知道触发 read_skill**
→ Mitigation: `format_compact` 已包含提示 "Use `read_skill` tool with the skill ID to get full instructions"。通过 benchmark 验证 skill 调用率。

**[Risk] Context budget 截断可能导致关键 skill 丢失**
→ Mitigation: 按 layer 优先级排序（AgentWorkspace > ProjectFastclaw > ...），从低优先级开始截断。先缩短 description 再 omit。截断 warning 发送到 UI 而非注入 prompt。

**[Risk] touch-triggered 条件激活增加 tool execution 后置计算**
→ Mitigation: 仅在文件路径变化时重新评估。使用 `globset` batch 匹配，性能可控。

**[Risk] frontmatter.tools 限制可能破坏现有 skill**
→ Mitigation: `tools: []`（空）表示不限制。仅非空时生效。Phase 2 实施，有充足测试时间。

**[Risk] reload_skills 一致性**
→ Mitigation: Phase 1 首要任务之一。统一初始化和 reload 路径，确保 builtin + ext + legacy + cross-tool 全部保留。

**[Risk] 双 disable 语义（deny list vs frontmatter.enabled）**
→ Mitigation: 明确优先级：deny 覆盖 enabled。UI 只操作 deny list。

## Migration Plan

分四个 Phase 逐步推进，每个 Phase 独立可测试和部署：

1. **Phase 1**（向后兼容）：修 bug + 默认值变更。用户可通过配置手动回退到 `full` 模式。
2. **Phase 2**（增量新功能）：新增 UI 和 budget 机制，不破坏现有行为。
3. **Phase 3**（新能力）：条件激活和 marketplace 为纯新增功能。
4. **Phase 4**（高级特性）：语义搜索为可选增强。

回滚策略：每个 Phase 可通过 `SkillsConfig` 配置字段（`prompt_mode`、`context_budget_percent`）独立回退。

## Open Questions

1. **compact 模式下 benchmark 通过率是否下降？** Phase 1 实施后立即验证。
2. **ext_registry 来源**：仅目录扫描（`extensions_dir`）还是预加载 MCP skill？Phase 1 先做目录扫描，Phase 3 扩展 MCP。
3. **write_skill 三种目标路径优先级**：workspace（agent 私有）/ project（`.xiaolin/skills/`）/ global（`~/.xiaolin/skills/`）——需统一 spec 和 skillify 的默认写入路径。
4. **HubClient 复用**：marketplace 是否复用已有 `HubClient`（`xiaolin-core/hub.rs`）还是新建 GitHub raw fetch？
5. **evolution skill promote UX**：Phase 4 时定义。
