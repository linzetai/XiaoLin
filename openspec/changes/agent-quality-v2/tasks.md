## 1. FileStateCache 接线 (P0)

- [x] 1.1 `stream_loop.rs` 创建 `Arc<FileStateCache::new()>` 并通过 `with_file_state_cache` scope 整个 turn
- [x] 1.2 验证 `read_file`/`write_file`/`edit_file` 中已有完整的 cache 读写逻辑（dedup + stale detection）
- [x] 1.3 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 2. 路径容错 (P0)

- [x] 2.1 `filesystem.rs` 新增 `find_similar_files(basename, root, max_depth, max_results)` + `format_not_found_with_suggestions`
- [x] 2.2 `read_file` NotFound 分支调用 `find_similar_files`，将建议附加到错误信息
- [x] 2.3 `edit_file` NotFound 分支同样附加路径建议
- [x] 2.4 补充 6 个单元测试：有/无匹配、max_results、跳过 .git/node_modules、格式化
- [x] 2.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 3. edit_file 结构化 errorCode (P1)

- [x] 3.1 定义 `EditErrorCode` 枚举（1=NoChange, 3=FileExists, 4=NotFound, 7=Stale, 8=NotMatched, 9=Ambiguous）
- [x] 3.2 修改 edit_file 12 个错误分支返回 JSON：`{errorCode, errorType, file, recovery_hint, message}`
- [x] 3.3 每个 errorCode 有明确的 recovery_hint，LLM 可直接按提示行动
- [x] 3.4 补充 4 个单元测试：JSON 字段校验、全枚举 JSON 解析、数值校验、特殊字符转义
- [x] 3.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 4. Prompt cache 优化 (P1)

- [x] 4.1 `PromptEngine` 已有 `DYNAMIC_BOUNDARY` 标记分隔 static/dynamic sections
- [x] 4.2 `mod.rs` 在组装 messages 时按 `DYNAMIC_BOUNDARY` 拆分为两个独立 system messages（static + dynamic）
- [x] 4.3 `turn_setup.rs` 工具 definitions 在 filter 后按 `function.name` 字母序排序
- [x] 4.4 `CacheBreakDetector` + prompt_engine 测试 + cache_break 测试全部通过，验证 static 部分稳定性
- [x] 4.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过

## 5. Benchmark 校准 (P0)

- [x] 5.1 根据实测 prompt overhead (~27K/iter) 校准 7 个 YAML 的 token_budget（30K→130K, 50K→200K, 80K→160K, 120K→260K×3）
- [x] 5.2 `tool_trace` grader 新增 `allowed_shell_patterns` 参数（serde default）
- [x] 5.3 grader 逻辑：当 `allowed_shell_patterns` 非空时跳过 `shell_exec` 禁止检查
- [x] 5.4 新增 `tool_trace_allowed_shell_patterns_bypass` 单元测试
- [x] 5.5 `cargo check` + `cargo clippy -- -D warnings` + `cargo test` 通过（22/22 benchmark 测试）
