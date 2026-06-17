## ADDED Requirements

### Requirement: Workspace initialization creates identity templates without BOOTSTRAP.md

系统 SHALL 在初始化 workspace 时创建以下 identity 模板文件：SOUL.md、IDENTITY.md、USER.md、AGENTS.md、TOOLS.md。系统 SHALL NOT 创建 BOOTSTRAP.md 文件。

#### Scenario: Fresh workspace initialization
- **WHEN** `ensure_workspace()` 在一个空目录上执行
- **THEN** 创建 SOUL.md、IDENTITY.md、USER.md、AGENTS.md、TOOLS.md 模板文件
- **THEN** 不创建 BOOTSTRAP.md 文件

#### Scenario: Existing workspace with identity files
- **WHEN** `ensure_workspace()` 在已有 identity 文件的目录上执行
- **THEN** 不覆盖现有文件
- **THEN** 不创建 BOOTSTRAP.md 文件

### Requirement: Context engine does not inject bootstrap messages

系统 SHALL NOT 在会话消息列表中注入 "Bootstrap Pending" 消息，无论 workspace 中是否存在 BOOTSTRAP.md 文件。

#### Scenario: Workspace has leftover BOOTSTRAP.md
- **WHEN** workspace 目录中存在 BOOTSTRAP.md 文件
- **THEN** context engine 忽略该文件，不注入任何 bootstrap 相关消息

#### Scenario: New session in initialized workspace
- **WHEN** 用户在已初始化的 workspace 中创建新会话
- **THEN** 会话消息中仅包含 identity 文件内容（SOUL.md、IDENTITY.md 等）
- **THEN** 不包含任何 "Bootstrap Pending" 或 BOOTSTRAP.md 的内容

## REMOVED Requirements

### Requirement: Bootstrap ritual on first conversation
**Reason**: Bootstrap 仪式机制依赖 LLM 自行删除文件的不可靠行为，且 identity 模板文件的占位符文本已足够引导 agent 进行身份初始化
**Migration**: Identity 模板文件的占位符内容（如 "_(pick something you like)_"）自然引导 agent 在首次对话中询问用户偏好，无需额外仪式文件
