# Skill Extractor 优化

## 问题

当前 skill_extractor 后台任务每 600 秒触发一次，对最近 200 条轨迹全量提取，每个 cluster 调一次 LLM（使用付费 kimi-for-coding 模型）。

实测数据：
- 每轮提取 17 个 candidates = 17 次 LLM 调用
- 每小时 ~100 次后台 LLM 调用
- 无增量/去重机制，相同 cluster 反复提取
- 5 小时 Kimi 额度被后台任务耗尽

## 方案

### 1. 事件驱动替代定时轮询

- 移除 600s 定时器
- 改为：每积累 N 次新成功对话后触发提取（N 可配，默认 10）
- 保留定时器作为 fallback（间隔从 600s 改为 3600s），但仅在有新轨迹时执行

### 2. Cluster 去重 + 持久化

- 对已提取的 cluster 计算指纹（`hash(sorted source_trajectory_ids)`）
- 持久化到数据库/文件，下次提取时跳过已有指纹的 cluster
- 只对"新发现的"cluster 调 LLM

### 3. 配置开关（默认关闭）

- `evolution.skill_extraction_enabled: bool`（默认 `false`）
- 前端设置面板增加 toggle 开关
- 用户可自行选择是否启用

### 4. 模型可配置

- `evolution.skill_extraction_model: Option<String>`（默认使用系统内最廉价模型或 `null`）
- 前端设置面板暴露 model 选择器
- 不强制使用 chat 的付费模型

### 5. 预算上限

- `evolution.skill_extraction_daily_limit: u32`（默认 50）
- 超过后当天不再调 LLM，仅做规则提取

## 涉及文件

| 文件 | 改动 |
|------|------|
| `crates/xiaolin-core/src/config.rs` | 新增 `skill_extraction_enabled`, `skill_extraction_model`, `skill_extraction_daily_limit` |
| `crates/xiaolin-gateway/src/state/mod.rs` | 重写提取触发逻辑（事件驱动 + 去重） |
| `crates/xiaolin-evolution/src/skill_extractor.rs` | 新增 `ClusterFingerprint` 和去重逻辑 |
| `crates/xiaolin-gateway/src/state/builder.rs` | 根据 `enabled` 开关决定是否启动后台任务 |
| `crates/xiaolin-app/src/components/settings/` | 前端 toggle + model selector |
| `crates/xiaolin-protocol/src/op.rs` | 新增 config set 字段 |

## 预期效果

| 指标 | 当前 | 优化后 |
|------|------|--------|
| 默认行为 | 每 10 分钟全量提取 | 关闭（需手动开启） |
| LLM 调用量 | ~100 次/小时 | ~5 次/天（仅新 cluster） |
| 模型 | kimi-for-coding（付费） | 用户自选（建议廉价模型） |
| 重复提取 | 有（同 cluster 反复） | 无（指纹去重） |
| 额度保护 | 仅 circuit breaker | 日限额 + 开关 + circuit breaker |

## 非目标

- 不改变 skill 本身的注入逻辑（system prompt 注入）
- 不改变规则提取的聚类算法
- 不改变 PatternTracker / SkillQualityValidator
