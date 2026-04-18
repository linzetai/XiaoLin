# fastclaw-observe

可观测性基础设施：结构化日志与 Prometheus 指标。

## 功能

- **初始化** — `init_observability` 配置 tracing-subscriber（env-filter + JSON 格式可选）
- **Prometheus 指标** — `render_metrics` 导出文本格式指标，供 `/metrics` 端点使用
- **预定义指标** — 聊天请求、工具调用、WebSocket 连接、会话操作、插件调用等维度的 counter/histogram
- **`MetricsCollector`** — 全局指标收集器，提供 `record_*` 辅助方法

## 关键导出

```rust
pub use init::init_observability;
pub use metrics::{render_metrics, MetricsCollector, default_metrics_collector};
```
