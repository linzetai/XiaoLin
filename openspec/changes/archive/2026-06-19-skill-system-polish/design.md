## Context

XiaoLin skill 系统经历了 Phase 1-4 大规模建设，功能已对齐行业（跨工具发现、向量搜索、evolution 学习、GUI 管理）。但在 Claude Code / Codex 三方对比中暴露了 Token 效率、权限安全、架构重复方面的短板。本次改动聚焦"打磨"而非"新功能"。

当前架构：
- 107 static skills + 3150 evolution skills
- 5% context budget ≈ 25,600 chars @ 128k context
- 注入路径：`inject_skills_prompt`（static）+ `inject_relevant_skills`（evolution），独立运行
- 搜索路径：`SearchSkillTool` hybrid + `SkillStore::find_similar` keyword，独立运行

## Goals / Non-Goals

**Goals:**
- 将 token 消耗降低 60%（5% → 2%），不影响 agent 任务通过率
- 修复 3 个 P0 正确性 bug（用量计数、hash 稳定性、extension 扫描）
- 新增 `when_to_use` 提升搜索精度
- 建立 evolution skill 安全审查基础

**Non-Goals:**
- 不重写 evolution 架构（dual-store 保留，统一搜索留给后续）
- 不实现 `$skill` mention 解析（需前端配合，单独 change）
- 不实现文件监听热重载（scope 较大，单独 change）
- 不实现 delta-based 注入（需重构注入模型为 per-turn 状态管理，scope 较大）

## Decisions

### D1: Budget percent 从 5% 降至 2%

**选择**: 2%（对齐 Codex）

**理由**: Codex 在 128k context 下用 2%（≈10,240 chars）稳定运行；Claude Code 用 1%。XiaoLin 的 Compact 模式每个 skill 只占 ~50 chars（name + desc），107 skills ≈ 5,350 chars，2% 充裕。

**替代方案**:
- 1%（CC 标准）→ 107 skills 已接近上限，留余量不足
- 3%（折中）→ 无充分理由比 Codex 高

### D2: 精确注入用量记录

**选择**: 在 `format_with_budget_ordered` 返回值中携带实际注入的 skill IDs，仅记录这些

**理由**: 当前在 format 之后从 `effective_reg.list()` 收集所有 enabled skills 的 IDs（`chat_pipeline.rs` L698-703），而非从 format 结果中提取实际注入的子集。修复点：让 `format_with_budget_ordered` 返回包含的 skill IDs，然后在 record 时使用该子集。

### D3: 替换 DefaultHasher

**选择**: `blake3` crate（需新增依赖到 `xiaolin-core/Cargo.toml`）

**理由**: blake3 性能接近非加密 hash 但输出跨平台/跨版本确定性。单文件改动量小（`skill_embedding.rs` 中 `content_hash` 函数）。

**替代方案**:
- `sha256` — 更慢，无额外安全需求
- `xxhash` 固定 seed — 快但需手动管理 seed 稳定性

### D4: Extension 嵌套扫描

**选择**: 修改 `load_extension_skills` 扫描 `extensions/*/skills/` 子目录

**理由**: 保持一层深度但多一级前缀。不做无限递归（避免扫描 `node_modules` 等巨目录）。

### D5: `when_to_use` 字段

**选择**: frontmatter 可选字段 `when_to_use: string`，搜索权重 2.0（与 CC 对齐）

**实现**: `SkillFrontmatter` 新增字段 → `compute_relevance` 新增权重 → Compact 注入时追加 `when: ...` 行

### D6: Safe-property 审查

**选择**: evolution promote 时校验 frontmatter 不含危险字段（`shell`、`hooks` 等），否则警告

**理由**: 自动学习的 skill 可能被恶意对话污染。参考 CC 的 `SAFE_SKILL_PROPERTIES` 白名单。

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Budget 降至 2% 可能导致 skill 被截断 | Compact 模式下每 skill ~50 chars，107 skills < 6k chars，2% ≈ 10k chars，充裕 |
| `blake3` 替换导致所有缓存失效 | 一次性全量重算，后续稳定；可在迁移时清除旧表 |
| `when_to_use` 增加 frontmatter 维护负担 | 设为可选，不强制；evolution 提取时自动填充 |
| Extension 二级扫描可能发现非预期目录 | 限制为 `extensions/*/skills/` 精确模式 |
