# fastclaw-security

横切安全层：认证、限流、注入防护与 SSRF 检查。

## 模块

| 模块 | 职责 |
|------|------|
| `auth` | 恒定时间 API Key 校验（`ApiKeyAuth`） |
| `rate_limit` | IP 维度滑动窗口限流（`RateLimiter`） |
| `prompt_guard` | 提示词注入检测与过滤 |
| `ssrf` | 私有 IP 阻断 + DNS 解析检查，防止 SSRF |

## 关键导出

```rust
pub use auth::{ApiKeyAuth, AuthConfig};
pub use rate_limit::RateLimiter;
pub use prompt_guard::PromptGuard;
pub use ssrf::SsrfChecker;
```
