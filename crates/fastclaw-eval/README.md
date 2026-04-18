# fastclaw-eval

Agent 行为评估框架：定义测试用例、运行评估套件、对比预期与实际行为。

## 功能

- **评估用例** — `EvalCase` 定义输入、预期行为与判定标准
- **用例加载** — `load_eval_cases_from_dir` 从 JSON 文件批量加载
- **套件运行** — `run_eval_suite` 批量执行评估并汇总结果
- **Mock Driver** — `MockEvalAgent` 用于单元测试

## 关键导出

```rust
pub use eval::{EvalCase, EvalResult, run_eval_case, run_eval_suite};
pub use driver::{EvalAgentDriver, MockEvalAgent};
pub use loader::load_eval_cases_from_dir;
```
