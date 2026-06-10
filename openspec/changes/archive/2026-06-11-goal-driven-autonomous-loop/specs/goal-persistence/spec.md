## ADDED Requirements

### Requirement: Goal stored in SQLite
Goal 数据 SHALL 持久化到 SQLite 数据库的 `goals` 表中，而非内存。

#### Scenario: Create goal persists to database
- **WHEN** agent 调用 create_goal
- **THEN** 新 goal 记录写入 SQLite goals 表

#### Scenario: Update goal persists to database
- **WHEN** agent 调用 update_goal 或系统更新 goal 状态
- **THEN** goals 表中对应记录被更新

### Requirement: Goal survives application restart
持久化的 goal SHALL 在应用重启后仍可恢复。

#### Scenario: Restart with active goal
- **WHEN** 应用关闭时存在 active goal，重启后用户打开同一 session
- **THEN** goal 数据从 SQLite 加载，状态保持为 paused（关闭视为中断）

### Requirement: Goal cascades with session deletion
当 session 被删除时，关联的 goal 记录 SHALL 一同删除。

#### Scenario: Delete session with goal
- **WHEN** 用户删除一个包含 goal 的 session
- **THEN** goals 表中对应的记录被级联删除

### Requirement: One active goal per session
每个 session 同一时间最多只有一个 active goal。

#### Scenario: Create goal when one already exists
- **WHEN** session 已有 active goal，agent 调用 create_goal
- **THEN** 旧 goal 自动标记为 cancelled，新 goal 创建为 active
