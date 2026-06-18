## Context

Benchmark v3 后的分析揭示了几个系统性问题：

- **首轮工具调用失败率 50%**：5/10 个任务在 iter 1 出现 `read_file`/`list_directory` 失败，通常 iter 2 重试成功。每次首轮失败浪费 ~27K tokens。
- **每迭代固定开销 ~27K tokens**：system prompt + tool definitions 占据大量空间，即使最简单的任务（加一行注释）也需要 109K tokens。
- **FileStateCache 已实现但未接线**：`with_file_state_cache` 从未被调用，read dedup 完全不工作。
- **edit_file 失败后缺乏恢复引导**：错误信息是纯文本，LLM 难以精确判断失败类型和恢复策略。

参考 claude-code 的设计：双层 description（短 `description()` + 长 `prompt()`）、`expandPath` 路径容错、`FILE_UNCHANGED_STUB` read dedup、结构化 errorCode、`SYSTEM_PROMPT_DYNAMIC_BOUNDARY` 缓存分层。

## Goals / Non-Goals

**Goals:**
- 消除首轮工具调用系统性失败（50% → 0%）
- 启用 FileStateCache read dedup，减少重复读文件的 token 消耗
- 降低 avg tokens/iter 15-20%（27K → 22K）
- edit_file 失败后 LLM 可通过结构化 errorCode 精确恢复
- Benchmark token budget 与实际 prompt overhead 对齐

**Non-Goals:**
- 不重构 system prompt 文本内容（仅加分层标记）
- 不修改 LLM API 调用协议
- 不引入新的工具
- 不改变 tool_search / deferred 机制（agent-capability-boost 已完成）

## Decisions

### D1: FileStateCache 接线方式 — task-local scope

**选择**: 在 `turn_setup.rs` 创建 `Arc<FileStateCache>`，通过 `with_file_state_cache` scope 整个 turn 执行。

**替代方案**: 放入 `TurnServices` 结构体。  
**不选原因**: 现有文件工具通过 task-local 访问，改为 struct field 需要修改所有工具签名。task-local scope 与现有 `with_work_dir` / `with_file_access_mode` 一致。

**实现要点**:
- `turn_setup.rs` 创建 `Arc<FileStateCache::new()>`
- `turn_loop` 或 `execute_unified` 中用 `with_file_state_cache(cache.clone(), async { ... }).await` 包裹整个执行
- FileStateCache 实例生命周期 = 一个 turn（session 级复用可后续考虑）

### D2: 路径容错 — `findSimilarFile` 提示

**选择**: `read_file`/`edit_file` 文件不存在时，在工作目录下搜索文件名匹配（basename 相同）的文件并在错误信息中提示。

**替代方案**: 自动跟随到正确路径。  
**不选原因**: 自动行为有安全风险（可能读取非预期文件），提示模式让 LLM 自行决策更安全。

**实现要点**:
- `filesystem.rs` 中 `read_file` / `edit_file` 的 NotFound 分支增加 `find_similar_files(basename, workspace_root)` 调用
- 返回 `"File not found: 'src/main.rs'. Did you mean: '/tmp/xxx/src/main.rs'?"` 格式
- 搜索范围限于 workspace root 下的前 3 层目录，限制结果数量（最多 3 个建议）

### D3: System prompt 分层 — boundary marker

**选择**: 在 system prompt 中插入 `<!-- CACHE_BOUNDARY -->` 标记，将 static 部分（角色、规范、决策树）和 dynamic 部分（MCP、env、memory）分开。LLM API 调用时可利用此标记实现 prompt cache breakpoint。

**实现要点**:
- `system-base.md` 保持不变（static）
- `chat_pipeline.rs` 组装 system prompt 时在 static 和 dynamic 内容之间插入 boundary
- LLM 调用层（`llm_call.rs`）识别 boundary 标记并设置 cache_control

### D4: edit_file 结构化 errorCode

**选择**: 参考 claude-code 的 errorCode 体系，在 edit_file 错误返回中附加结构化 JSON（`errorCode` + `recovery_hint`）。

| errorCode | 场景 | recovery_hint |
|-----------|------|--------------|
| 1 | old_string == new_string | 修改 new_string 使其不同 |
| 3 | 空 old_string 但文件非空 | 使用 edit 而非 write |
| 4 | 文件不存在 | 检查路径 + did you mean |
| 7 | 文件被外部修改 | 重新 read_file |
| 8 | old_string 未找到 | 检查上下文或使用 search_in_files |
| 9 | 多处匹配 | 增加上下文或使用 replace_all |

### D5: Benchmark budget 校准

**选择**: 基于实测 prompt overhead（~27K tokens/iter）重新计算 budget。

公式: `budget = prompt_overhead * expected_turns * 1.2`（20% 余量）

| 任务 | 当前 budget | 校准 budget |
|------|-----------|-----------|
| simple-task-budget | 30K | 130K (4 turns) |
| analyze-architecture | 80K | 160K (5 turns) |
| add-delete-feature | 120K | 260K (8 turns) |
| multi-step-efficiency | 50K | 200K (6 turns) |

## Risks / Trade-offs

- **[FileStateCache scope 过短]** → turn 级实例意味着跨 turn 不共享。如果需要 session 级 dedup，后续需要提升 scope 到 session 存储。Mitigation: turn 内已覆盖大部分重复读场景。
- **[findSimilarFile 性能]** → 目录遍历可能在大型项目中较慢。Mitigation: 限制搜索深度（3 层）和结果数量（3 个）。
- **[Prompt cache 效果依赖 API 支持]** → 不同 LLM provider 对 prompt cache 的支持程度不同。Mitigation: boundary marker 是无侵入的标记，不支持缓存时无副作用。
- **[Budget 校准使 benchmark 更宽松]** → 可能隐藏真正的效率问题。Mitigation: 同时引入 completion_tokens 和 avg_tokens_per_iter 作为追踪指标。
