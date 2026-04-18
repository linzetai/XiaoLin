# fastclaw-cron

持久化定时任务调度器。

## 功能

- **Cron 表达式** — 标准 cron 语法定义调度规则
- **SQLite 持久化** — `CronJobStore` 将任务状态持久化，支持崩溃恢复
- **可插拔触发器** — `JobTrigger` trait 定义任务执行逻辑
- **调度器** — `CronScheduler` 管理任务注册、调度与执行

## 关键导出

```rust
pub use scheduler::CronScheduler;
pub use trigger::JobTrigger;
pub use store::{CronJobStore, CronJob, JobAction, JobStatus};
```
