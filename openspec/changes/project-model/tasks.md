## 1. 数据库 Schema 与迁移

- [x] 1.1 在 `xiaolin-session/src/store.rs` 的 `initialize()` 中添加 `CREATE TABLE IF NOT EXISTS projects` 语句（id, name, root_path, color, pinned, archived, created_at, last_opened_at）
- [x] 1.2 在 `xiaolin-session/src/store.rs` 的 migration 逻辑中添加 `sessions` 表 `project_id TEXT` 列检测和 ALTER TABLE
- [x] 1.3 在 `xiaolin-session/src/models.rs` 中定义 `Project` struct（derive FromRow、Serialize、Deserialize）
- [x] 1.4 在 `xiaolin-session/src/models.rs` 中为 `Session` 和 `SessionSummary` struct 添加 `project_id: Option<String>` 字段

## 2. 项目注册表核心逻辑

- [x] 2.1 在 `xiaolin-core/src/` 中创建 `project.rs` 模块，定义 `generate_project_id(root_path) -> String`（SHA-256 前 16 hex）和 `ProjectConfig` struct
- [x] 2.2 在 `xiaolin-core/src/project.rs` 中实现 `load_project_config(root_path) -> ProjectConfig`（读取 `.xiaolin/project.json`）
- [x] 2.3 在 `xiaolin-core/src/project.rs` 中实现 `write_project_config(root_path, config)`
- [x] 2.4 在 `xiaolin-core/src/lib.rs` 中导出 `pub mod project`

## 3. SessionStore 项目 CRUD 方法

- [x] 3.1 在 `xiaolin-session/src/store.rs` 中实现 `create_project(root_path, name, color) -> Project`（含路径规范化、ID 生成、去重检测）
- [x] 3.2 实现 `find_or_create_project(work_dir) -> Project`（直接使用传入路径，不调用 detect_workspace_root）
- [x] 3.3 实现 `list_projects(include_archived: bool) -> Vec<Project>`（含 session_count 子查询）
- [x] 3.4 实现 `get_project(id) -> Option<Project>`
- [x] 3.5 实现 `update_project(id, patch) -> Result<()>`（支持 name/color/pinned/archived 部分更新）
- [x] 3.6 实现 `delete_project(id) -> Result<()>`（级联清除 sessions.project_id）
- [x] 3.7 实现 `update_project_last_opened(id)`

## 4. Session-Project 绑定逻辑

- [x] 4.1 修改 `create_session_with_work_dir` 和 `create_session_full`：当 work_dir 非空时调用 `find_or_create_project` 并设置 project_id
- [x] 4.2 修改 `update_work_dir`：当 work_dir 变更时同步更新 project_id（或清空）— 通过 `handle_sessions_set_work_dir` 和 `setup_chat` 实现
- [x] 4.3 实现启动迁移逻辑：在 gateway builder 中扫描 `sessions WHERE work_dir IS NOT NULL AND project_id IS NULL`，按 unique work_dir 批量 find_or_create_project 并回填 project_id

## 5. Protocol 定义

- [x] 5.1 在 `xiaolin-protocol/src/op.rs` 中为 `ClientOp` 添加 `ProjectsList`、`ProjectsCreate`、`ProjectsUpdate`、`ProjectsDelete`、`ProjectsDetect` 变体
- [x] 5.2 在 `ClientOp::from_typed` 解析中添加 `projects.*` 方法的匹配逻辑
- [x] 5.3 在 `xiaolin-protocol/generated/protocol.ts` 中添加 `Project` 和 `BackendProject` TypeScript 接口
- [x] 5.4 修改 `BackendSession` TypeScript 接口，添加 `projectId?: string | null` 字段

## 6. Gateway WebSocket 处理器

- [x] 6.1 创建 `xiaolin-gateway/src/ws/project.rs`，实现 `handle_projects_list`
- [x] 6.2 实现 `handle_projects_create`
- [x] 6.3 实现 `handle_projects_update`
- [x] 6.4 实现 `handle_projects_delete`
- [x] 6.5 实现 `handle_projects_detect`（调用 detect_workspace_root + detect_project_hints）
- [x] 6.6 在 `xiaolin-gateway/src/ws/mod.rs` 中添加 `projects.*` 路由分发
- [x] 6.7 修改 `handle_sessions_set_work_dir`：在更新 work_dir 后同步调用 find_or_create_project 并更新 project_id
- [x] 6.8 修改 sessions.list / sessions.get 响应，包含 project_id 字段

## 7. 前端 API 层

- [x] 7.1 在 `xiaolin-app/src/lib/transport.ts` 中添加 `listProjects`、`createProject`、`updateProject`、`deleteProject`、`detectProject` 函数
- [x] 7.2 在 `xiaolin-app/src/lib/transport.ts` 中添加 `projects.changed` 事件监听

## 8. 前端 Project Store

- [x] 8.1 创建 `xiaolin-app/src/lib/stores/project-store.ts`，定义 `Project` 接口和 `useProjectStore` Zustand store
- [x] 8.2 实现 `syncProjects` action（处理 projects.list 响应）
- [x] 8.3 实现 `createProject`、`updateProject`、`deleteProject` actions（调用 API 并乐观更新）
- [x] 8.4 实现 `activeProjectId` 派生逻辑（跟随 active session 的 projectId）
- [x] 8.5 在 `stores/index.ts` 中导出 `useProjectStore`

## 9. 前端 ChatMeta 集成

- [x] 9.1 在 `types.ts` 的 `ChatMeta` 和 `BackendSession` 接口中添加 `projectId: string | null` 字段
- [x] 9.2 修改 `chat-meta-store.ts` 的 `createChatMeta` 添加 `projectId: null` 默认值
- [x] 9.3 修改 `syncSessionsForAgent`：从 backend session 同步 projectId
- [x] 9.4 修改 `AppSidebar.tsx` 的 `groupedChats`：按 projectId 分组，无 projectId 时显示在 Chats 区块

## 10. 集成与验证

- [x] 10.1 在 gateway 启动流程中调用项目迁移逻辑（task 4.3）
- [x] 10.2 在 store 初始化中触发 projects.list 同步 + 订阅 projects.changed 事件
- [x] 10.3 运行 `cargo check` 确认 Rust 编译通过
- [x] 10.4 运行 `cargo clippy -- -D warnings` 确认无警告
- [x] 10.5 验证端到端流程：创建 session 时自动创建 project、sidebar 正确分组、project CRUD 操作正常
