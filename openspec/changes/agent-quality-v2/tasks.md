## 1. FileStateCache 接线 (P0)

- [ ] 1.1 `turn_setup.rs` 创建 `Arc<FileStateCache::new()>` 并存储到 `TurnServices` 或 setup 返回值
- [ ] 1.2 `execute_unified` 或 turn loop 中用 `with_file_state_cache(cache, ...)` 包裹工具执行 scope
- [ ] 1.3 验证 `read_file` 在同一 turn 内对未变文件返回 `FILE_UNCHANGED_STUB`
- [ ] 1.4 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 2. 路径容错 (P0)

- [ ] 2.1 `filesystem.rs` 新增 `find_similar_files(basename, workspace_root, max_depth=3, max_results=3)` 函数
- [ ] 2.2 `read_file` NotFound 分支调用 `find_similar_files`，将建议附加到错误信息
- [ ] 2.3 `edit_file` NotFound 分支同样附加路径建议
- [ ] 2.4 补充单元测试：有/无匹配文件、多匹配文件
- [ ] 2.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 3. edit_file 结构化 errorCode (P1)

- [ ] 3.1 定义 `EditErrorCode` 枚举（1=no_change, 4=not_found, 7=stale, 8=not_matched, 9=ambiguous）
- [ ] 3.2 修改 edit_file 各错误分支返回 JSON 格式：`{errorCode, errorType, recovery_hint, message}`
- [ ] 3.3 确保 LLM 可直接解析 errorCode 并按 recovery_hint 行动
- [ ] 3.4 补充单元测试覆盖所有 errorCode
- [ ] 3.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 4. Prompt cache 优化 (P1)

- [ ] 4.1 `chat_pipeline.rs` system prompt 组装时在 static/dynamic 之间插入 boundary marker
- [ ] 4.2 `llm_call.rs` 检测 boundary marker 并在 API 请求中设置 `cache_control` breakpoint
- [ ] 4.3 工具 definitions 排序保持 deterministic（按名称字母序）
- [ ] 4.4 验证连续两次 LLM 调用的 static prompt 部分字节相同
- [ ] 4.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 5. Benchmark 校准 (P0)

- [ ] 5.1 根据实测 prompt overhead (~27K/iter) 更新所有 task YAML 的 token budget
- [ ] 5.2 `tool_trace` grader 支持 `allowed_shell_patterns` 参数
- [ ] 5.3 更新 grader 逻辑区分 shell for build/test vs shell for file ops
- [ ] 5.4 运行全量 benchmark 验证校准后指标合理
- [ ] 5.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过
