## Why

Benchmark 数据显示 agent 在标准任务上 80% 功能完成率但存在系统性效率问题：首轮工具调用失败率高（50% 的任务首轮失败）、每次迭代固定开销约 27K tokens（system prompt + tool defs 过大）、`FileStateCache` 已实现但未接线导致无法避免重复读文件、edit_file 失败后缺乏结构化恢复引导。参考 claude-code 架构，本次改进针对这些具体瓶颈，预计可降低 avg tokens/iter 20%+ 并消除首轮失败。

## What Changes

- 接线 `FileStateCache`，启用 read dedup（未变文件返回 stub 而非完整内容）
- 文件工具路径容错：文件不存在时搜索相似路径并提供建议，而非直接拒绝
- System prompt 分层缓存：引入 static/dynamic boundary 标记，支持 LLM prompt cache
- Tool definitions session cache：按 session 稳定工具 schema，避免 prompt cache miss
- edit_file 结构化 errorCode：参考 claude-code 的 errorCode 1-9 体系，为每种失败类型提供精确恢复指引
- Benchmark token budget 校准：基于实测 prompt overhead 重新计算合理 budget

## Capabilities

### New Capabilities
- `file-state-cache-wiring`: 将已实现的 FileStateCache 接入 agent 运行时，启用 read dedup
- `path-error-recovery`: 文件路径不存在时搜索相似文件并提示，减少首轮失败
- `prompt-cache-optimization`: System prompt 分层（static/dynamic）+ tool schema session cache
- `edit-structured-errors`: edit_file 结构化错误码体系，精确引导 LLM 恢复
- `benchmark-calibration`: 校准 benchmark token budget 和 grader 精细度

### Modified Capabilities

## Impact

- `crates/xiaolin-tools-fs/src/filesystem.rs` — FileStateCache 接线、路径容错
- `crates/xiaolin-agent/src/runtime/turn_setup.rs` — FileStateCache 初始化
- `crates/xiaolin-agent/src/runtime/mod.rs` — prompt cache boundary
- `crates/xiaolin-core/src/tool.rs` — schema cache 支持
- `crates/xiaolin-tools-fs/src/file_state_cache.rs` — dedup 逻辑增强
- `benchmarks/tasks/*.yaml` — budget 校准
- System prompt 文件 — 分层标记
