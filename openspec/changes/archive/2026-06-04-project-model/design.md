## Context

当前 `sessions` 表有一个 `work_dir TEXT` 可选列，前端 `ChatMeta.workDir` 存储原始路径字符串。`SessionList.tsx` 通过 `extractProjectName(workDir)` 提取路径末段作为分组标签——这是唯一的"项目"概念。后端 `detect_workspace_root()` 通过向上查找标记文件定位项目根目录，`detect_project_hints()` 嗅探项目类型注入 system prompt，`load_project_mcp_config()` 加载 `.xiaolin/mcp.json`。

这些逻辑互不关联、缺乏统一的 Project 实体。layout-overhaul 原型图的 Sidebar Projects 区域、Header 项目名、InputBar 分支选择器都需要一个正式的、可持久化的 Project 模型。

## Goals / Non-Goals

**Goals:**
- 引入持久化的 Project 实体，替代当前从 work_dir 推导"项目"的隐式行为
- Session 与 Project 建立正式关联（project_id 外键）
- 提供 WebSocket API 供前端 CRUD 项目
- 定义 `.xiaolin/project.json` 配置文件结构
- 前端 Zustand store 管理项目状态
- 向后兼容：无 project_id 的老 session 继续工作

**Non-Goals:**
- Git 集成（branch、status、diff、commit）——属于独立的 `git-integration` change
- 文件系统监控（watch file changes）——属于独立 change
- 多用户/团队项目共享——当前为单用户桌面应用
- 项目模板/脚手架功能
- WorkspacePanel 的实际数据接入——layout-overhaul 已处理 mock 数据

## Decisions

### D1: 混合持久化策略（SQLite + 文件系统）

**选择**：SQLite `projects` 表存全局注册表 + `.xiaolin/project.json` 存项目级配置

**替代方案**：
- 纯 SQLite：所有项目配置都存数据库 → 无法跟 git 仓库走，团队无法共享
- 纯文件系统：每个项目的 `.xiaolin/project.json` 存所有信息 → 无法维护全局状态（pinned、last_opened、color 是 UI 偏好，不应随仓库走）

**理由**：SQLite 存 UI 状态和全局索引（快速列出所有项目、排序、pin），`.xiaolin/project.json` 存项目自身的语义配置（display name、description、default model）。分离关注点：UI 偏好 vs 项目配置。

### D2: Project ID 使用 root_path 的 SHA-256 前缀

**选择**：`project_id = hex(sha256(canonicalized_root_path))[..16]`

**替代方案**：
- UUID v4：不可预测，同一路径多次创建会产生不同 ID
- 路径字符串直接当 ID：路径中可能有特殊字符，不适合做主键

**理由**：路径的确定性哈希保证同一目录始终映射到同一 ID，避免重复注册。canonicalize 解决大小写和符号链接问题。16 字符的十六进制前缀（64 位）冲突概率极低。

### D3: Session 新增 project_id 列，保留 work_dir

**选择**：`sessions` 表新增 `project_id TEXT` 可选列，同时保留 `work_dir`

**替代方案**：
- 删除 work_dir，完全由 project.root_path 替代 → 破坏兼容性，且 session 的实际执行路径可能不是项目根目录（如 worktree）
- 只添加 project_id 不保留 work_dir → 丢失精确的执行路径信息

**理由**：`project_id` 是组织层面的关联（哪个项目），`work_dir` 是执行层面的路径（在哪个目录运行工具）。两者语义不同，应该共存。向后兼容：`project_id = NULL` 的 session 降级为当前行为。

### D4: 隐式创建 + 显式管理

**选择**：当 session 设置 work_dir 时自动检测并创建 Project；用户可以在 UI 中显式管理（改名、换色、pin、archive、删除）

**替代方案**：
- 仅显式创建：用户必须手动"添加项目" → 增加认知负担
- 仅隐式创建：无法管理不再使用的项目 → 注册表无限膨胀

**理由**：隐式创建确保零配置即可工作（和当前体验一致），显式管理满足高级用户需求。

### D5: 前端 project-store 独立于 chat-meta-store

**选择**：新建 `project-store.ts` Zustand store，`chat-meta-store` 通过 `projectId` 引用

**替代方案**：
- 在 chat-meta-store 中内嵌项目逻辑 → 职责过重，项目可能跨 agent 共享

**理由**：Project 和 Session 是不同的领域概念。project-store 管理项目列表和状态，chat-meta-store 管理会话列表。通过 projectId 松耦合关联。

### D6: WebSocket API 前缀 `projects.*`

**选择**：新增 `projects.list`、`projects.create`、`projects.update`、`projects.delete`、`projects.detect` 方法

**理由**：与现有 `sessions.*` 命名风格一致。`projects.detect` 接收一个路径，返回检测到的项目根和建议的 Project 配置（基于 `detect_workspace_root` + `detect_project_hints`）。

### D7: 迁移现有 session 的 work_dir 到 project

**选择**：gateway 启动时执行一次性迁移——扫描所有有 work_dir 但没有 project_id 的 session，按 work_dir 查找或创建 Project，回填 project_id

**替代方案**：
- 懒迁移：session 被访问时才迁移 → 全局项目列表在迁移完成前不完整
- 不迁移：老 session 永远没有 project_id → 分组逻辑需要永久维护两条路径

**理由**：一次性迁移简单可靠，且数据量很小（通常几十到几百个 session）。

## Risks / Trade-offs

- **[路径变更]** 用户移动或重命名项目目录后 root_path 失效 → 缓解：UI 显示"项目不可达"状态，提供"重新定位"操作。后续可增加文件系统监控自动检测。
- **[ID 冲突]** SHA-256 前缀 16 字符理论上有冲突可能 → 缓解：冲突概率为 2^-64，实际场景（< 10^6 项目）可忽略。若发生冲突，创建时检测并追加随机后缀。
- **[迁移失败]** 启动迁移时某些 work_dir 路径已不存在 → 缓解：仍创建 Project 记录但标记为 unreachable，不影响 session 正常使用。
- **[配置冲突]** `.xiaolin/project.json` 中的 name 与 SQLite 中的 name 不同步 → 缓解：文件系统配置为权威来源（source of truth），SQLite 中的 name 作为缓存/覆盖。加载项目时优先读 `.xiaolin/project.json`，若无则用 SQLite 中的值。
- **[性能]** 项目列表查询增加一次 JOIN → 缓解：项目数量极小，且 SQLite 本地查询延迟可忽略。

## Migration Plan

1. **数据库迁移**：SessionStore 初始化时（已有的 migration 模式）检测 `projects` 表是否存在，不存在则创建；检测 `sessions.project_id` 列是否存在，不存在则 ALTER TABLE 添加
2. **数据迁移**：gateway 启动时扫描 `sessions WHERE work_dir IS NOT NULL AND project_id IS NULL`，按 canonicalized work_dir 查找或创建 Project，回填 project_id
3. **前端渐进**：project-store 加载后与 chat-meta-store 同步——有 projectId 的 session 按 project 分组，无 projectId 的降级为当前 workDir 分组
4. **回滚**：project_id 列为 nullable，删除 projects 表和 project_id 列不影响 sessions 表的其他功能

## Open Questions

- 是否需要 `.xiaolin/project.json` 的 `defaultModel` 覆盖用户级模型设置？还是仅在 UI 中作为快捷选择？
- 项目 color 是否支持自定义 hex 值，还是限定为预设调色板（如 8 种颜色）？
- 归档（archived）的项目是否仍然在 Sidebar 显示（折叠区域），还是完全隐藏？
