## 1. Tool Selection Decision Tree

- [x] 1.1 在 `prompt_sections/mod.rs` 的 tool_guidance section 中添加 Step 0-3 决策树文本
- [x] 1.2 添加 3-5 个 few-shot 工具选择示例（glob vs bash find, search_in_files vs shell grep 等）
- [x] 1.3 添加"搜索后再说不知道"规则
- [x] 1.4 cargo check + clippy 验证

## 2. Path Error Recovery 增强

- [x] 2.1 在 `filesystem.rs` 中实现 `suggest_path_under_cwd()` 函数
- [x] 2.2 修改 `read_file` 的 FileNotFound 错误处理，集成 suggest_path_under_cwd + find_similar_files
- [x] 2.3 修改 `edit_file` 的 FileNotFound 错误处理，同样集成
- [x] 2.4 错误消息中包含 "Current working directory: {cwd}" 信息
- [x] 2.5 编写单元测试验证 suggest_path_under_cwd 逻辑
- [x] 2.6 cargo check + clippy 验证

## 3. Git 快照注入

- [x] 3.1 在 `context_assembly.rs` 中实现 `collect_git_snapshot()` 函数（branch + status + commits）
- [x] 3.2 添加 2000 字符截断逻辑
- [x] 3.3 在 `turn_setup.rs` 中调用 collect_git_snapshot 并注入到 messages
- [x] 3.4 cargo check + clippy 验证

## 4. 验证

- [x] 4.1 运行 `cargo test --workspace` 确保无回归（26 个相关测试全通过，6 个已有失败与本次无关）
- [x] 4.2 运行 benchmark 对比 post-quality-v2 基线（通过率 3/10→4/10，extract-display-trait 从超时恢复，multi-step 翻转为 PASS）
