## ADDED Requirements

### Requirement: 后端 Artifact 记录

Agent runtime 在文件工具执行成功后 SHALL 记录一条 `FileArtifact`。注入点在 `xiaolin-agent` 的 tool round 成功回调中（扩展已有 `SessionFileTracker`），而非 `xiaolin-tools-fs` 层，以避免双轨记录并获取 session context。

#### Scenario: write_file 成功
- **WHEN** `write_file` 工具成功写入文件
- **THEN** 记录 `FileArtifact { session_id, path, operation: "created" | "modified", timestamp, tool_call_id, bytes }`
- **AND** `operation` 根据文件在写入前是否存在判断：不存在 = `created`，已存在 = `modified`

#### Scenario: edit_file 成功
- **WHEN** `edit_file` 工具成功编辑文件
- **THEN** 记录 `FileArtifact { session_id, path, operation: "modified", timestamp, tool_call_id, bytes }`

#### Scenario: create_file 成功
- **WHEN** `create_file` 工具成功创建文件
- **THEN** 记录 `FileArtifact { session_id, path, operation: "created", timestamp, tool_call_id, bytes }`

#### Scenario: 同文件多次操作
- **WHEN** 同一文件在同一 session 中被多次操作
- **THEN** 每次操作都记录一条 artifact
- **AND** 前端展示时按 path 去重，显示最新一次操作的信息

### Requirement: WS event 推送

Gateway SHALL 在收到 artifact 记录后通过 WebSocket 向前端推送 `file_artifact` 事件。

#### Scenario: 实时推送
- **WHEN** 后端记录一条新的 FileArtifact
- **THEN** 通过 session 绑定的 WS 连接发送 `{ type: "file_artifact", data: { sessionId, path, operation, timestamp, toolCallId, bytes } }`
- **AND** 前端 fileStore 接收并更新内存缓存

#### Scenario: 连接恢复
- **WHEN** WebSocket 重新连接
- **THEN** 前端通过 `artifacts.list` WS op 获取当前 session 的完整 artifact 列表
- **AND** 与内存缓存合并（以后端数据为准）

### Requirement: SQLite 持久化

FileArtifact 记录 SHALL 持久化到 SQLite 数据库。

#### Scenario: 写入
- **WHEN** 新的 FileArtifact 记录产生
- **THEN** 异步写入 `file_artifacts` 表
- **AND** 写入失败时 `warn!` 记录错误，不影响工具执行

#### Scenario: 查询
- **WHEN** 前端请求某 session 的 artifact 列表
- **THEN** 返回该 session 下所有 artifact 记录，按 timestamp DESC 排序
- **AND** 同一 path 的多条记录都返回（前端负责去重展示）

#### Scenario: session 删除清理
- **WHEN** 用户删除一个 session
- **THEN** 该 session 下的所有 artifact 记录一并删除

### Requirement: WS op artifacts.list

Gateway SHALL 提供 `artifacts.list` WebSocket 操作，返回指定 session 的 artifact 列表。（注：artifact 数据在 gateway SQLite 中，不走 Tauri IPC。）

#### Scenario: 正常查询
- **WHEN** 前端发送 `artifacts.list` WS op，携带 `{ sessionId }`
- **THEN** 返回该 session 的所有 FileArtifact 记录
- **AND** 格式为 `Array<{ path: string; operation: string; timestamp: string; toolCallId: string; bytes: number }>`

#### Scenario: session 不存在
- **WHEN** 查询一个不存在的 sessionId
- **THEN** 返回空数组（不报错）
