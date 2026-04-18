# fastclaw-model-router

模型路由与成本追踪：根据策略、复杂度与预算选择最优模型。

## 功能

- **五种路由策略** — fixed（固定）、cost（成本优先）、quality（质量优先）、latency（延迟优先）、fallback（兜底链）
- **复杂度分层** — Tiny → Frontier 多级复杂度自动评估
- **Token 估算** — `TokenEstimate` 预估输入/输出 token 数
- **成本估算** — `CostEstimator` 按模型定价估算调用成本
- **预算追踪** — `BudgetTracker` 原子级预算预留/释放

## 关键导出

```rust
pub use router::{ModelRouter, RouteResult};
pub use budget::BudgetTracker;
pub use cost::CostEstimator;
pub use token::TokenEstimate;
```
