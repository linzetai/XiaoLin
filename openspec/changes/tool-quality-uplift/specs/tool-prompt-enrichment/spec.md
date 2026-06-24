## ADDED Requirements

### Requirement: 薄描述工具必须补齐行为指导

仅有简短 `description()`、缺乏行为指导的工具 SHALL 提供丰富的 `prompt()`，内容覆盖：何时使用、与其他工具的配合关系、反模式、关键参数交互。**范围冻结**：优先覆盖统一 `lsp` 工具、`file_outline`、`code_sections`、subagent 全家、`skill`/`identity`；**排除已有 rich prompt 的工具**（shell_exec、web_search、read_file 等），不追求覆盖全部工具。

#### Scenario: 大文件理解前优先使用结构概览
- **WHEN** `file_outline` / `code_sections` 的 `prompt()` 被发送给模型
- **THEN** 该 prompt MUST 明确建议"读大文件前先用本工具获取结构，再定向 read_file 目标区段"，并给出与 `read_file` 的配合说明

#### Scenario: 工具配合与反模式被显式说明
- **WHEN** 为 code-intel 或 subagent 工具补齐 `prompt()`
- **THEN** prompt MUST 至少包含一条 when-to-use、一条与相关工具的配合建议、一条反模式（避免误用的场景）

### Requirement: 富 prompt 不破坏 UI 短描述

补齐 `prompt()` 时，工具的 `description()`（UI 展示用）SHALL 保持简短；二者职责分离，丰富指导仅进入发送给 LLM 的 `prompt()`，不污染 UI 短描述。

#### Scenario: description 与 prompt 分离
- **WHEN** 某工具同时定义了简短 `description()` 与丰富 `prompt()`
- **THEN** UI 渲染 MUST 使用 `description()`，发送给 LLM 的工具定义 MUST 使用 `prompt()`，二者互不混用
