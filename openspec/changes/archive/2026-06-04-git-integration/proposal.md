## Why

layout-overhaul 原型图的 WorkspacePanel Review 标签页（文件变更列表、inline diff、Stage/Revert 操作）、Header（git stats、Commit 按钮）、InputBar（分支选择器）都需要结构化的 Git 数据作为后端支撑。当前系统仅通过 shell_exec 调用 git CLI，没有专门的 Git 数据 API。Agent 可以读取 git status，但前端 UI 无法获取结构化的 Git 状态数据。此外，当文件被修改（Agent 操作或外部编辑）时，前端无法实时感知 Git 状态变化。

此 change 依赖 `project-model`（提供 Project.root_path 作为 git 操作的目标目录）。

## What Changes

- **Git 查询服务**：新增 Rust 模块提供结构化 Git 查询——current_branch、branch_list、status（staged/unstaged 分组）、diff（per-file hunks）、diff_stat、log。内部通过 git CLI porcelain 格式解析
- **Git 写操作**：新增 stage、unstage、commit、revert 操作，每个操作完成后自动触发状态刷新
- **Git 实时监控**：使用 notify crate 监控 `.git/HEAD`、`.git/index`、`.git/refs/heads/` 变化，debounce 200ms 后推送 `git.status_changed` 事件；Agent 文件写操作后主动触发；前端 30 秒轮询兜底
- **Git WebSocket API**：新增 `git.*` 方法族供前端调用
- **前端 git-store**：Zustand store 管理 Git 状态，接入 WorkspacePanel Review 标签页、Header、InputBar 的实际数据

## Capabilities

### New Capabilities
- `git-query-service`: Git 查询服务——current_branch、branch_list、status、diff、diff_stat、log 的 Rust 实现，使用 git CLI porcelain 输出格式，返回类型安全的 struct
- `git-mutate-operations`: Git 写操作——stage、unstage、commit、revert，含操作后自动触发状态刷新，写操作安全校验
- `git-watcher`: Git 状态实时监控——notify 监控 .git 关键文件、debounce、per-project watcher 生命周期管理、Agent 操作后主动触发
- `git-websocket-api`: Git WebSocket API——git.status / git.diff / git.stage / git.unstage / git.commit / git.revert / git.branches / git.log 方法定义与处理器
- `git-frontend-store`: 前端 Git 状态管理——Zustand store、事件监听、WorkspacePanel Review 标签页 / Header / InputBar 数据接入

### Modified Capabilities
- `workspace-panel`: WorkspacePanel 的 Review 标签页从 mock 数据切换到真实 Git 数据，文件变更列表和 inline diff 从 git-store 获取，Stage/Revert/Commit 按钮执行真实操作；非活跃 Review 标签在 git 状态变更时显示通知角标
- `chat-input-bar`: InputBar 底部的分支选择器从预留改为从 git-store 获取当前分支和分支列表

## Impact

- **后端 crates**：`xiaolin-tools-fs`（新增 git 模块，复用已有的 `run_git_command` helper）、`xiaolin-gateway`（新增 ws/git.rs 处理器 + git watcher 服务）、`xiaolin-protocol`（新增 Git 相关 op 定义和 TypeScript 类型）
- **前端**：新增 `git-store.ts`、修改 WorkspacePanel Review 标签页和 InputBar 组件接入真实数据
- **依赖**：无新增 Rust crate（使用现有 notify v6 + tokio::process::Command）
- **兼容性**：向后兼容——非 git 项目中 git.* API 返回空数据，UI 降级为隐藏 git 相关元素
