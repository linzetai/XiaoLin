## 1. SubAgentDef 字段扩展

- [x] 1.1 在 `SubAgentDef` 中新增 `max_result_chars: Option<usize>` 字段，`serde(default)` + `skip_serializing_if`
- [x] 1.2 在 `builtin_subagent_defs()` 中为各内置 def 保持 `max_result_chars: None`（使用默认值）
- [x] 1.3 在 `parse_markdown_subagent_def` 和 JSON 解析路径验证新字段能正确反序列化

## 2. Sub-agent 模型覆盖

- [x] 2.1 在 `subagent.rs` 的 `SubAgentTool::execute` 中，当 `def.model` 为 `Some` 时覆盖 `AgentConfig.model`
- [x] 2.2 添加单元测试验证 model override 生效和 None 时继承的行为

## 3. 结果截断参数化

- [x] 3.1 将 `sidechain::truncate_result` 函数签名改为 `truncate_result(text: &str, max_chars: usize) -> String`
- [x] 3.2 更新 `SubAgentManager::spawn` 中 forwarder 调用 `truncate_result` 的地方，传入 def 的 `max_result_chars` 或默认值
- [x] 3.3 在 `truncate_result` 内添加上限 clamp 逻辑（max 131072）
- [x] 3.4 更新现有 `truncate_result` 调用方传入 `MAX_RESULT_CHARS`

## 4. Session 清理修复

- [x] 4.1 在 `SubAgentManager::cleanup_session` 中增加 `self.session_event_senders.remove(session_id)`
- [x] 4.2 添加单元测试验证 cleanup 后 `session_event_senders` 不再包含该 session

## 5. 验证

- [x] 5.1 `cargo check --workspace` 通过
- [x] 5.2 `cargo clippy --workspace -- -D warnings` 零警告
- [x] 5.3 `cargo test --workspace --exclude xiaolin-app` 全部通过
