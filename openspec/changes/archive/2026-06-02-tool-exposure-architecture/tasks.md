## 1. ToolExposure 枚举 + Tool trait 扩展

- [x] 1.1 在 `xiaolin-core/src/tool.rs` 中定义 `ToolExposure` 枚举（Direct / Deferred）
- [x] 1.2 为 `Tool` trait 增加 `fn exposure(&self) -> ToolExposure` 默认方法（默认 Direct）
- [x] 1.3 `ExitPlanModeTool` 和 `EnterPlanModeTool` override `exposure()` 返回 Deferred
- [x] 1.4 `ToolSearchTool` 等现有 deferred 工具改用 `exposure()` 自声明

## 2. ToolProfile + mode-aware 工具提升

- [x] 2.1 定义 `ToolProfile` struct（promote: Vec<String>, demote: Vec<String>）
- [x] 2.2 实现 `ToolProfile::for_mode(ExecutionMode)` 预定义 profile
- [x] 2.3 在 `ToolRegistry` 中增加 `definitions_with_profile(&ToolProfile)` 方法
- [x] 2.4 重构 `execute_unified` 中的工具收集逻辑，使用 `definitions_with_profile`
- [x] 2.5 移除 `execute_unified` 中的临时 `activate_deferred("exit_plan_mode")` 调用

## 3. ToolRegistry 内部重构

- [x] 3.1 将 `deferred: HashSet<String>` 改为基于 `Tool::exposure()` 驱动的过滤
- [x] 3.2 保留 `register_deferred()` 作为向后兼容方法（内部设置 exposure override）
- [x] 3.3 确保 `search_deferred()` 仍然正确工作（兼容新的 exposure 机制）
- [x] 3.4 运行 `cargo clippy -- -D warnings` 确认无警告

## 4. AgentToolsConfig.profile 接入

- [x] 4.1 定义预置 profile 映射（"plan" / "readonly" / "full"）
- [x] 4.2 在 `SubAgentToolFilter` 中增加 `profile: Option<String>` 字段
- [x] 4.3 在子 agent 工具过滤逻辑中，profile 的 demote 列表与 `SubAgentToolFilter.denied` 合并
- [x] 4.4 修复所有 `SubAgentToolFilter` 字面构造器以包含 `profile` 字段

## 5. Mode Attachment 基础设施

- [x] 5.1 创建 `xiaolin-agent/src/runtime/mode_attachments.rs` 模块
- [x] 5.2 定义 `ModeAttachment` struct（full_template, sparse_template, turns_between, full_every_n）
- [x] 5.3 实现 Plan 模式的完整版和简短版 attachment 模板（中英双语）
- [x] 5.4 实现节流逻辑（turn 计数 + full/sparse 交替）+ reentry 通知

## 6. ExecutionModeState 增强

- [x] 6.1 在 `ExecutionModeState` 中增加 `plan_turn_counter: AtomicU32`
- [x] 6.2 增加 `has_exited_plan: AtomicBool` 用于 reentry 检测
- [x] 6.3 实现集中的 `transition()` 方法，自动更新 counter 和 reentry 状态
- [x] 6.4 增加 `increment_plan_turn()` 和 `plan_turn_count()` 访问器

## 7. 注入 Mode Attachment

- [x] 7.1 在 `execute_stream_inner` 的每轮迭代中注入 mode attachment
- [x] 7.2 将 attachment 作为 user-role 消息注入到 LLM 请求的消息列表中
- [x] 7.3 从 `session_guidance_section()` 移除 Plan 模式相关的指令块（中英双语）
- [x] 7.4 更新相关测试以反映新的行为（plan 指令不在系统提示中）

## 8. 集成测试 + 验证

- [x] 8.1 单元测试：`ToolProfile::plan_mode()` 返回正确的 promote/demote 列表
- [x] 8.2 单元测试：`definitions_with_profile` 正确提升/降级工具
- [x] 8.3 单元测试：Plan 模式下 exit_plan_mode 通过 profile 出现在工具列表中
- [x] 8.4 单元测试：attachment 节流逻辑（turn 0 完整、turn 1-4 跳过、turn 5 简短、turn 25 完整）
- [x] 8.5 `cargo clippy -- -D warnings` 全项目零警告
