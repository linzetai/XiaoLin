## ADDED Requirements

### Requirement: 文件树浏览

Files tab 的文件列表区域 SHALL 支持浏览当前 session 工作目录下的文件树结构。

#### Scenario: 工作目录已设置
- **WHEN** 当前 session 的 `workDir` 已设置
- **THEN** 文件列表顶部显示工作目录名（最后一级目录名）
- **AND** 默认展示第一层目录内容（文件和子目录）
- **AND** 子目录显示为可展开的树节点

#### Scenario: 懒加载子目录
- **WHEN** 用户点击展开一个子目录
- **THEN** 通过 `list_directory` IPC 命令加载该目录内容
- **AND** 加载期间显示 spinner
- **AND** 加载完成后展示子目录内容

#### Scenario: 文件类型图标
- **WHEN** 渲染文件树节点
- **THEN** 文件夹使用 Folder Phosphor 图标
- **AND** 文件根据扩展名使用不同颜色或图标（如 `.rs` 用 Rust 色调）

#### Scenario: 文件点击
- **WHEN** 用户在文件树中点击一个文件
- **THEN** 在查看器中打开该文件
- **AND** 如果文件已在 tab 中打开，切换到该 tab 而非新开

#### Scenario: workDir 未设置
- **WHEN** 当前 session 的 `workDir` 为 null
- **THEN** 文件树区域显示提示："设置工作目录以浏览文件"

### Requirement: Session Artifacts 优先展示

文件列表 SHALL 在文件树之上显示当前 session 的 artifact 列表（agent 操作过的文件）。

#### Scenario: 有 artifact 记录
- **WHEN** 当前 session 存在 artifact 记录
- **THEN** 文件列表顶部显示 "Session Files" 分组
- **AND** 列出所有被 agent 操作过的文件（按 path 去重，显示最新操作）
- **AND** 每个文件旁显示操作类型标识：`C`（created，绿色）、`M`（modified，蓝色）

#### Scenario: artifact 实时更新
- **WHEN** Agent 在当前 session 中操作了新文件
- **THEN** Session Files 列表实时更新，新文件出现在列表顶部

### Requirement: IPC 命令 list_directory

Tauri IPC SHALL 提供 `list_directory` 命令，返回目录内容。

#### Scenario: 正常列举
- **WHEN** 前端调用 `list_directory(path)`
- **THEN** 返回该目录下的直接子项
- **AND** 格式为 `Array<{ name: string; isDirectory: boolean; size: number; modifiedAt: string }>`
- **AND** 结果按：目录在前 + 文件在后，各自按名称字母排序

#### Scenario: 路径安全校验
- **WHEN** 请求的路径 canonicalize 后不在 `workDir` 范围内
- **THEN** 返回错误 "path outside allowed directory"
- **AND** 不返回任何目录内容

#### Scenario: 隐藏文件过滤
- **WHEN** 目录中包含以 `.` 开头的隐藏文件/目录
- **THEN** 默认不显示隐藏文件
- **AND** 忽略 `node_modules`、`target`、`.git`、`__pycache__` 等常见大型目录

### Requirement: IPC 命令 file_metadata

Tauri IPC SHALL 提供 `file_metadata` 命令，返回文件元信息。

#### Scenario: 正常查询
- **WHEN** 前端调用 `file_metadata(path)`
- **THEN** 返回 `{ size: number; modifiedAt: string; isDirectory: boolean; mimeType?: string }`

#### Scenario: 路径安全校验
- **WHEN** 请求的路径 canonicalize 后不在 `workDir` 范围内
- **THEN** 返回错误 "path outside allowed directory"
