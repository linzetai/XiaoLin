## Why

FastClaw 的工具暴露和提示词组装架构存在**静态二分法**的局限——工具被硬编码为 eager（始终发给 LLM）或 deferred（需要 tool_search 发现），无法根据运行时上下文（执行模式、agent 类型、session 状态）动态调整。这在 plan-mode-v2 实现中暴露了严重 Bug：`exit_plan_mode` 工具注册为 deferred，导致 Plan 模式下 agent 无法调用它。同时，提示词中的模式指令占用固定 token 且无法做频率控制。竞品分析（Claude Code、Codex CLI）表明业界已采用更灵活的管线式架构。

## What Changes

- 引入 `ToolExposure` 三级枚举（Direct / Deferred / DirectModelOnly），替代当前 ToolRegistry 的 `deferred: HashSet<String>` 二分法。每个工具通过 `Tool::exposure()` 自声明暴露级别，支持运行时 override
- 实现 mode-aware 工具提升机制：根据 `ExecutionMode` 自动调整工具的 exposure（如 Plan 模式下 `exit_plan_mode` 从 Deferred 提升为 Direct）
- 将 Plan 模式指令从系统提示词的 `session_guidance_section()` 迁移到 per-turn attachment 机制，支持节流（每 N 轮完整提醒 / 中间简短提醒）
- 实现集中的模式转换状态机，跟踪 entry/exit/reentry 事件，防止重复/冲突的提示词注入
- 实现 `AgentToolsConfig.profile` 字段的实际功能，连接预定义的工具 profile（plan / readonly / full）

## Capabilities

### New Capabilities
- `tool-exposure`: 工具暴露级别系统——三级 ToolExposure 枚举、自描述接口、运行时 override、mode-aware promotion
- `mode-attachments`: 模式感知的 per-turn attachment 注入——节流策略、完整/简短模板切换、模式转换状态跟踪

### Modified Capabilities
- `agent-config-tools`: AgentToolsConfig.profile 字段接入预定义 ToolProfile，实际生效

## Impact

- `fastclaw-core/src/tool.rs`：ToolRegistry 重构，移除 deferred HashSet，增加 exposure 驱动的工具收集
- `fastclaw-agent/src/runtime/mod.rs`：execute_unified 中的工具收集逻辑重构
- `fastclaw-agent/src/runtime/prompt_sections/dynamic.rs`：Plan 模式指令迁移到 attachment
- `fastclaw-agent/src/builtin_tools/plan_mode.rs`：模式转换状态机
- `fastclaw-agent/src/subagent.rs`：子 agent 工具过滤使用 profile 驱动
- 需要同步更新前端（Plan 模式 attachment 在 UI 中不可见但需要正确处理）
