## Context

当前 Git 能力完全依赖 Agent 通过 `shell_exec` 调用 git CLI。`worktree.rs` 中有 `run_git_command` helper 和 `is_git_repo` 检测，`shell_readonly.rs` 将 git 子命令分为 ReadOnly/Write/Dangerous 三类。但没有结构化的 Git 数据 API 供前端直接消费。

layout-overhaul 的 WorkspacePanel spec 定义了多标签右侧面板；其中 Review 标签页包含 staged/unstaged 文件列表和 inline diff 展示（当前为 mock 数据）。Header spec 定义了 git stats 和 Commit 按钮（预留），InputBar spec 定义了分支选择器（预留）。这些都需要实时的、结构化的 Git 状态数据。

`project-model` change 提供了 `Project.root_path`，作为 Git 操作的目标目录。

## Goals / Non-Goals

**Goals:**
- 提供结构化的 Git 查询 API（status、diff、branches、log），返回类型安全的 Rust struct
- 提供 Git 写操作 API（stage、unstage、commit、revert），含安全校验
- 实时监控 Git 状态变化，推送到前端
- 前端 Zustand store 管理 Git 状态，接入 layout-overhaul 的 UI 组件
- 非 git 项目优雅降级

**Non-Goals:**
- 替换 Agent 的 shell_exec git 能力——Agent 仍然可以自由使用 git CLI
- 支持 push、pull、merge、rebase——这些高风险操作由 Agent 通过 shell_exec 执行，走审批流程
- 使用 gix/gitoxide 替代 git CLI——初始版本全部使用 CLI，后续按需迁移
- 支持 SVN、Mercurial 等非 Git VCS
- Git 认证管理（SSH key、credential helper）

## Decisions

### D1: 全部使用 git CLI + porcelain 输出

**选择**：所有 Git 操作通过 `tokio::process::Command("git")` 执行，查询命令使用 `--porcelain` 格式

**替代方案**：
- gix（gitoxide）纯 Rust 库 → API 不成熟，增加大量编译时间，worktree 等功能支持不完整
- libgit2/git2-rs → C 绑定，增加链接复杂度，且已逐步被 gix 取代

**理由**：
- `worktree.rs` 已有成熟的 `run_git_command` helper 可以直接复用
- `git status --porcelain=v2` 格式稳定，跨 git 版本一致
- `git diff --numstat` 和 `git diff --unified` 输出格式简单，解析可靠
- 零新依赖，编译速度不受影响

### D2: Git 服务模块放在 xiaolin-tools-fs crate

**选择**：在 `xiaolin-tools-fs/src/` 中新增 `git.rs` 模块

**替代方案**：
- 新建 xiaolin-git crate → 过度拆分，git 操作本质上是文件系统操作
- 放在 xiaolin-core → 会引入 tokio::process 异步依赖

**理由**：`xiaolin-tools-fs` 已有 `worktree.rs`（git worktree 操作）和 `shell_readonly.rs`（git 命令分类），git 查询/写操作属于同一领域。复用已有的 `run_git_command` 和 `is_git_repo`。

### D3: 三级刷新策略

**选择**：
1. **notify 监控 .git 关键文件**（Level 1）：监控 `.git/HEAD`、`.git/index`、`.git/refs/heads/`，debounce 200ms 后执行 git status 并通过 ws_broadcast 推送
2. **Agent 操作后主动触发**（Level 2）：文件写工具（EditFile、WriteFile、ApplyPatch、ShellExec）执行成功后，检查 project root 是否是 git 仓库，是则触发 git status 刷新
3. **前端轮询兜底**（Level 3）：前端每 30 秒请求一次 git.status（如果最近 30 秒内已被 Level 1/2 刷新则跳过）

**替代方案**：
- 纯轮询（3 秒间隔）→ 高频查询浪费资源，且 3 秒延迟对 UI 不够即时
- 纯 notify 监控所有文件 → 大仓库文件变化频繁，噪声太大
- 纯 Agent 后触发 → 无法感知外部编辑器修改

**理由**：三级策略覆盖所有场景——Level 1 处理 git 命令操作（即时），Level 2 处理 Agent 文件修改（即时），Level 3 兜底外部编辑器修改（30 秒内）。notify 只监控 .git 目录（少量文件），不监控工作目录（避免大仓库性能问题）。

### D4: Per-project GitWatcher 生命周期

**选择**：每个活跃 project 维护一个独立的 GitWatcher 实例，存储在 `HashMap<ProjectId, GitWatcher>`

**理由**：
- 不同 project 的 .git 目录不同，需要独立的 notify watcher
- 当 project 变为非活跃时停止 watcher（释放 fd 和内存）
- 最多同时监控前端打开的 project（通常 1-2 个）
- GitWatcher 在 gateway 中作为 AppState 的一部分管理

### D5: Git 写操作安全等级

**选择**：
- `git.stage` / `git.unstage`：低风险，无需确认
- `git.commit`：中风险，需要 commit message，前端 UI 确认
- `git.revert`：高风险（丢失工作目录变更），前端 UI 必须二次确认

**理由**：与 `shell_readonly.rs` 中的安全分类一致。WorkspacePanel Review 标签页的 UI 交互已经包含了确认步骤（点击按钮 → 执行），高风险操作增加确认对话框。

### D6: 非 git 项目降级

**选择**：`git.status` API 对非 git 目录返回 `{ "isGitRepo": false }` 而非报错，前端隐藏所有 git 相关 UI 元素

**理由**：XiaoLin 不要求项目必须是 git 仓库。WorkspacePanel Review 标签页、Header stats、InputBar 分支选择器在非 git 项目中不显示或降级，不影响其他功能。

## Risks / Trade-offs

- **[git 版本兼容]** `--porcelain=v2` 需要 git >= 2.11 → 缓解：2.11 发布于 2016 年，绝大多数系统已满足。启动时检查 git 版本，不满足则禁用 git 功能并日志警告。
- **[大仓库性能]** git status 在 monorepo（> 100K 文件）中可能耗时 > 1 秒 → 缓解：结果缓存 + debounce，`--untracked-files=no` 选项减少扫描范围。
- **[并发安全]** 多个 git 操作同时执行可能冲突（index.lock）→ 缓解：写操作串行化（per-project mutex），查询操作并行安全。
- **[worktree 场景]** 在 worktree 中 `.git` 是一个文件而非目录 → 缓解：使用 `git rev-parse --git-dir` 获取实际 .git 目录路径。
- **[外部修改竞争]** 用户在终端手动 git commit 和前端 git.commit 同时进行 → 缓解：写操作使用 mutex，如果 git 报锁冲突则返回友好错误消息。

## Open Questions

- git.log 返回多少条记录？固定限制还是前端指定 limit？
- 是否需要支持查看特定 commit 的 diff（而不仅是 working tree diff）？
- Commit 操作是否需要支持 GPG 签名？
