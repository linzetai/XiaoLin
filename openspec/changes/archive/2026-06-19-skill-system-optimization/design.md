## Context

Skill 系统是 XiaoLin 的核心能力注入机制。当前版本已在 `SkillRegistry`、`SkillEmbeddingStore`、`SearchSkillTool` 和 `AppState` 中实现了完整的 CRUD + 搜索 + 热重载管线。

现有热重载路径：
- `write_skill` 工具 → `reload_callback` → `reload_skills()` + `spawn_skill_embedding_update()`
- WebSocket `skills.refresh` → `reload_skills()`
- MCP skill 发现 → `refresh_mcp_skills()` → `reload_skills()`

缺失：用户直接编辑磁盘上的 `SKILL.md` 文件时不会自动检测变更。

参考实现：`AgentDefWatcher`（`crates/xiaolin-gateway/src/agent_def_watcher.rs`）已用 `notify` crate + debounce 实现了 agent 定义文件的热重载。

## Goals / Non-Goals

**Goals:**
- 新增 `SkillWatcher`，监控 project/global/extension skill 目录
- 优化 `compute_relevance` 的 `to_lowercase()` 热路径开销
- 评估并改进 `search_by_vector` 的数据访问模式

**Non-Goals:**
- 不改变现有 skill 的 CRUD 接口和语义
- 不引入新的外部依赖（`notify` 已存在于 workspace）
- 不重构 embedding 存储引擎（如换用 HNSW 索引），仅优化数据访问
- 不改变 `SkillRegistry` 的线程安全模型（继续使用 `ArcSwap`）

## Decisions

### D1: SkillWatcher 参考 AgentDefWatcher 架构

**选择**: 直接复用 `AgentDefWatcher` 的架构模式 — `notify::RecommendedWatcher` + `tokio::mpsc` + debounce loop。

**理由**:
- 已验证模式，`AgentDefWatcher` 运行稳定
- 代码量小（< 100 行），维护成本低
- 共享相同的 `RecursiveMode::Recursive`（skill 在子目录中）

**替代方案**:
- 合并进 `AgentDefWatcher` — 但两者监控不同路径集，关注不同文件扩展名，职责不同，分离更清晰

### D2: 搜索小写缓存使用 SkillRegistry 级 HashMap

**选择**: 在 `SkillRegistry` 中新增 `lowercase_cache: HashMap<String, CachedLowercase>` 字段（包含 name_lower、desc_lower、when_lower、content_lower）。cache 在 `reload_skills` 时整体重建。

**理由**:
- `SkillRegistry` 生命周期与 skill 内容绑定 — registry 替换时 cache 自然失效
- 不需要额外的失效逻辑
- 搜索函数改为接收 cache 引用，避免在热路径分配

**替代方案**:
- 在 `SkillEntry` 上存储 — 需要 `SkillEntry` 可变，与当前 immutable 设计冲突
- 全局 `RwLock<HashMap>` — 引入锁竞争，不如随 registry 一起 `ArcSwap`

### D3: 向量搜索优化 — 内存缓存而非 SQLite 端计算

**选择**: 在 `SkillEmbeddingStore` 中维护内存缓存 `Vec<(String, Vec<f32>)>`，在 `upsert`/`prune` 后标记 dirty，下次 `search_by_vector` 前按需刷新。

**理由**:
- Skill 数量通常 < 200，全量内存缓存完全可行
- 避免每次搜索的 SQLite I/O
- SQLite 端 cosine similarity UDF 需要加载额外扩展或自定义函数，复杂度不值

**替代方案**:
- SQLite virtual table / UDF — 复杂，跨平台兼容性风险
- 不优化 — 当前 < 50 skill 场景下性能可接受，可延后

## Risks / Trade-offs

- **[内存开销增加]** → 缓存所有 skill 的小写文本 + embedding 向量。对于 200 个平均 5KB 的 skill，增加约 2MB，完全可接受
- **[Watcher 平台差异]** → `notify` 在 Linux (inotify) / macOS (FSEvents) / Windows (ReadDirectoryChangesW) 上行为略有差异 → 使用 `RecommendedWatcher`（已在 `AgentDefWatcher` 验证）
- **[Debounce 窗口]** → 300ms debounce 可能导致快速连续编辑时短暂看到中间状态 → 与 `AgentDefWatcher` 一致，用户无感知
- **[Cache 一致性]** → 若 `reload_skills` 外的路径修改了 registry，cache 可能过期 → 所有修改路径都通过 `reload_skills` 或 `ArcSwap::store`，只需确保 cache 在这些点更新
