---
title: 常见问题与排障
summary: FAQ、性能调优建议、调试技巧与已知限制。
---

# 常见问题（FAQ）

## 安装与启动

**Q：`cargo build` 很慢或失败？**  
A：确认已安装 `openssl`/TLS 依赖与 LLVM；使用 `cargo build -j N` 控制并行度；在 CI 中启用 `sccache`。

**Q：网关启动后端口被占用？**  
A：修改 `gateway.port` 或结束占用进程；Docker 映射时注意宿主与容器端口一致。

**Q：找不到配置文件？**  
A：检查 `config/default.json` 与 `~/.fastclaw/config/default.json`；使用 `fastclaw config path` 查看解析路径。

**Q：首次安装后需要手动创建配置和身份文件吗？**  
A：通常不需要。首启会自动创建 `~/.fastclaw/config/default.json`；创建或更新 Agent 时会自动生成该 Agent 的 `SOUL.md` / `USER.md` / `AGENTS.md`。

## 桌面应用

**Q：桌面应用启动后显示连接失败？**  
A：应用内嵌网关进程内启动，若端口（默认 18789）被占用会自动选择随机端口。检查系统托盘通知是否显示启动错误；可在终端查看 `~/.fastclaw/logs/` 下的日志。

**Q：桌面应用和 CLI 是否共享数据？**  
A：是的。桌面应用与 CLI 共享 `~/.fastclaw/` 下的配置、会话、Agent 和技能数据。两者可交替使用。

**Q：桌面应用如何管理技能（Skills）？**  
A：在设置面板中可查看、启用/禁用、上传技能（支持文件夹或 zip 压缩包）。技能数据存储在 `~/.fastclaw/skills/` 目录下。

## 模型与凭证

**Q：返回 401 或模型不可用？**  
A：核对 `credentials.*.apiKey` 与 `models.*.baseUrl`；确认 `agents.list[].model` 的 provider 前缀与 `models` 键一致。

**Q：流式输出中断？**  
A：查看反向代理 **超时**；SSE 需禁用中间缓冲；客户端需持续读取直至 `[DONE]` 等价事件。

**Q：代码能力里的 LSP（如 rust-analyzer）要用户自己安装吗？**  
A：发布安装包可以内置 `rust-analyzer`。运行时会优先使用内置二进制，找不到才回退系统 PATH，因此标准安装流程下用户无需手动安装。

## 工具与代码能力

**Q：Agent 的 `ask_question` 工具怎么用？**  
A：这是一个内置的人机回环工具。Agent 在任务中遇到需要用户确认的决策时，会自动弹出结构化问题（多选/单选），用户回答后 Agent 继续执行。支持超时自动跳过。在桌面应用中直接弹窗展示，WebSocket 客户端通过 `chat.ask_question` 事件接收。

**Q：`workspace_symbols` / `go_to_definition` / `find_references` 需要什么前置条件？**  
A：这些工具优先使用 LSP（如 `rust-analyzer`）。发布安装包已内置 `rust-analyzer`，标准安装无需额外操作。若 LSP 不可用，工具会自动降级为文本搜索。

## 渠道与 Webhook

**Q：飞书 Challenge 失败？**  
A：确认公网 URL、TLS 与路径 `/webhook/feishu`；阅读网关日志中的校验错误。

## 性能调优

- **嵌入本地模型**：首次下载后驻留磁盘；多实例部署可共享只读模型缓存卷。
- **SQLite**：会话与检查点 WAL 文件增长时定期备份与 `VACUUM` 维护计划。
- **限流**：为面向公网的网关开启 `gateway.rateLimit`，防止恶意刷爆上游 LLM。
- **模型并发**：使用 `maxConcurrent` 限制单 provider 并发，平滑账单与尾延迟。

## 调试技巧

```bash
fastclaw doctor
fastclaw config check
RUST_LOG=debug fastclaw serve
```

- 对 **单条 HTTP** 使用 `curl -v` 观察认证头与 TLS。
- 对 **DAG** 先用 `/api/v1/dag/validate` 再执行，缩小问题面。
- 对 **Agent 逻辑** 临时提高 `logging.level` 为 `debug`（注意日志脱敏）。

## 已知限制

- 主配置 **不通过环境变量** 展开占位符（历史 `${VAR}` 语法已移除）；敏感信息请用外部密钥注入 + 受控分发。
- **Agent 热重载** 针对 Agent 目录；修改主 `default.json` 通常需重启网关。
- 部分示例 JSON 键可能 **领先于** 某一发行版的 serde 结构体，请以当前提交源码为准。

## 相关文档

- [快速开始](../start/getting-started.md)
- [网关配置](../gateway/configuration.md)
- [安全](../security/index.md)
