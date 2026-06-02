# Proposal: 移除 Git Tool，统一使用 Shell

## 背景

当前 `GitTool`（`crates/xiaolin-agent/src/builtin_tools/git.rs`）是一个独立的内置工具，封装了 status/diff/log/branch/show/stash_list 6 个子命令，返回结构化 JSON。

实际使用中，这层封装带来的问题大于收益：

1. **维护负担高**：需要解析 git porcelain 输出、处理各种边界情况（UTF-8 截断、多行 body、numstat 解析），容易出 bug
2. **功能受限**：只封装了 6 个读操作，LLM 实际需要的 git 操作远不止这些（cherry-pick、rebase、blame、stash pop、bisect 等），每新增一个都要写解析代码
3. **重复能力**：Agent 已有 `shell_exec` 可直接执行任何 git 命令，git CLI 本身的输出对 LLM 足够可读
4. **prompt 引导更灵活**：与其维护结构化 wrapper，不如通过 system prompt 教 LLM 如何用 `--format`、`--stat`、`--numstat` 等选项高效获取信息

## 目标

- 移除 `GitTool` 内置工具，减少代码维护量
- Agent 通过 `shell_exec` 执行 git 命令，功能更完整
- 保持 git 操作能力不降级

## 方案

1. 从 builtin_tools 注册表中移除 `GitTool`
2. 删除 `git.rs` 源文件
3. 清理 `mod.rs` 中的 `mod git` / `pub use` / 注册调用
4. 删除 `optimize-git-tool` change（之前创建的已无意义）

## 不做的事

- 不需要在 system prompt 中加 git 指引（LLM 本身就会用 git）
- 不影响 shell_exec 的审批机制（git 命令仍走正常审批流程）

## 影响范围

- `crates/xiaolin-agent/src/builtin_tools/git.rs` — 删除
- `crates/xiaolin-agent/src/builtin_tools/mod.rs` — 清理引用
- tool_count 减 1（85 → 84）
- 无协议变更、无前端变更
