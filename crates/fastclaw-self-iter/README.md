# fastclaw-self-iter

自迭代修复引擎：诊断错误、沙箱验证与自动修复闭环。

## 功能

- **诊断器** — `Diagnostician` 从执行错误中提取结构化诊断信息
- **迭代引擎** — `SelfIterEngine` 驱动"诊断 → 修复 → 验证"循环
- **沙箱运行器** — `SandboxRunner` 抽象层，在受控环境中验证修复结果

## 关键导出

```rust
pub use diagnostician::Diagnostician;
pub use engine::{SelfIterEngine, IterationResult};
pub use sandbox::SandboxRunner;
```
