## ADDED Requirements

### Requirement: 搜索热路径避免重复 to_lowercase
`compute_relevance` 函数 SHALL 使用预计算的小写内容缓存，而非在每次关键词搜索调用时对每个 skill 的 `content` 做 `to_lowercase()`。

#### Scenario: 多次搜索复用缓存
- **WHEN** 用户连续执行多次 `search_skills` 调用
- **THEN** 系统 MUST 复用已缓存的小写内容，不重复计算
- **THEN** 搜索延迟 SHOULD 低于无缓存版本（特别是 skill 数量 > 20 且内容较长时）

#### Scenario: skill 内容变更后缓存失效
- **WHEN** skill 内容因 `write_skill` 或 watcher 触发而更新
- **THEN** 系统 MUST 清除对应 skill 的小写缓存
- **THEN** 下次搜索使用新内容

### Requirement: 向量搜索避免全量加载
`search_by_vector` SHALL 优化数据访问模式，减少不必要的全量 I/O。

#### Scenario: 少量 skill 时行为不变
- **WHEN** 注册的 skill 数量 < 50
- **THEN** 性能表现与当前实现一致
- **THEN** 搜索结果与当前实现完全一致

#### Scenario: 大量 skill 时性能可控
- **WHEN** 注册的 skill 数量 > 100
- **THEN** 向量搜索 SHOULD 避免将所有 embedding 同时加载到内存
- **THEN** 可考虑分批加载或 SQLite 端计算（UDF/virtual table）

### Requirement: 搜索结果正确性不变
优化 MUST NOT 改变搜索结果的排序或内容。

#### Scenario: 关键词搜索结果一致
- **WHEN** 对相同 registry 和 query 执行 keyword_search
- **THEN** 返回的结果集和排序 MUST 与优化前完全一致

#### Scenario: 混合搜索结果一致
- **WHEN** 对相同 registry、embeddings 和 query 执行 hybrid_search
- **THEN** 返回的结果集和排序 MUST 与优化前完全一致
