## Context

Benchmark 数据显示 agent 在 10 个任务中只有 3 个通过（30%），主要失败原因：
- 首轮工具调用路径错误率高（~50% 的任务首次 read_file 失败），浪费 1-2 轮
- 缺乏结构化工具选择指南，agent 随机选择 list_directory/glob/read_file
- 项目上下文不足，agent 不知道当前在哪个分支、有哪些修改

参考 claude-code 实现：不注入 file tree（token 开销大且易过时），靠 prompt 引导 + 工具层纠错 + 轻量 git 快照。

## Goals / Non-Goals

**Goals:**
- 通过 prompt 引导将首轮工具选择正确率从 ~50% 提升到 ~80%+
- 通过路径纠错将 FileNotFound 错误的恢复从 2 轮减少到 1 轮
- 通过 git 快照让 agent 对项目状态有即时感知
- Benchmark 通过率从 30% 提升到 50%+

**Non-Goals:**
- 不注入完整 file tree（token 开销过大，不是 claude-code 的做法）
- 不修改 LLM 调用参数（temperature 等）
- 不重构工具注册/执行框架
- 不优化 system prompt 总体积（v2 已做过）

## Decisions

### Decision 1: Tool Selection Decision Tree 放在 system prompt 哪里

**选择**: 放在 `prompt_sections/mod.rs` 的 tool_guidance section 中（已有），作为 static section（可缓存）

**理由**: 
- 决策树是通用规则，不随会话变化，适合缓存
- 放在工具指南部分语义自然，模型能关联到工具使用
- 替代方案：放在 `system-base.md` 中 → 但那是用户可覆盖的，我们需要硬编码

**具体内容**:
```
Step 0: 需要工具吗？如果可以直接回答，就不要调用工具
Step 1: 有专用工具吗？read_file/edit_file/glob/search_in_files 总是优于 shell_exec
Step 2: 需要发现文件位置？先 glob 再 read_file（不要猜路径直接 read）
Step 3: 需要搜索内容？用 search_in_files（不要 shell grep）
```

加 few-shot 示例："找所有 .tsx 文件" → glob, 不是 bash find

### Decision 2: suggest_path_under_cwd 的实现策略

**选择**: 在 `filesystem.rs` 的 `ensure_within_workspace` 失败路径中嵌入，与已有 `find_similar_files` 互补

**算法**: 
1. 请求路径在 cwd 的 parent 目录下，但不在 cwd 下 → 尝试 join(cwd, relative_from_parent)
2. 如果修正后的路径存在 → 返回 "Did you mean {corrected}?"
3. 否则 fallback 到 find_similar_files

**理由**: claude-code 的 `suggestPathUnderCwd` 修复的是 monorepo 场景下 LLM 常漏掉 repo 目录的问题，这在 benchmark 的 temp 目录场景同样适用

### Decision 3: Git 快照的采集和注入位置

**选择**: 在 `context_assembly.rs` 新增 `collect_git_snapshot()` 函数，在 `turn_setup.rs` 的 project hints 之后注入

**采集内容**:
- `git branch --show-current` → 当前分支
- `git status --short` → 修改文件列表（截断到 2000 字符）
- `git log --oneline -5` → 最近 5 条 commit

**注入格式**:
```
─── Git Context ───
Branch: main
Status:
 M src/lib.rs
 M src/main.rs
Recent commits:
abc1234 fix: resolve path error
def5678 feat: add git snapshot
───────────────────
```

**理由**: 轻量（通常 < 500 字符），对理解项目当前状态极有价值，claude-code 验证过这个策略

### Decision 4: "搜索后再说不知道"规则

**选择**: 在 tool_guidance section 中追加规则："在声称文件/函数不存在之前，必须至少执行一次 glob 或 search_in_files 搜索"

**理由**: benchmark 中多次出现 agent 不搜索就声称文件不存在然后用 write_file 创建的情况

## Risks / Trade-offs

- **[Risk] Decision Tree 增加 system prompt token** → 约增加 300-500 tokens（< 2% 增幅），可接受
- **[Risk] Git 快照在非 git 目录下会失败** → 已有 `is_git` 检测，仅在 git repo 中注入
- **[Risk] suggest_path_under_cwd 误判** → 只在路径确实在 parent 目录下且修正后路径存在时才建议，误报概率低
- **[Risk] Few-shot 示例过多会过度约束模型行为** → 控制在 3-5 个示例内
