## Why

XiaoLin 的 skill 系统在三方对比中（vs Claude Code ~84分, Codex ~76分）综合得分 ~73分。核心短板在于 **Token 纪律（5% budget 远超行业 1-2%）**、**权限安全（仅 deny list）**和**架构重复（static + evolution 双轨独立运行）**。当前系统已功能完备（Phase 1-4 已全部实施），是时候从"功能堆叠"转向"精细打磨"。

## What Changes

- **降低 context budget 至 2%** — 与 Codex 对齐，减少 60% token 浪费
- **修复注入用量过度计数** — 仅记录 truncation 后实际注入的 skill IDs
- **替换 `DefaultHasher` 为 `blake3`** — 确保 embedding 缓存跨 Rust 版本稳定
- **Extension 嵌套 skill 扫描** — 递归或显式注册嵌套路径
- **新增 `when_to_use` frontmatter 字段** — 搜索权重 2.0，参考 Claude Code 的 `whenToUse` 设计
- **Safe-property 白名单** — evolution 学习的 skill 自动 promote 前需审查

> **Out of scope (留给后续 change):**
> - Delta-based skill 注入（需重构注入模型为 per-turn 状态管理）
> - Listing / 全文分离（需配合 delta 注入重构）

## Capabilities

### New Capabilities
- `token-discipline`: Token 预算优化——降低 budget percent、delta 注入、listing/body 分离
- `skill-safety`: Skill 权限安全——safe-property 白名单、frontmatter.tools 被动生效、YAML 校验诊断
- `search-enhancement`: 搜索增强——when_to_use 字段、统一搜索 API、稳定 content hash

### Modified Capabilities

## Impact

- `crates/xiaolin-core/src/config.rs` — budget percent 默认值
- `crates/xiaolin-core/src/skill.rs` — frontmatter 新字段、搜索权重
- `crates/xiaolin-gateway/src/chat_pipeline.rs` — 注入逻辑重构（delta + 分离）
- `crates/xiaolin-agent/src/builtin_tools/skill.rs` — search 权重调整
- `crates/xiaolin-core/src/skill_usage.rs` — 精确注入记录
- `crates/xiaolin-gateway/src/state/mod.rs` — extension 扫描修复
- 前端无需变更（后端优化为主）
