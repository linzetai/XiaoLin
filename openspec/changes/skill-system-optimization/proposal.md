## Why

Skill 系统在对比 Codex 和 Claude Code 的实现后，发现三个性能/可维护性瓶颈：

1. `compute_relevance` 搜索时对每个 skill 的完整 content 做 `to_lowercase()`，随 skill 数量和内容长度增长为 O(N×C) 开销
2. `search_by_vector` 每次搜索全量加载 SQLite 中所有 embeddings 到内存，无法利用数据库索引
3. Skill 目录缺少 filesystem watcher — agent 定义目录已有 `AgentDefWatcher`，但 skill 目录的变更（用户直接编辑 SKILL.md）不会自动生效

## What Changes

- 为 `compute_relevance` 引入预计算的小写缓存，避免搜索热路径上的重复 `to_lowercase()`
- 优化 `search_by_vector` 的向量检索路径，减少全量 I/O
- 新增 `SkillWatcher`（类似现有 `AgentDefWatcher`），监控 skill 目录变更并自动触发 `reload_skills()`

## Capabilities

### New Capabilities
- `skill-file-watcher`: 监控 skill 目录（project/global/extension）的文件变更，debounce 后自动热重载

### Modified Capabilities
- `skill-search`: 优化搜索性能 — 小写缓存 + 向量检索路径改进

## Impact

- `crates/xiaolin-agent/src/builtin_tools/skill.rs` — `compute_relevance` 函数重构
- `crates/xiaolin-core/src/skill_embedding.rs` — `search_by_vector` 优化
- `crates/xiaolin-gateway/src/state/mod.rs` — 集成 `SkillWatcher`
- `crates/xiaolin-gateway/src/state/builder.rs` — 初始化 watcher
- 新增 `crates/xiaolin-gateway/src/skill_watcher.rs`（参考 `agent_def_watcher.rs`）
- 依赖：`notify` crate（已有）
