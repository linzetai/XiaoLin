## 1. Git 查询服务（Rust）

- [x] 1.1 在 `xiaolin-tools-fs/src/` 中创建 `git.rs` 模块，将 `worktree.rs` 中的 `run_git_command` 和 `is_git_repo` 提取为公共 helper
- [x] 1.2 定义数据结构：`GitStatus`、`FileChange`（path, status, old_path）、`DiffStat`、`DiffHunk`、`DiffLine`、`Branch`、`CommitSummary`
- [x] 1.3 实现 `resolve_git_dir(dir) -> PathBuf`（处理 worktree 场景，调用 `git rev-parse --git-dir`）
- [x] 1.4 实现 `current_branch(dir) -> String`（解析 `git rev-parse --abbrev-ref HEAD`）
- [x] 1.5 实现 `branch_list(dir) -> Vec<Branch>`（解析 `git branch -a --format`）
- [x] 1.6 实现 `git_status(dir) -> GitStatus`（解析 `git status --porcelain=v2 --branch`，分离 staged/unstaged）
- [x] 1.7 实现 `diff_stat(dir) -> DiffStat`（解析 `git diff --stat --numstat`）
- [x] 1.8 实现 `file_diff(dir, path, staged) -> Vec<DiffHunk>`（解析 `git diff [--cached] -- <path>` 的 unified diff 输出）
- [x] 1.9 实现 `git_log(dir, limit) -> Vec<CommitSummary>`（解析 `git log --format --shortstat -n <limit>`）
- [x] 1.10 在 `xiaolin-tools-fs/src/lib.rs` 中导出 `pub mod git`

## 2. Git 写操作（Rust）

- [x] 2.1 在 `git.rs` 中实现 `git_stage(dir, files) -> Result`
- [x] 2.2 实现 `git_unstage(dir, files) -> Result`（使用 `git restore --staged`）
- [x] 2.3 实现 `git_commit(dir, message) -> CommitResult`（包含新 commit SHA）
- [x] 2.4 实现 `git_revert_files(dir, files) -> Result`（`git checkout --` 已跟踪文件 + 删除未跟踪文件）
- [x] 2.5 实现 per-project write mutex：`GitWriteLock` 使用 `tokio::sync::Mutex` 确保写操作串行化
- [x] 2.6 实现 `wait_for_git_lock(dir, timeout)` 检测 `.git/index.lock` 并等待

## 3. Git Watcher 服务

- [x] 3.1 在 `xiaolin-gateway/src/` 中创建 `git_watcher.rs`，定义 `GitWatcher` struct（持有 notify watcher + debounce channel）
- [x] 3.2 实现 `GitWatcher::new(project_id, git_dir, ws_broadcast)` — 创建 notify watcher 监控 .git/HEAD、.git/index、.git/refs/heads/
- [x] 3.3 实现 debounce 逻辑：200ms quiet period 后执行 git_status 并通过 ws_broadcast 发送 `git.status_changed` 事件
- [x] 3.4 实现 `GitWatcherManager`：`HashMap<ProjectId, GitWatcher>`，提供 `ensure_watcher(project_id)` 和 `stop_watcher(project_id)`
- [x] 3.5 在 AppState 中添加 `git_watcher_manager: Arc<GitWatcherManager>` 字段
- [x] 3.6 在 session 激活/切换时调用 `ensure_watcher` 启动对应 project 的 git 监控

## 4. Agent 操作后触发

- [x] 4.1 在文件写工具（EditFile、WriteFile、ApplyPatch）的成功回调中，检查文件是否在某个活跃 project 的 git 仓库中
- [x] 4.2 如果是，调用 `GitWatcherManager::trigger_refresh(project_id)` 触发 git status 刷新（debounced）
- [x] 4.3 在 shell_exec 工具的成功回调中，如果命令包含 git 写操作子命令（add/commit/checkout 等），同样触发刷新

## 5. Protocol 定义

- [x] 5.1 在 `xiaolin-protocol/src/op.rs` 中为 `ClientOp` 添加 `GitStatus`、`GitDiff`、`GitBranches`、`GitLog`、`GitStage`、`GitUnstage`、`GitCommit`、`GitRevert` 变体
- [x] 5.2 在 `ClientOp::from_typed` 中添加 `git.*` 方法的解析逻辑
- [x] 5.3 在 `xiaolin-protocol/generated/protocol.ts` 中添加 `GitStatus`、`FileChange`、`DiffHunk`、`DiffLine`、`Branch`、`CommitSummary`、`DiffStats` TypeScript 接口

## 6. Gateway WebSocket 处理器

- [x] 6.1 创建 `xiaolin-gateway/src/ws/git.rs`，实现 `handle_git_status`
- [x] 6.2 实现 `handle_git_diff`
- [x] 6.3 实现 `handle_git_branches`
- [x] 6.4 实现 `handle_git_log`
- [x] 6.5 实现 `handle_git_stage`
- [x] 6.6 实现 `handle_git_unstage`
- [x] 6.7 实现 `handle_git_commit`
- [x] 6.8 实现 `handle_git_revert`
- [x] 6.9 在 `xiaolin-gateway/src/ws/mod.rs` 中添加 `git.*` 路由分发

## 7. 前端 API 与 Store

- [x] 7.1 在 `xiaolin-app/src/lib/api.ts` 中添加 `gitStatus`、`gitDiff`、`gitBranches`、`gitLog`、`gitStage`、`gitUnstage`、`gitCommit`、`gitRevert` 函数
- [x] 7.2 在 `xiaolin-app/src/lib/transport.ts` 中添加 `git.status_changed` 事件监听
- [x] 7.3 创建 `xiaolin-app/src/lib/stores/git-store.ts`，定义 `useGitStore` Zustand store
- [x] 7.4 实现 store 的自动跟踪活跃 project（监听 project-store 的 activeProjectId 变化）
- [x] 7.5 实现 30 秒轮询 fallback 逻辑
- [x] 7.6 在 `stores/index.ts` 中导出 `useGitStore`

## 8. 前端 UI 接入

- [x] 8.1 在 WorkspacePanel 中注册 Review 标签页（`ReviewTabContent` / `ReviewTabFooter`），文件变更列表从 mock 改为 git-store 的 staged/unstaged
- [x] 8.2 修改 Review 标签页 diff 展示：从 mock 改为 git-store 的 selectedDiff
- [x] 8.3 修改 Review 标签页操作按钮：Stage All 调用 git-store.stageFiles、Revert All 增加确认框后调用 git-store.revertFiles
- [x] 8.4 为 unstaged 文件增加 "+" stage 按钮、为 staged 文件增加 "-" unstage 按钮
- [x] 8.5 修改 InputBar 底部：分支 chip 从硬编码改为从 git-store.branch 获取，非 git 项目隐藏
- [x] 8.6 修改 Header：git stats 区域从预留改为从 git-store.stats 获取（files changed +N -N）
- [x] 8.7 在 WorkspacePanel Review 标签页和 InputBar 中处理 isGitRepo=false 的降级展示
- [x] 8.8 当 Review 标签非活跃且 git 状态有变更时，在 Review 标签图标上显示通知角标，切换到 Review 后清除

## 9. 集成与验证

- [x] 9.1 运行 `cargo check` 确认 Rust 编译通过
- [x] 9.2 运行 `cargo clippy -- -D warnings` 确认无警告
- [x] 9.3 为 git_status 解析逻辑编写单元测试（mock porcelain v2 输出）
- [x] 9.4 为 file_diff 解析逻辑编写单元测试（mock unified diff 输出）
- [x] 9.5 端到端验证：修改文件 → WorkspacePanel Review 标签页实时显示变更 → Stage → Commit → 状态刷新；非活跃 Review 标签显示角标
