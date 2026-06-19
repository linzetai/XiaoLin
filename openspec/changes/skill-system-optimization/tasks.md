## 1. SkillWatcher — 文件系统监控

- [ ] 1.1 创建 `crates/xiaolin-gateway/src/skill_watcher.rs`，参考 `agent_def_watcher.rs` 架构，使用 `notify::RecommendedWatcher` + `tokio::mpsc` + 300ms debounce
- [ ] 1.2 Watcher 过滤仅 `SKILL.md` 文件变更（检查文件名或路径含 `SKILL.md`）
- [ ] 1.3 Debounce 后调用 `AppState::reload_skills()` + `spawn_skill_embedding_update()`
- [ ] 1.4 在 `state/builder.rs` 的初始化流程中启动 `SkillWatcher`，传入 project/global/extension skill 目录列表
- [ ] 1.5 错误处理：目录不存在时 `warn!` 并跳过，不影响启动
- [ ] 1.6 在 `state/mod.rs` 中存储 `_skill_watcher: Option<SkillWatcher>` 以保持生命周期

## 2. 搜索小写缓存

- [ ] 2.1 在 `SkillRegistry` 中新增 `lowercase_cache: HashMap<String, CachedLowercase>` 字段（name_lower, desc_lower, when_lower, content_lower）
- [ ] 2.2 在 `reload_skills` 重建 registry 时同步构建 lowercase_cache
- [ ] 2.3 修改 `compute_relevance` 函数签名，接收 cache 引用而非每次重新计算
- [ ] 2.4 确保 `keyword_search` 和 `hybrid_search` 传递 cache

## 3. 向量搜索优化

- [ ] 3.1 在 `SkillEmbeddingStore` 中新增 `cached_embeddings: RwLock<Option<Vec<(String, Vec<f32>)>>>` 内存缓存
- [ ] 3.2 `search_by_vector` 优先使用内存缓存，仅在 cache 为 None 时从 SQLite 加载
- [ ] 3.3 `upsert` 和 `prune` 操作后将 cache 标记为 dirty（设为 None）
- [ ] 3.4 确保并发安全：`RwLock` 读写不阻塞其他操作

## 4. 验证

- [ ] 4.1 `cargo check` + `cargo clippy -- -D warnings` 零警告
- [ ] 4.2 `cargo test -p xiaolin-core` 全部通过（skill + skill_embedding 相关测试）
- [ ] 4.3 手动测试：启动 dev server，直接编辑 SKILL.md，验证 watcher 触发重载（日志输出确认）
- [ ] 4.4 手动测试：`search_skills` 调用验证结果正确性不变
