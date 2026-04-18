# fastclaw-context

六层上下文拼装引擎：滚动压缩与用户画像。

## 功能

- **上下文引擎** — `ContextEngine` 按层级组装系统提示、记忆注入、工具描述、会话历史等
- **上下文管理器** — `ContextManager` 协调多来源上下文并按 token 预算裁剪
- **滚动压缩** — `ContextCompactor` 对超长会话进行摘要压缩
- **用户画像** — `UserProfile` 整合用户偏好与历史行为

## 关键导出

```rust
pub use engine::ContextEngine;
pub use manager::{ContextManager, AssembledContext};
pub use compressor::ContextCompactor;
pub use user_profile::UserProfile;
```
