## ADDED Requirements

### Requirement: Child agent messages persisted to sidechain file

子 agent 的每条消息 SHALL 持久化到独立的 sidechain JSONL 文件。

#### Scenario: Background subagent writes sidechain
- **WHEN** 子 agent 执行过程中产出 assistant/user/tool messages
- **THEN** 每条消息追加写入 `{session_dir}/sidechains/{run_id}.jsonl`
- **AND** 消息格式为 JSON line: `{ "role": "...", "content": "...", "timestamp": ..., "agent_id": "..." }`

#### Scenario: Sync subagent also writes sidechain
- **WHEN** 同步（非 background）子 agent 执行
- **THEN** 同样写入 sidechain 文件（不因同步模式跳过持久化）

### Requirement: Sidechain includes metadata header

Sidechain 文件第一行 SHALL 为 metadata header。

#### Scenario: File creation
- **WHEN** 新的子 agent run 开始
- **THEN** 创建 sidechain 文件，第一行写入 header: `{ "_meta": true, "run_id": "...", "agent_id": "...", "parent_session_id": "...", "task": "...", "started_at": ... }`

### Requirement: Result extraction from sidechain

子 agent 完成时 SHALL 从 sidechain 提取最终结果返回给父级。

#### Scenario: Normal completion
- **WHEN** 子 agent stream 正常结束
- **THEN** 取 sidechain 中最后一条 `role: "assistant"` 消息的 content 作为工具返回值

#### Scenario: Long result truncation
- **WHEN** 最终 assistant 消息超过 4096 字符
- **THEN** 截断到 4096 字符，末尾追加 `\n[truncated — use subagent_get for full result]`

#### Scenario: Empty result
- **WHEN** 子 agent 未产出任何 assistant 消息就结束（如被取消）
- **THEN** 返回 `"[subagent terminated without producing a result]"`

### Requirement: Resume subagent from sidechain

系统 SHALL 支持从 sidechain 恢复子 agent 执行。

#### Scenario: Resume previously interrupted run
- **GIVEN** 存在 `{session_dir}/sidechains/{run_id}.jsonl` 文件
- **WHEN** 调用 `resume_subagent(run_id)` 工具
- **THEN** 读取 sidechain 消息作为 initial context，继续执行，新消息追加到同一文件

#### Scenario: Resume non-existent run
- **WHEN** 指定的 run_id 不存在对应 sidechain 文件
- **THEN** 返回 ToolResult::err("sidechain not found for run_id: ...")

### Requirement: Sidechain files cleaned up with session

Session 删除时 SHALL 级联删除关联的 sidechain 文件。

#### Scenario: Session cleanup
- **WHEN** session 被删除
- **THEN** `{session_dir}/sidechains/` 目录及其内容被删除
