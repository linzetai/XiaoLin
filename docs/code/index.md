---
title: 代码智能
summary: LSP 集成（符号搜索、定义跳转、引用查找）为 Agent 提供仓库级语义定位能力。
---

# 代码智能（Code Intelligence）

`fastclaw-agent` 内置 LSP 集成为 FastClaw 提供 **仓库级语义定位** 能力，使 Agent 在软件开发场景下可定位符号、跳转定义、查找引用。

## LSP 集成

`fastclaw-agent` 内置 **`LspSessionManager`**，管理每个工作区的 LSP 会话（当前以 `rust-analyzer` 为主），提供三个内置工具：

| 工具 | LSP 方法 | 降级行为 |
|------|----------|----------|
| `workspace_symbols` | `workspace/symbol` | 正则启发式搜索 |
| `go_to_definition` | `textDocument/definition` | 文本搜索 |
| `find_references` | `textDocument/references` | 文本搜索 |

发布安装包（CLI 与 Tauri 桌面应用）可内置 `rust-analyzer`，运行时优先使用内置二进制，找不到时回退系统 PATH。

## 相关文档

- [系统架构](../concepts/architecture.md)
- [工具与插件](../tools/index.md)
