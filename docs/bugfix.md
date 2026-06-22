# XiaoLin 代码审查 Bug 追踪

> 审查日期：2026-06-22
> 审查范围：全项目（~482 个源文件）
> 发现问题：🔴 23 / 🟡 50 / 🟢 22

## 状态说明

| 状态 | 含义 |
|------|------|
| ⬜ OPEN | 待修复 |
| 🔧 IN_PROGRESS | 修复中 |
| ✅ FIXED | 已修复（附 commit hash） |
| ⏭️ DEFERRED | 已推迟（附原因） |
| 🚫 WONTFIX | 不修复（附原因） |

---

## P0 — 必须修复

### 安全类

#### BUG-001 🔴 沙箱不可用时静默回退到宿主 shell 执行

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/runtimes/shell.rs` L127–151
- **问题**：`SandboxManager::is_available()` 为 false 时仅 `warn!`，随后 `build_plain_command` 在宿主执行。用户/UI 可能仍认为有沙箱保护。`SandboxPreference::Required` 和 `Auto` 行为相同。
- **影响**：权限提升，用户不知情地在无隔离环境执行命令
- **建议**：`Required` 时直接失败；`Auto` 时需显式 escalation 或用户确认后再 fallback
- **相关规则**：新增规则 #21
- **修复记录**：2026-06-22 沙箱 Required 时直接报错，Auto 时输出添加无隔离警告前缀

---

#### BUG-002 🔴 API Key 热更新不生效

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-gateway/src/lib.rs` L138–142、L208–212
- **关联文件**：`crates/xiaolin-gateway/src/ws/config.rs` L226–238
- **问题**：`ApiKeyAuth` 在 `build_app` 时从静态 `config.security.api_keys` 构造并注入 Extension。`config.set` 可写 `security` 并更新 `config_live`，但未刷新 `ApiKeyAuth`。旧 key 仍可用，新 key 无效。
- **影响**：密钥轮换窗口期安全漏洞
- **建议**：将 `ApiKeyAuth` 改为 `Arc<ArcSwap<AuthConfig>>` 或在 `config.set` 的 `security` 分支同步重建 auth 层
- **修复记录**：2026-06-22 ApiKeyAuth 改为 ArcSwap 动态读取，config.set 同步 reload

---

#### BUG-003 🔴 exec_command 无 Shell 安全校验且直接 `shell -c`

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-fs/src/exec_command.rs` L318–321、L112–155
- **问题**：用户 `cmd` 经 `Command::new(shell).arg("-c").arg(cmd)` 执行，未调用 `ShellSecurityChecker` / `PathValidator`，也未走 `SandboxManager`。该工具仍在 `xiaolin-agent` 注册（`builtin_tools/mod.rs` L288–294）。
- **影响**：绕过 sandbox 与命令注入检测的通道
- **建议**：弃用并 unregister，或复用 `shell_readonly` + sandbox 管线
- **修复记录**：2026-06-22 exec_command 执行前添加 ShellSecurityChecker 校验

---

#### BUG-004 🔴 SSRF 检查与 HTTP 请求之间存在 DNS TOCTOU

- **状态**：⏭️ DEFERRED: 需要自定义 DNS resolver/connector，影响面大
- **文件**：`crates/xiaolin-security/src/ssrf.rs` L86–109
- **关联文件**：`crates/xiaolin-tools-network/src/lib.rs` L203–236
- **问题**：`ssrf_check_url` 解析 DNS 后返回 OK，但 `reqwest` 在 `send()` 时可能再次解析。DNS rebinding 可绕过私有 IP 检查。
- **影响**：SSRF 攻击可访问内网服务
- **建议**：连接前用解析结果 pin IP（自定义 connector / `reqwest` DNS override），或禁止 hostname 直连
- **修复记录**：

---

#### BUG-005 🔴 飞书加密推送未实现但配置声明了

- **状态**：⏭️ DEFERRED: 需要实现完整 AES-256-CBC 加解密流程

- **文件**：`extensions/feishu/src/plugin.rs` L27
- **关联文件**：`extensions/feishu/src/webhook.rs` L12–13；`extensions/feishu/Cargo.toml` L22–23
- **问题**：配置和 schema 声明了 `encrypt_key`，但代码从未解密 `encrypt` 字段。`hmac`/`sha2` 依赖已引入却未使用。用户开启「加密推送」后事件体不可解析，功能静默失效。
- **影响**：飞书加密推送场景完全不可用
- **建议**：实现 AES-256-CBC + SHA256 签名校验；或未实现前启动时检测 `encrypt_key` 并 fail-fast 告警
- **修复记录**：

---

#### BUG-006 🔴 飞书 OAuth Token 无刷新机制

- **状态**：⏭️ DEFERRED: 需要实现完整 OAuth 授权码 + refresh_token 流程

- **文件**：`extensions/feishu/src/oauth.rs` L8–25
- **关联文件**：`extensions/feishu/src/client.rs` L97–108
- **问题**：`user_access_token` 为静态配置字符串，无 refresh_token、无过期检测、无自动刷新。过期后 bitable/doc/task 等 user-scoped 工具批量失败。
- **影响**：长时间运行后所有用户级 API 调用失败
- **建议**：实现完整 OAuth 授权码 + refresh 流程
- **修复记录**：

---

#### BUG-007 🔴 微信多账号场景下可能选错 API Client

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/plugin.rs` L170–185、L300–303、L410–412
- **问题**：`find_client_for_target` 在找不到 context token 映射时，直接 fallback 到第一个账号。多账号并存时可能串号。
- **影响**：消息发送到错误账号
- **建议**：严格按 `account_id` 选 client，去掉 fallback，找不到则报错
- **修复记录**：2026-06-22 删除 fallback 到第一个账号的逻辑，找不到返回 None

---

### 正确性类

#### BUG-008 🔴 SubAgent 结果截断使用字节索引，多字节字符会 panic

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-agent/src/subagent_manager.rs` L206、L514、L539、L609
- **问题**：`&text[..2000]`、`&task[..100]` 按字节截断。中文/emoji 触发 UTF-8 字符边界 panic。
- **影响**：生产环境 panic 导致任务中断
- **建议**：改为 `text.floor_char_boundary(2000)` 或 `text.chars().take(n).collect()`
- **相关规则**：规则 #1
- **修复记录**：2026-06-22 subagent_manager.rs 所有字节截断改为 floor_char_boundary

---

#### BUG-009 🔴 PTY 空闲清理任务从未启动

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-pty/src/manager.rs` L116–125
- **关联文件**：`crates/xiaolin-gateway/src/state/builder.rs` L1032
- **问题**：`PtySessionManager::start_cleanup_task()` 已实现，但全仓库无调用方。PTY 会话不会自动清理。
- **影响**：长时间运行后 PTY 资源泄漏
- **建议**：在 gateway 启动时调用 `pty_manager.start_cleanup_task()`
- **相关规则**：新增规则 #24
- **修复记录**：2026-06-22 在 builder.rs 中调用 pty_manager.start_cleanup_task()

---

#### BUG-010 🔴 exec_command 过期清理未终止子进程

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-fs/src/exec_command.rs` L402–406
- **问题**：`cleanup_expired()` 用 `retain` 丢弃超时会话，但未调用 `child.kill()`。子进程继续运行成为孤儿进程。
- **影响**：资源泄漏，孤儿进程占用系统资源
- **建议**：在 `retain` 闭包中先 `kill()` 再移除
- **相关规则**：新增规则 #26
- **修复记录**：2026-06-22 cleanup_expired 先 kill 子进程再移除，添加日志

---

#### BUG-011 🔴 Session Actor emit_sync 静默丢弃关键事件

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-session-actor/src/actor.rs` L514–540
- **问题**：`TurnStart` / `TurnAborted` 等同步事件经 `try_send` 发送，channel 满时丢弃且无重试。
- **影响**：UI 状态与后端不一致，用户可能永远看不到 `TurnEnd`
- **建议**：生命周期事件用 `BackpressurePolicy::Block` 或带超时的 `send().await`
- **修复记录**：2026-06-22 生命周期事件改为带超时 send().await，buffer 增至 1024

---

#### BUG-012 🔴 exec_command 输出截断使用字节索引

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-fs/src/exec_command.rs` L383–384
- **问题**：`output[..max_chars]` 按字节截断，多字节 UTF-8 可能 panic。
- **影响**：含中文输出时 panic
- **建议**：用 `char_indices()` 安全截断
- **相关规则**：规则 #1
- **修复记录**：2026-06-22 输出截断改为 chars().take(max_chars).collect()

---

#### BUG-013 🔴 持久化缓存使用 DefaultHasher

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-code/src/symbol_index.rs` L249–253、L286–288
- **问题**：`file_hash` 写入 SQLite `symbol_index.db`，用 `DefaultHasher`。rustc 升级后 hash 变化导致索引失效或行为不一致。
- **影响**：升级 Rust 编译器后缓存全量失效
- **建议**：改用 `blake3` / `sha256`
- **相关规则**：规则 #13
- **修复记录**：2026-06-22 DefaultHasher 替换为 blake3::hash，新增 blake3 依赖

---

#### BUG-014 🔴 PlanPanel 流式 delta 不触发重渲染

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/message-stream/PlanPanel.tsx` L92–99、L144–146
- **问题**：`plan_delta` 仅写入 `bufferRef.current`，只有遇到换行才 `setStableContent`。无换行的流式内容长时间不更新。
- **影响**：用户看到的 Plan 内容延迟显示
- **建议**：用 `requestAnimationFrame` 节流刷新 state
- **相关规则**：新增规则 #25
- **修复记录**：2026-06-22 添加 rAF 节流刷新机制，流式内容实时更新

---

#### BUG-015 🔴 会话消息 hydration 存在竞态

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/message-stream/MessageStream.tsx` L126–143
- **关联文件**：`crates/xiaolin-app/src/lib/stores/stream-store.ts` L307–310
- **问题**：`getSessionMessages` 无 abort 控制，切换会话后旧请求可能覆盖新会话的实时流。`loadChatStream` 整表替换 `streams[chatId]`。
- **影响**：切换会话后消息丢失或显示错误
- **建议**：用 `AbortController`；合并而非替换；仅在 `stream.length === 0` 时 hydrate
- **修复记录**：2026-06-22 添加 AbortController + 仅空流 hydrate + loadChatStream 防覆盖

---

#### BUG-016 🔴 PlanApprovalCard 自动批准可能重复执行

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/message-stream/PlanApprovalCard.tsx` L205–210
- **问题**：`countdown === 0` 时 `useEffect` 调用 `executeActionRef`，但 `isDisabled` 依赖异步 `setApproved`，同一渲染周期或 Strict Mode 下可能触发两次。
- **影响**：重复 approve/reject 操作
- **建议**：用 `useRef` 标记 `autoApprovedRef` 防止重入
- **修复记录**：2026-06-22 添加 autoApprovedRef 防重入守卫

---

#### BUG-017 🔴 飞书 Webhook 模式下 mention_only 失效

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L456–467
- **关联文件**：`extensions/feishu/src/ws/transport.rs` L40–46
- **问题**：Webhook 路径始终设置 `bot_mentioned: false`，不解析 mentions。群聊中未 @ 机器人的消息也会被处理。
- **影响**：群聊中所有消息都触发 bot 响应
- **建议**：在 webhook 中复用 `messaging/inbound/parse.rs` + `mention.rs` 逻辑
- **相关规则**：新增规则 #22
- **修复记录**：2026-06-22 Webhook 复用 parse_im_mentions_from_message，正确设置 bot_mentioned

---

#### BUG-018 🔴 飞书消息去重未接入生产路径

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/messaging/inbound/dedup.rs` L5–46
- **关联文件**：`extensions/feishu/src/ws/transport.rs` L17–71；`extensions/feishu/src/plugin.rs` L518–557
- **问题**：`MessageDedup` 仅有单元测试，WS 事件桥、webhook handler、Gateway 均未调用。
- **影响**：重连/重投/双通道时同一消息被重复处理
- **建议**：在所有入口用 `Arc<Mutex<MessageDedup>>` 去重
- **相关规则**：新增规则 #22
- **修复记录**：2026-06-22 FeishuPlugin 新增 dedup 字段，WS/webhook 入口均调用去重

---

#### BUG-019 🔴 飞书 WebSocket stop 生命周期不完整

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L545–557、L562–574
- **关联文件**：`extensions/feishu/src/ws/client.rs` L154–157、L339–349
- **问题**：`stop()` 仅 `notify_waiters()`，不关闭 WebSocket writer，event bridge 无 `CancellationToken`，stop 后仍可能继续投递消息。
- **影响**：停止后仍有消息处理，资源未释放
- **建议**：close writer、abort/join 后台 task、event bridge 绑定 cancel token
- **相关规则**：新增规则 #26
- **修复记录**：2026-06-22 添加 CancellationToken + 关闭 writer + event bridge cancel

---

## P1 — 建议改进

### 后端核心

#### BUG-020 🟡 热路径 DB 查询失败被静默吞掉

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-gateway/src/chat_pipeline.rs` L689–694
- **问题**：`usage_counts(30).await` 失败时用 `.ok()` 静默降级，无 `warn!`
- **建议**：改为 `match` + `tracing::warn!`
- **相关规则**：规则 #9
- **修复记录**：2026-06-22 usage_counts 失败改为 match + tracing::warn + 降级空 HashMap

---

#### BUG-021 🟡 沙箱拒绝后无二次审批即降级

- **状态**：⏭️ DEFERRED: 需要重新设计 orchestrator 审批流程

- **文件**：`crates/xiaolin-agent/src/runtime/orchestrator.rs` L361–377
- **问题**：Phase 2 已获用户批准后，沙箱 `Denied` 直接以 `SandboxBackend::None` 重试
- **建议**：escalation 前再次确认
- **修复记录**：

---

#### BUG-022 🟡 Message Bus HTTP API 无 HMAC

- **状态**：⏭️ DEFERRED: 需要评估 HMAC key 分发和管理机制

- **文件**：`crates/xiaolin-core/src/bus.rs` L240–248；`crates/xiaolin-gateway/src/state/builder.rs` L585
- **问题**：生产用 `MessageBus::new()` (`hmac_key: None`)，校验跳过
- **建议**：生产启用 `new_with_hmac`
- **修复记录**：

---

#### BUG-023 🟡 Session HTTP 鉴权 helper 为空实现

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-gateway/src/routes/session.rs` L27–28
- **问题**：`ensure_session_http_auth` 恒返回 `Ok(())`
- **建议**：删除或实现
- **修复记录**：2026-06-22 删除空实现的 ensure_session_http_auth 及其调用

---

#### BUG-024 🟡 skills.list 对 deny_list 使用线性查找

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-gateway/src/ws/skills.rs` L58、L117
- **问题**：`deny_list.iter().any(|d| d == &s.id)` 为 O(N×M)
- **建议**：预构建 `HashSet<&str>`
- **相关规则**：规则 #16
- **修复记录**：2026-06-22 deny_list 预构建为 HashSet 后查找

---

#### BUG-025 🟡 HTTP skills 列表使用过滤后 registry

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-gateway/src/routes/chat.rs` L70–74
- **问题**：被 deny 的 skill 从 HTTP 列表消失，用户无法重新启用
- **建议**：与 WS 对齐，用 unfiltered registry + `enabled` 字段
- **相关规则**：规则 #3
- **修复记录**：2026-06-22 HTTP API 改用 unfiltered_skill_registry + deny 标记 enabled

---

#### BUG-026 🟡 Evolution 注入路径未检查 deny list

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-agent/src/runtime/mod.rs` L951–1004
- **问题**：`inject_relevant_skills` 只查 `SkillStore::find_similar`，不检查 registry deny
- **建议**：注入前交叉检查 deny_list
- **相关规则**：规则 #7
- **修复记录**：2026-06-22 AgentRuntime 新增 skills_deny，注入前过滤被 deny 的 skill

---

#### BUG-027 🟡 skills 配置 static vs live 不一致

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-gateway/src/chat_pipeline.rs` L642
- **关联文件**：`crates/xiaolin-gateway/src/state/mod.rs` L2347–2371
- **问题**：`deny` 热更新，`allow`/`promptMode`/`contextBudgetPercent` 需重启
- **建议**：统一从 `config_live` 读取
- **相关规则**：规则 #2
- **修复记录**：2026-06-22 promptMode/contextBudgetPercent 改为从 config_live 读取

---

### 后端基础设施

#### BUG-028 🟡 EventLog 缓冲区满时丢弃事件

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-session/src/event_log.rs` L91–93
- **问题**：`try_send` 失败仅 `warn!`，不重试
- **建议**：增大 buffer 或溢出时 spool 到磁盘
- **修复记录**：2026-06-22 channel 容量增至 2048，失败时 spawn + timeout 重试

---

#### BUG-029 🟡 ForkSession / RollbackTurns 为未实现桩

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-session-actor/src/actor.rs` L219–225
- **问题**：协议层暴露但实际无效果
- **建议**：返回明确错误，或从协议中移除
- **修复记录**：2026-06-22 ForkSession/RollbackTurns 改为返回 Error 事件而非静默忽略

---

#### BUG-030 🟡 ssrfAllowedHosts 完全跳过私有 IP 检查

- **状态**：⏭️ DEFERRED: 需要评估白名单安全策略设计

- **文件**：`crates/xiaolin-security/src/ssrf.rs` L77–84
- **问题**：白名单 host 不做 DNS 解析校验
- **建议**：文档强调风险；或白名单仍解析 DNS
- **修复记录**：

---

#### BUG-031 🟡 MCP OAuth Token 明文落盘

- **状态**：⏭️ DEFERRED: 需要集成 OS keyring 或加密存储方案

- **文件**：`crates/xiaolin-mcp/src/oauth.rs` L393–427
- **问题**：`FileTokenStore` 以明文 JSON 存 token
- **建议**：使用 OS keyring 或至少限制文件权限
- **相关规则**：新增规则 #23
- **修复记录**：

---

#### BUG-032 🟡 Landlock 外部模式 policy 序列化失败会 panic

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-sandbox/src/landlock.rs` L115–116
- **问题**：`serde_json::to_string(fs_policy).unwrap_or_else(|err| panic!(...))`
- **建议**：返回 `Result`
- **修复记录**：2026-06-22 返回 Result + PolicySerializationFailed 错误，不再 panic

---

#### BUG-033 🟡 FileStateCache::check_stale 同步读取整文件

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-fs/src/file_state_cache.rs` L117–126
- **问题**：mtime 变化时用 `std::fs::read_to_string` 全量读入，阻塞 async runtime
- **建议**：异步 + 流式 hash
- **修复记录**：2026-06-22 check_stale 改为 async，使用 tokio::fs 读取

---

#### BUG-034 🟡 SkillStore::find_similar 全表扫描 + N+1 查询

- **状态**：⏭️ DEFERRED: 需要 FTS/倒排索引架构改造

- **文件**：`crates/xiaolin-evolution/src/skill_store.rs` L274–309、L691–729
- **问题**：每次调用 SELECT 全部 skills，每个 skill 再查 parameters
- **建议**：FTS/倒排索引；批量加载 parameters
- **修复记录**：

---

#### BUG-035 🟡 Skill 聚类 O(N²) + LCS O(N×M)

- **状态**：⏭️ DEFERRED: 需要 MinHash/LSH 等算法优化

- **文件**：`crates/xiaolin-evolution/src/skill_extractor.rs` L128–142、L390–405
- **问题**：轨迹数增长时 CPU 开销急剧上升
- **建议**：预索引 tool sequence、MinHash/LSH
- **修复记录**：

---

#### BUG-036 🟡 SymbolIndex 全局 Mutex 阻塞所有 lookup

- **状态**：⏭️ DEFERRED: 需要 SQLite 多连接 + WAL 架构改造

- **文件**：`crates/xiaolin-tools-code/src/symbol_index.rs` L19–21、L116–145
- **问题**：后台扫描与前台 lookup 共用同一 `Mutex<Connection>`
- **建议**：多连接 + WAL 或读写分离
- **修复记录**：

---

#### BUG-037 🟡 terminal_capture 全文件读入再取尾部

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-fs/src/terminal.rs` L186–200
- **问题**：大 panel 文件时 I/O 开销高
- **建议**：从尾部 seek 读取
- **修复记录**：2026-06-22 大文件从尾部反向 chunk 读取，小文件 async 读取

---

#### BUG-038 🟡 ToolRegistry 锁 poison 时直接 expect panic

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-core/src/tool.rs` L431–437
- **问题**：`.read().expect("ToolRegistry poisoned")` 并发 panic 后可能拖垮 gateway
- **建议**：改用 `parking_lot::RwLock`
- **修复记录**：2026-06-22 ToolRegistry 内部 RwLock 替换为 parking_lot::RwLock

---

#### BUG-039 🟡 Session Actor Mutex 使用 .unwrap()

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-session-actor/src/actor.rs` L299、L521、L543
- **问题**：fanout `Mutex` poison 时 panic
- **建议**：改用 `parking_lot::Mutex`
- **修复记录**：2026-06-22 Session Actor Mutex 替换为 parking_lot::Mutex

---

#### BUG-040 🟡 内存 dedup 指纹用 DefaultHasher

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-evolution/src/skill_extractor.rs` L381–387
- **问题**：虽非持久化，但与项目规范不一致
- **建议**：统一 `blake3`
- **修复记录**：2026-06-22 cluster_fingerprint 改用 blake3

---

#### BUG-041 🟡 FileStateCache 使用 DefaultHasher

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-tools-fs/src/file_state_cache.rs` L218–221
- **问题**：与项目规范不一致
- **建议**：改用 `blake3` 或 `xxhash` 固定 seed
- **修复记录**：2026-06-22 compute_hash 改用 blake3

---

### 前端

#### BUG-042 🟡 WindowResizeHandles 在 early return 之后调用 Hook

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/layout/AppLayout.tsx` L23–34
- **问题**：`if (!isTauri) return null` 在 `useCallback` 之前
- **建议**：将 early return 移到所有 Hook 之后
- **相关规则**：规则 #11
- **修复记录**：2026-06-22 将 early return 移到所有 Hook 之后

---

#### BUG-043 🟡 SessionList 订阅整个 streams 对象

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/session-list/SessionList.tsx` L163
- **问题**：`useStreamStore((s) => s.streams)` 使任意会话消息变化都重渲染整个侧边栏
- **建议**：列表项内用 `useChatStream(chatId)` 逐条订阅
- **修复记录**：2026-06-22 拆分 SessionChatPreview 子组件，单会话独立订阅

---

#### BUG-044 🟡 WS 事件 payload 大量 `as` 断言

- **状态**：⏭️ DEFERRED: 需要设计 WS 事件 discriminated union 类型体系

- **文件**：`crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts` L198–232
- **问题**：后端字段变更时编译期无法感知
- **建议**：定义 discriminated union + 类型守卫
- **修复记录**：

---

#### BUG-045 🟡 TerminalPanel 使用 dangerouslySetInnerHTML

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/shell/TerminalPanel.tsx` L80、L89–98
- **问题**：终端输出经 `ansiToHtml` 转 HTML，ANSI 正则替换后可能含注入内容
- **建议**：改用 xterm.js 或 DOM text + CSS 着色
- **修复记录**：2026-06-22 删除 dangerouslySetInnerHTML，改为 ANSI→React span 渲染

---

#### BUG-046 🟡 列表 key 使用 index

- **状态**：✅ FIXED

- **文件**：`PlanPanel.tsx` L355；`StepIndicator.tsx` L279；`DiffCard.tsx` L103
- **问题**：排序或插入时可能导致错误复用
- **建议**：使用稳定 id
- **修复记录**：2026-06-22 列表 key 改为稳定 id（step+status/line type/src）

---

#### BUG-047 🟡 useMessageStreamChat effect 刻意省略依赖

- **状态**：⏭️ DEFERRED: 需要重构 effect 依赖和 ref 追踪模式

- **文件**：`crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts` L127–243
- **问题**：省略 `streaming` 等依赖，detached stream 逻辑可能不一致
- **建议**：用 ref 追踪 streaming 状态，或补全依赖
- **修复记录**：

---

#### BUG-048 🟡 WechatQrModal 用 useState 充当 interval 容器

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/plugins/PluginsView.tsx` L1867–1871
- **问题**：`useState` 充当 ref 用，Strict Mode 下行为难预测
- **建议**：改为 `useRef`
- **修复记录**：2026-06-22 interval useState 改为 useRef

---

#### BUG-049 🟡 transport / api 双路径并存

- **状态**：⏭️ DEFERRED: 需要跨 22+ 组件统一迁移到 api.ts

- **文件**：22 个组件直接 `import transport`；13 个用 `api`
- **问题**：新增 WS op 时易漏同步三层类型
- **建议**：UI 层统一经 `api.ts` 导出
- **相关规则**：规则 #5, #6
- **修复记录**：

---

#### BUG-050 🟡 MessageStream 测试钩子挂载在 window

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/message-stream/MessageStream.tsx` L81–123
- **问题**：`(window as any).__xiaolin_*` 暴露在生产 bundle
- **建议**：包在 `import.meta.env.DEV` 条件下
- **修复记录**：2026-06-22 测试钩子包裹在 import.meta.env.DEV 条件中

---

#### BUG-051 🟡 PluginsView 单文件 2400+ 行

- **状态**：⏭️ DEFERRED: 需要按功能拆分 2400+ 行文件

- **文件**：`crates/xiaolin-app/src/components/plugins/PluginsView.tsx`
- **问题**：状态、轮询、WeChat 登录、Skills/MCP/Channels 混在一起
- **建议**：按 Tab/Modal 拆分子模块
- **修复记录**：

---

#### BUG-052 🟡 附件 previewUrl 缺少 unmount 清理

- **状态**：✅ FIXED

- **文件**：`crates/xiaolin-app/src/components/message-stream/MessageStream.tsx` L311–318
- **问题**：切换会话时未 `revokeObjectURL`，长期运行可能泄漏
- **建议**：在 `useEffect` cleanup 中 revoke
- **修复记录**：2026-06-22 unmount 时遍历附件和草稿 revokeObjectURL

---

### 扩展模块

#### BUG-053 🟡 飞书 Webhook 验签在 token 未配置时默认放行

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L247–251
- **问题**：`verification_token` 为 None/空时返回 true
- **建议**：生产模式要求非空 token
- **修复记录**：2026-06-22 verify_token 未配置时 fail-closed，新增 allow_insecure_webhook

---

#### BUG-054 🟡 飞书 verify_webhook JSON 解析失败静默通过

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L359
- **问题**：`serde_json::from_slice(...).unwrap_or_default()` 非法 body 得空对象
- **建议**：解析失败应 bail
- **修复记录**：2026-06-22 JSON 解析失败返回错误而非 unwrap_or_default

---

#### BUG-055 🟡 飞书 tenant token 刷新 thundering herd

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/client.rs` L323–364
- **问题**：多并发请求同时穿透到 token API
- **建议**：使用 single-flight / OnceCell
- **修复记录**：2026-06-22 tenant token 刷新改为 Mutex + double-check 模式

---

#### BUG-056 🟡 飞书 WS 分片缓存无 TTL

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/ws/client.rs` L454–476
- **问题**：`fragment_cache` 分片未收齐时永不 eviction
- **建议**：为每个 msg_id 加 timestamp，定期清理
- **相关规则**：新增规则 #27
- **修复记录**：2026-06-22 fragment_cache 条目带时间戳，60 秒 TTL 清理

---

#### BUG-057 🟡 飞书 Webhook 仅处理 text，与能力声明不一致

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L414–417
- **问题**：`capabilities().media = true` 但丢弃 image/file/post
- **建议**：扩展 parse 逻辑，至少对 image/file 生成 attachments
- **修复记录**：2026-06-22 非 text 消息生成占位描述而非丢弃

---

#### BUG-058 🟡 飞书三套并行 inbound 架构

- **状态**：⏭️ DEFERRED: 需要收敛三套 inbound 架构为统一模块

- **文件**：`plugin.rs`（ChannelPlugin）、`ws/transport.rs`（WS 解析）、`channel/handler.rs`（遗留 Axum）
- **问题**：mention 解析在 3 处重复实现，行为已出现分歧
- **建议**：收敛为单一 `inbound` 模块
- **相关规则**：新增规则 #22
- **修复记录**：

---

#### BUG-059 🟡 飞书 im_core_tools 与注释意图不符

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/plugin.rs` L146–157、L502–505
- **问题**：注释称 NOT exposed to LLM，但 `tools()` 合并了 im_core_tools
- **建议**：分层注册 channel_internal_tools vs llm_tools
- **修复记录**：2026-06-22 tools() 只返回 llm_tools()，im_core_tools 改为 #[cfg(test)]

---

#### BUG-060 🟡 微信 message_id 为空时 ReplyCache 可能冲突

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/message.rs` L90–93
- **问题**：空 `message_id` 写入空字符串 key，多条消息共享同一 cache 槽
- **建议**：空 `message_id` 时用 UUID
- **修复记录**：2026-06-22 空 message_id 时生成 UUID 替代

---

#### BUG-061 🟡 微信凭证明文落盘

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/auth/credential.rs` L30–35、L44–57
- **问题**：`bot_token` 以明文 JSON 存储，无文件权限限制
- **建议**：使用加密存储；写入时 chmod 600
- **相关规则**：新增规则 #23
- **修复记录**：2026-06-22 写入后设置文件权限 0600

---

#### BUG-062 🟡 微信 Debug 日志可能泄漏敏感 payload

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/api/client.rs` L131–132
- **问题**：`tracing::debug!(body = %body, "sendMessage body")` 记录完整请求体含 token
- **建议**：仅记录 body_len，敏感字段 redact
- **修复记录**：2026-06-22 Debug 日志只记录 body_len 和关键字段

---

#### BUG-063 🟡 微信 long-poll 每次新建 HTTP Client

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/api/client.rs` L95–97
- **问题**：每次 poll `Client::builder().build()`，无连接池复用
- **建议**：使用专用 client，per-call 设置 timeout
- **修复记录**：2026-06-22 新增 long_poll_client 字段复用 HTTP Client

---

#### BUG-064 🟡 微信 ReplyCache 无界增长

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/plugin.rs` L34–60
- **问题**：每条消息 insert 到 DashMap，无 TTL、无容量上限
- **建议**：LRU + TTL（24h）
- **相关规则**：新增规则 #27
- **修复记录**：2026-06-22 回复后 remove + 容量上限 10000 + 超限 clear

---

#### BUG-065 🟡 微信 ContextToken 持久化在 async 热路径阻塞

- **状态**：✅ FIXED

- **文件**：`extensions/wechat/src/plugin.rs` L115–130
- **问题**：`ContextTokenCache::persist` 使用同步 `std::fs::*`
- **建议**：`tokio::fs` + 防抖批量写
- **修复记录**：2026-06-22 persist 改为 spawn_blocking 异步写盘

---

#### BUG-066 🟡 飞书 media 下载无大小上限

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/client.rs` L291–320
- **问题**：整文件读入 `Vec<u8>`，大文件可能 OOM
- **建议**：设置 max size；流式写入临时文件
- **修复记录**：2026-06-22 下载前检查 Content-Length，流式读取累计超 50MB 报错

---

#### BUG-067 🟡 飞书/微信配置 schema 与运行时行为不一致

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/core/config_schema.rs` L45–48
- **问题**：schema 声明了 `allow_from`、`brand` 但未读取/enforcement
- **建议**：实现过滤，或从 schema 移除
- **修复记录**：2026-06-22 从 schema 移除未实现的 brand/allow_from 字段

---

#### BUG-068 🟡 飞书 UTF-8 不安全字节切片

- **状态**：✅ FIXED

- **文件**：`extensions/feishu/src/ws/client.rs` L186、L194
- **问题**：`&text[..text.len().min(500)]` 中文/emoji 可能 panic
- **建议**：`text.chars().take(500).collect::<String>()`
- **相关规则**：规则 #1
- **修复记录**：2026-06-22 字节截断改为 floor_char_boundary

---

## P2 — 可选优化（🟢）

| # | 问题 | 文件 |
|---|------|------|
| 069 | DefaultHasher 用于运行时 cache break（非持久化） | `agent/runtime/cache_break_detection.rs` |
| 070 | Agent runtime 存在大量 `#[allow(dead_code)]` | `agent/runtime/mod.rs` |
| 071 | `bypass_approval_on_escalation` trait 方法无调用方 | `runtimes/shell.rs` |
| 072 | MCP tools prompt 缓存使用进程级静态 RwLock | `chat_pipeline.rs` |
| 073 | `infer_parameters` 用 Vec::contains 去重 | `skill_extractor.rs` |
| 074 | `filesystem.rs` edit log 使用 as_object_mut().unwrap() | `filesystem.rs` |
| 075 | MessageStream estimateSize 可动态估算 | `MessageStream.tsx` |
| 076 | CopyButton setTimeout unmount 时未 clear | `CopyButton.tsx` |
| 077 | 多处不必要的 HTTP Client / 数据 clone | `feishu/plugin.rs`、`wechat/media/*.rs` |
| 078 | URL 查询参数未统一编码 | `feishu/client.rs` |
| 079 | symbol_index SKIP_DIRS 未含 .cursor/skills | `symbol_index.rs` |
| 080 | SessionActor relay 任务无生命周期绑定 | `actor.rs` |

---

## 新增审查规则

本次审查发现 7 个新的重复模式，已追加到 `.cursor/rules/code-generation-quality.mdc` 规则 #21–#27：

| 规则 | 内容 | 来源问题 |
|------|------|----------|
| #21 | 安全降级必须显式通知用户 | BUG-001 |
| #22 | 多路径入站消息解析必须统一 | BUG-017, BUG-018, BUG-058 |
| #23 | 凭证存储禁止明文落盘 | BUG-031, BUG-061 |
| #24 | 后台清理任务写了必须启动 | BUG-009 |
| #25 | 流式 UI 更新必须触发 React 重渲染 | BUG-014 |
| #26 | 子进程/资源清理必须覆盖所有退出路径 | BUG-010, BUG-019 |
| #27 | 无界缓存/集合必须设置容量上限或 TTL | BUG-056, BUG-064 |
