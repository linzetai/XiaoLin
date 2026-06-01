## ADDED Requirements

### Requirement: event_log 批量写入
EventLog SHALL 通过内部 buffered writer 机制将多个事件合并为单次 SQLite 事务写入，而非每个事件独立 INSERT。

#### Scenario: 正常流式回复
- **WHEN** 一次 LLM 流式回复产生 100 个 ContentDelta 事件
- **THEN** event_log 将这些事件分批写入 SQLite（每批最多 64 条或每 50ms flush 一次），总 INSERT 语句数 SHALL 少于 10 次

#### Scenario: 低频事件
- **WHEN** 两个事件之间间隔超过 50ms
- **THEN** 前一个事件在 50ms 内被 flush 到 SQLite，不会无限等待凑批

### Requirement: flush on shutdown
EventLog writer task SHALL 在收到 shutdown 信号时将 buffer 中残余事件全部 flush 到 SQLite。

#### Scenario: graceful shutdown
- **WHEN** 应用收到 shutdown 信号且 buffer 中有 5 条未写入事件
- **THEN** 这 5 条事件在 shutdown 完成前被写入 SQLite

#### Scenario: writer channel 关闭
- **WHEN** 所有 event sender 被 drop（channel 关闭）
- **THEN** writer task drain 完 buffer 后自动退出

### Requirement: 非阻塞 send
调用方向 event_log 提交事件时 SHALL 使用非阻塞 send（`try_send`）。channel 满时 SHALL 丢弃事件并记录 warning 日志，不阻塞聊天热路径。

#### Scenario: channel 满时降级
- **WHEN** event buffer channel 已满（容量 1024）且有新事件提交
- **THEN** 新事件被丢弃，tracing::warn 记录丢弃计数，聊天流不受影响
