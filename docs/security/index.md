---
title: 安全与加固
summary: 纵深防御体系：认证、限流、SSRF/路径穿越/注入防护、Webhook 签名、消息完整性、沙箱隔离与预算控制。
---

# 安全概览

FastClaw 面向 **半可信渠道与终端用户**，默认采用纵深防御：认证、限流、沙箱、提示词层检测、SSRF 防护、路径穿越防护与消息完整性校验等组合使用。

## API Key 认证

- `security.apiKeys` 配置合法密钥列表；HTTP/WebSocket 请求携带 `Authorization: Bearer <key>` 头或约定查询参数。
- **恒定时间比较**（constant-time）防止计时侧信道泄漏。
- `/metrics` 端点同样受认证保护——不再豁免，防止内部指标泄漏。
- `/auth/status` 端点始终返回 `authRequired: true`，不暴露实际配置状态。
- 生产环境应 **轮换密钥** 并限制下发范围；勿在仓库中硬编码。

## 速率限制

- `gateway.rateLimit`：`enabled`、`maxRequests`、`windowSecs` 组合成固定窗口限流，抑制爆破与误触发的级联成本。
- 结合反向代理（Nginx/Envoy）可做全局配额与地理封禁。

## SSRF 防护

内置 `HttpFetchTool` / `WebFetchTool` 在发起外部请求前：

1. 仅允许 `http` 和 `https` 协议（阻止 `file://`、`ftp://` 等）。
2. 执行 **DNS 解析后检查**——解析到的 IP 地址若属于 RFC 1918/4193 保留地址段（如 `10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`、`127.0.0.0/8`、`::1`、`fd00::/8` 等），请求被拒绝。
3. 阻止 DNS 重绑定攻击：在连接前检查而非依赖 DNS 缓存。

## 路径穿越防护

涉及文件系统操作的功能均具备路径验证：

- **PatchEngine / write_skill**：使用 `validate_skill_id` 严格校验（仅允许字母数字、连字符、下划线），对构造好的路径做 `canonicalize` 确认在预期目录内。
- **Hub 插件安装**：从 ZIP 解压时验证每个文件名不包含 `..` 且不以 `/` 开头，解压后对 canonical 路径做边界检查。
- **Code Sandbox**：`test_runner` 写入文件前强制 canonical path 检查。

## Webhook 签名验证

各渠道扩展通过 `ChannelPlugin::verify_webhook` 在 `handle_webhook` **之前**进行平台特定签名验证：

| 渠道 | 验证方式 |
|------|----------|
| Slack | `X-Slack-Signature` + `X-Slack-Request-Timestamp` HMAC-SHA256 |
| WhatsApp | `X-Hub-Signature-256` HMAC-SHA256 |
| Feishu | Verification Token + Event Token 校验 |

验证失败时立即拒绝请求，不进入业务处理逻辑。

## WASM 沙箱隔离

- 插件受 **内存上限、执行时间（epoch）、Fuel** 约束，避免恶意或缺陷插件拖垮宿主。
- `env::abort` 调用在宿主侧触发 **trap**（非 no-op），确保异常终止可被正确捕获。
- Epoch 线程具备 **优雅退出** 机制（`Drop` 实现），避免资源泄漏。
- 仅暴露 **白名单 host 能力** 给插件；敏感系统调用不可达。
- 插件加载需 **HMAC-SHA256 签名验证**（当配置了 `trusted_keys` 时强制要求 `manifest.signature` 存在）。

## 提示注入防御

- `security.promptInjectionDetection` 打开时，对入站用户文本做启发式/模型辅助检测，降低 **间接提示注入** 风险。
- 系统消息标识使用 `starts_with` 精确匹配（而非 `contains`），防止用户通过消息内容注入系统标记。
- 提示蒸馏中对用户内容使用 `replacen` 限定替换（防止模板注入），且自动修剪 prompt 避免无限增长。
- 仍需在 **系统提示** 中明确工具边界与数据来源标签，避免模型被社交工程欺骗。

## 消息签名与重放防护（HMAC）

内部 **消息总线** 支持 `MessageBus::new_with_hmac`：

- 对 `AgentMessage` 关键字段做 **HMAC-SHA256**，十六进制编码存放于 `signature` 字段。
- 启用后 **未签名或签名错误** 的消息会被拒绝。
- **时间戳验证**：消息 `timestamp` 必须在当前时间 ±5 分钟窗口内（允许 30 秒时钟偏移），超出范围视为重放并拒绝。
- 每个 topic 的订阅者数量上限为 256，自动清理已关闭的 sender，防止资源泄漏。

## 代码执行沙箱

DAG 引擎与 CodeSandbox 中的代码执行：

- **禁止 Shell 执行**：DAG code 节点仅允许 `python`、`javascript`、`rust` 三种语言，`shell`/`bash` 及未知语言均被拒绝。
- **代码长度限制**：单次执行代码不超过 100KB。
- **编译/执行超时**：Rust 编译超时 60 秒，所有执行均有运行超时。
- **输出截断**：输出超过长度限制时在 UTF-8 安全边界处截断。

## 预算与资源控制

- **模型路由预算**：`BudgetTracker` 使用原子操作实现 `try_reserve` / `release_reservation`，防止 TOCTOU 竞态导致预算超支。超出每日预算时严格拒绝请求。
- **自迭代硬上限**：`HARD_MAX_ROUNDS = 20`、`MAX_PROMPT_CHARS = 100,000`，防止无限循环与内存爆炸。
- **每 Provider 并发限制**：`models.<provider>.maxConcurrent` 限制对同一云厂商的并发请求，防止 **配额耗尽、账单尖峰** 与 **429 风暴**。

## 工具执行策略

Agent 运行时执行工具调用前进行策略校验：

- 支持 `tool_allow_list`（白名单）和 `tool_deny_list`（黑名单）。
- 连续错误达到阈值时自动停止执行，防止无限重试循环。

## 数据隔离

- **DmScope 会话隔离**：通过 `per-peer`、`per-channel-peer`、`per-account-channel-peer` 确保不同用户/账户/渠道的会话数据严格分离。
- **WebSocket 广播隔离**：禁止通配符订阅（`*`），防止跨会话数据泄漏。
- **Memory API 访问控制**：记忆端点需认证，删除操作生成审计日志。

## 数据库安全

- SQLite 连接池默认启用 `PRAGMA foreign_keys = ON`，确保引用完整性。
- SQL 查询中用户输入使用参数化绑定；`LIKE` 操作对通配符（`%`、`_`）做转义处理。
- 原子操作：cron 任务使用条件 `UPDATE`（CAS）防止重复执行。

## 相关文档

- [网关配置](../gateway/configuration.md)
- [配置字段参考](../gateway/configuration-reference.md)
- [REST API](../reference/api.md)
