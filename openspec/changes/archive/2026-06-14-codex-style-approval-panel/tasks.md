## 1. Protocol 层扩展

- [x] 1.1 在 `crates/xiaolin-protocol/src/approval.rs` 新增 `ActionRiskLevel` 枚举 (Low/Medium/High)，含 Serialize/Deserialize/TS derive
- [x] 1.2 扩展 `PendingAction::FileWrite` 增加 `content: Option<String>` 字段（serde skip_serializing_if）
- [x] 1.3 扩展 `PendingAction::ApplyPatch` 增加 `diff: Option<String>` 字段（serde skip_serializing_if）
- [x] 1.4 在 `crates/xiaolin-protocol/src/event.rs` 的 `ApprovalRequired` 中增加 `risk_level: Option<ActionRiskLevel>` 字段（不再需要 suggested_prefix，prefix 直接在 available_decisions 的变体中携带）
- [x] 1.5 在 `ApprovalDecision` 枚举新增 `ApprovedWithPolicyAmend { prefix: Vec<String> }` 变体
- [x] 1.6 添加序列化/反序列化单元测试（ActionRiskLevel roundtrip、PendingAction 含 content 的 roundtrip、ApprovalRequired 含 risk_level 的 roundtrip、ApprovedWithPolicyAmend roundtrip）

## 2. 后端风险推断逻辑

- [x] 2.1 在 `crates/xiaolin-agent/src/runtime/orchestrator.rs` 新增 `fn infer_risk_level(action: &PendingAction, workspace: &Path) -> ActionRiskLevel` 函数
- [x] 2.2 实现 ShellCommand 风险规则：匹配 rm -rf/sudo/chmod/curl|sh 等模式 → High，其余 → Medium
- [x] 2.3 实现 FileWrite/ApplyPatch 风险规则：path 在 workspace 外 → High，workspace 内 → Medium
- [x] 2.4 实现 NetworkAccess 风险规则：→ Medium
- [x] 2.5 在两处 emit `ApprovalRequired` 的代码中调用 `infer_risk_level` 并填入 `risk_level: Some(level)`
- [x] 2.6 根据 risk_level 和 action 类型决定 available_decisions 是否包含 `ApprovedWithPolicyAmend { prefix }`（仅 Medium ShellCommand，prefix 为第一个 token）
- [x] 2.7 添加 `infer_risk_level` 单元测试（覆盖 High/Medium 各场景）

## 3. 后端内容填充 + ExecPolicy 持久化

- [x] 3.1 在 `crates/xiaolin-agent/src/runtime/runtimes/file.rs` 的 `FileWriteRuntime::to_pending_action` 中提取 content 并截断到 2000 chars（使用 floor_char_boundary）
- [x] 3.2 在 `FileEditRuntime::to_pending_action` 中从 args 提取 old_string/new_string 构造简化 diff 字符串并截断
- [x] 3.3 新增 `fn truncate_preview(s: &str, max_chars: usize) -> String` 工具函数
- [x] 3.4 更新 `map_tool_to_pending_action` 函数以适配新字段（content/diff 填充）
- [x] 3.5 在 orchestrator 的 decision match 分支中处理 `ApprovedWithPolicyAmend`：调用 `policy.add_session_rule()` + `spawn_blocking(blocking_append_allow_prefix_rule)`
- [x] 3.6 确定 ExecPolicy 文件路径（workspace `.xiaolin/exec_policy.rules` 或类似位置）
- [x] 3.7 确保 `cargo check` 和 `cargo clippy -- -D warnings` 通过

## 4. 前端审批面板重写

- [x] 4.1 更新 `ApprovalData` 接口：`riskLevel` 类型改为 `"low" | "medium" | "high"`，新增 `actionLabel` 字段
- [x] 4.2 重写 `ApprovalCard.tsx` 布局：左竖线 + 意图标题 + 始终可见命令/路径 + cwd 次级展示 + 垂直列表决策按钮
- [x] 4.3 实现风险等级到左竖线颜色的映射（high→red, medium→amber, low→neutral）
- [x] 4.4 实现 content/diff 可折叠预览区域（<=5行自动展开，>5行折叠）
- [x] 4.5 添加键盘快捷键支持：useEffect 注册 window keydown (y/s/p/n/a)，检查 event.target 避免输入框冲突
- [x] 4.6 实现提交后禁用状态（高亮已选决策，灰化其他选项）
- [x] 4.7 实现"记住前缀"选项：从 available_decisions 中的 ApprovedWithPolicyAmend 变体提取 prefix，显示 `[P] 记住「{prefix}」前缀` 按钮，点击/按 p 发送原始变体回后端

## 5. 前端适配层

- [x] 5.1 更新 `useMessageStreamChat.ts` 中 approval_required 事件处理：读取新 `risk_level` 字段，从 available_decisions 中识别 ApprovedWithPolicyAmend 变体及其 prefix
- [x] 5.2 删除旧的 `"danger"/"caution"/"safe"` 映射逻辑，使用后端原始值（fallback: `"medium"`）
- [x] 5.3 删除 `ApprovalCard` 中不再使用的 `ShieldWarning`/`ShieldCheck`/`ShieldSlash` 导入和 riskStyles 对象
- [x] 5.4 处理 onDecision 回调：当决策为 `approved_with_policy_amend` 时，附带 prefix 字段发送到后端

## 6. 验证

- [x] 6.1 `cargo check` 全项目通过
- [x] 6.2 `cargo clippy -- -D warnings` 零警告
- [x] 6.3 `cargo test -p xiaolin-protocol` 通过（新增的序列化测试）
- [x] 6.4 `cargo test -p xiaolin-agent` 通过（risk_level 推断 + policy amend 处理测试）
- [x] 6.5 前端 `tsc --noEmit` 通过
- [x] 6.6 MCP 触发审批面板端到端验证：发送触发 shell 命令的消息，确认面板渲染正确且键盘快捷键可用
- [x] 6.7 验证"记住前缀"流程：approve with policy amend → 确认 exec_policy 文件写入 → 后续同前缀命令自动放行
