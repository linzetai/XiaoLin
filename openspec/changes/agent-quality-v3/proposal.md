## Why

Benchmark 分析显示 agent 成功率仅 30%（3/10），核心瓶颈是：首轮工具调用路径错误（浪费 1-2 轮）、缺乏工具选择决策框架（agent 随机选工具）、缺少项目上下文（agent 不知 git 状态和文件结构）。参考 claude-code 的实现，通过三项改进可显著提升 agent 首次调用成功率和整体任务完成率。

## What Changes

- **System prompt 增加 Tool Selection Decision Tree**：在 prompt 中加入 Step 0-3 工具选择决策树和 few-shot 示例，引导 agent 在首轮就选对工具（如"查找文件用 Glob 不用 Bash"）
- **工具层路径纠错增强**：新增 `suggest_path_under_cwd()` 函数，修复 agent 常见的"漏掉 repo 目录"路径错误；结合已有的 `find_similar_files()`，在 FileNotFound 错误中提供精确的路径建议
- **Git 快照注入到首轮上下文**：在 turn_setup 阶段自动注入当前分支、`git status --short`、最近 5 条 commit，让 agent 零成本获取项目上下文
- **"搜索后再说不知道"规则**：在 system prompt 中明确要求 agent 先用 glob/search 搜索后，才能声称文件不存在

## Capabilities

### New Capabilities
- `tool-selection-guidance`: System prompt 中的工具选择决策树和 few-shot 示例，降低首轮工具选择错误率
- `path-error-recovery`: 工具层增强路径纠错，包括 suggest_path_under_cwd 和改进的 did-you-mean 提示
- `git-context-injection`: 首轮自动注入 git 快照（branch/status/commits），提供项目感知

### Modified Capabilities

## Impact

- `crates/xiaolin-agent/src/runtime/prompt_sections/` — 新增/修改 prompt sections
- `crates/xiaolin-agent/src/runtime/turn_setup.rs` — git 快照注入逻辑
- `crates/xiaolin-agent/src/runtime/context_assembly.rs` — git 快照采集
- `crates/xiaolin-tools-fs/src/filesystem.rs` — suggest_path_under_cwd 函数 + 错误消息增强
- `benchmarks/tasks/**/*.yaml` — 可能需要调整 benchmark 阈值以反映改进
