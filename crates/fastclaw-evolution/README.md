# fastclaw-evolution

自我进化系统：反馈收集、评估、提示蒸馏与 Hermes 风格技能生命周期。

## 功能

- **反馈收集** — `FeedbackStore` 存储用户反馈与评分
- **策略评估** — `PromptDistiller` 根据规则或可选 LLM 进行提示词蒸馏
- **轨迹存储** — `TrajectoryStore` 记录 Agent 执行轨迹
- **技能提取** — `SkillExtractor` 从轨迹中提取可复用技能
- **技能存储** — `SkillStore` 管理技能的全生命周期（提取 → 存储 → 检索 → 注入 → 退役）

## 关键导出

```rust
pub use distiller::PromptDistiller;
pub use evaluator::Evaluator;
pub use feedback::FeedbackStore;
pub use skill_extractor::SkillExtractor;
pub use skill_store::SkillStore;
pub use trajectory::TrajectoryStore;
```
