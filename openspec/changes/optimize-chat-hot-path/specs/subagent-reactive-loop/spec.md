## MODIFIED Requirements

### R1: Supervised Wait
- 当主 agent turn 中存在活跃 sub-agent runs 时，harness 自动进入等待状态
- Turn 不得在有活跃 sub-agent 时结束

#### Scenario: GC 正确清理 IM 渠道的 session 资源
- **WHEN** 一个通过 IM 渠道（feishu/wechat）创建的 session 的所有 sub-agent 均已终止，且 session actor 已被销毁
- **THEN** `chat_locks` 和 `chat_cancels` 中该 session 对应的条目 SHALL 在下一次 GC 周期被正确移除，使用 `session_key` 而非 `chat_id` 进行匹配

#### Scenario: HTTP 渠道 session 的 GC 兼容性
- **WHEN** 一个通过 HTTP 渠道创建的 session 走正常 GC 路径
- **THEN** GC 行为不受 key 统一改造影响，session 资源正常回收
