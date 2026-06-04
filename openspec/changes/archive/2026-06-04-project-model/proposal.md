## Why

当前 XiaoLin 没有正式的 Project 实体——"项目"仅是从 session 的 `work_dir` 路径末段推导出的标签。这导致同一目录在不同大小写/尾斜杠下被视为不同"项目"、无法给项目指定名称/颜色/图标、无法 pin/archive、项目级配置散落在文件系统各处、目录删除后 session 的 work_dir 变为悬空引用。layout-overhaul 的原型图（Sidebar Projects 区域、Header 项目名、InputBar 分支选择器、WorkspacePanel 文件变更）都需要一个正式的 Project 模型作为后端支撑。

## What Changes

- **新增 `projects` 表**：SQLite 中创建 `projects` 表（id, name, root_path, color, pinned, archived, created_at, last_opened_at），作为全局项目注册表
- **Session 关联 Project**：`sessions` 表新增 `project_id TEXT` 可选外键，指向 `projects.id`；保留 `work_dir` 字段作为执行时 cwd，向后兼容无 project_id 的老 session
- **项目级配置文件**：定义 `.xiaolin/project.json` 的结构（display name、description、default model、sandbox settings），与已有的 `.xiaolin/mcp.json`、`.xiaolin/rules/` 共同构成项目配置层
- **项目生命周期 API**：WebSocket 新增 `projects.*` 方法族（list / create / update / delete / detect），供前端 CRUD 和自动发现
- **前端 project-store**：Zustand store 管理项目列表、活跃项目、pin/archive 状态，替代当前 SessionList 中从 workDir 推导分组的逻辑

## Capabilities

### New Capabilities
- `project-registry`: 项目注册表——SQLite `projects` 表的 schema、CRUD 操作、路径规范化与去重、自动发现（从 work_dir 隐式创建）
- `project-session-binding`: 项目与会话的绑定关系——session.project_id 字段、新建 session 时自动关联 project、迁移现有 session 的 work_dir 到 project_id
- `project-config-file`: 项目配置文件——`.xiaolin/project.json` 的结构定义、读取/写入逻辑、与现有 MCP/rules/skills 配置的集成
- `project-lifecycle-api`: 项目生命周期 WebSocket API——projects.list / create / update / delete / detect 方法定义、请求/响应格式、事件广播
- `project-frontend-store`: 前端项目状态管理——Zustand project store、项目列表同步、活跃项目跟踪、pin/archive 操作

### Modified Capabilities
- `workspace-grouped-sessions`: 会话分组从按 workDir 字符串推导改为按 project_id 分组，Project 为 null 的 session 归入"未关联项目"组
- `session-workspace-sync`: 新建 session 时除了设置 work_dir，还需自动查找或创建对应的 Project 并设置 project_id
- `project-config-discovery`: 现有的项目配置发现逻辑需要从 project registry 获取 root_path，而非仅依赖 cwd 检测

## Impact

- **后端 crates**：`xiaolin-session`（新增 projects 表和迁移）、`xiaolin-core`（新增 project.rs 模块）、`xiaolin-gateway`（新增 ws/project.rs 处理器）、`xiaolin-protocol`（新增 Project 相关 op 定义）
- **前端**：新增 `project-store.ts`、修改 `chat-meta-store.ts`（ChatMeta 新增 projectId）、修改 SessionList 分组逻辑
- **数据库**：`projects` 表（新增）、`sessions` 表新增 `project_id` 列（迁移）
- **API**：WebSocket 新增 `projects.*` 方法族
- **兼容性**：向后兼容——`project_id` 为 nullable，老 session 不受影响；前端对无 project_id 的 session 降级为当前按 workDir 分组行为
