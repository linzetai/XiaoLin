# fastclaw-security

横切安全层：认证、限流、注入防护、危险操作拦截与 SSRF 检查。

## 模块

| 模块 | 职责 |
|------|------|
| `auth` | 恒定时间 API Key 校验（`ApiKeyAuth`） |
| `rate_limit` | IP 维度滑动窗口限流（`RateLimiter`） |
| `prompt_guard` | 提示词注入检测与过滤（High 风险自动拦截，Medium 仅告警） |
| `dangerous_ops` | 危险 shell 命令拦截策略 |
| `ssrf` | 私有 IP 阻断 + DNS 解析检查，防止 SSRF |

`prompt_guard` 由 `security.promptInjectionDetection` 配置项控制开关，已集成至 gateway chat pipeline。

## 关键导出

```rust
pub use auth::{ApiKeyAuth, AuthConfig};
pub use rate_limit::RateLimiter;
pub use prompt_guard::{PromptGuard, PromptGuardResult, RiskLevel};
```
