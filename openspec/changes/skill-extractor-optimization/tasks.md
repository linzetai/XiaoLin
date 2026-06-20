# Skill Extractor 优化 — 任务清单

## 1. 配置层

- [x] 1.1 `config.rs`: 新增 `skill_extraction_enabled: bool`（默认 false）
- [x] 1.2 `config.rs`: 新增 `skill_extraction_model: Option<String>`
- [x] 1.3 `config.rs`: 新增 `skill_extraction_daily_limit: u32`（默认 50）
- [x] 1.4 `config.rs`: 将 `skill_extraction_interval_secs` 默认从 600 改为 3600

## 2. Cluster 去重持久化

- [x] 2.1 定义 `ClusterFingerprint` 结构（hash of sorted trajectory_ids）
- [x] 2.2 实现持久化存储（SQLite skill_cluster_fingerprints 表或 JSON 文件）
- [x] 2.3 提取前检查指纹，跳过已提取 cluster
- [x] 2.4 提取成功后写入指纹

## 3. 事件驱动触发

- [x] 3.1 在 trajectory 写入时递增计数器
- [x] 3.2 计数器达到阈值时触发提取（替代纯定时器）
- [x] 3.3 保留定时器作为 fallback（3600s），但仅在有新轨迹时执行
- [ ] 3.4 `evolution.skill_extraction_trigger_count: u32`（默认 10）

## 4. 模型路由

- [x] 4.1 `LlmSkillExtraction` 使用 `skill_extraction_model` 指定的模型
- [x] 4.2 若未配置，使用系统中最廉价模型（或 fallback 到 default）
- [x] 4.3 确保 provider 路由正确（参考现有 `create_provider_with_credentials`）

## 5. 预算限制

- [x] 5.1 实现 daily call counter（in-memory + 日期检查）
- [x] 5.2 达到限额后 skip LLM，仅做规则提取
- [x] 5.3 日志中报告当日已用/剩余

## 6. 前端设置 UI

- [x] 6.1 设置面板增加 "技能自动提取" toggle（对应 `skill_extraction_enabled`）
- [x] 6.2 展开后显示模型选择器（对应 `skill_extraction_model`）
- [x] 6.3 展开后显示每日上限输入框
- [x] 6.4 通过 `config.set` WS API 保存配置

## 7. 闭环验证

- [x] 7.1 dev 测试：默认关闭时无后台 LLM 调用
- [ ] 7.2 dev 测试：开启后仅新 cluster 触发 LLM
- [ ] 7.3 dev 测试：达到日限额后停止 LLM 调用
