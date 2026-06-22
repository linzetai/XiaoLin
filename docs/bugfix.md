# XiaoLin 全仓库代码审查报告

> 审查日期：2026-06-22
> 审查范围：28 个 Rust crate + 2 个 extension + Tauri 前端
> 审查规则：code-generation-quality.mdc（38 条）+ Rust best practices
> 总计发现：**42 HIGH / 110 MEDIUM / 60 LOW = 212 个问题**
> **已修复：156 个** | 剩余：48 个

### 修复进度

| 轮次 | Commit | 修复数 | 优先级 |
|------|--------|-------|--------|
| 第一轮 | `887145f` | 42 | 全部 HIGH |
| 第二轮 | `ebea89f` | 43 | MEDIUM |
| 第三轮 | `02729b3` | 30 | LOW |
| 第四轮 | `bc7f651` | 51 | 无界缓存+错误处理+安全+性能+前端 |

### 标记说明

- ✅ = 已修复
- （无标记）= 未修复

## 问题汇总

| Crate 分组 | HIGH | MEDIUM | LOW | 小计 | 已修复 |
|-----------|------|--------|-----|------|-------|
| xiaolin-core / agent / session / session-actor | 3 | 15 | 7 | 25 | 12 |
| xiaolin-protocol / gateway / mcp | 4 | 10 | 8 | 22 | 12 |
| xiaolin-tools-fs / network / browser / code | 7 | 15 | 7 | 29 | 20 |
| xiaolin-security / sandbox / linux-sandbox / execpolicy / network-proxy | 9 | 19 | 6 | 34 | 19 |
| xiaolin-evolution / memory / context / treesitter | 7 | 12 | 4 | 23 | 14 |
| xiaolin-model-router / guardian / observe / cron / pty / self-iter / benchmark | 4 | 15 | 15 | 34 | 20 |
| xiaolin-app (Tauri 后端 + 前端) | 3 | 14 | 5 | 22 | 8 |
| extensions/feishu + extensions/wechat | 5 | 10 | 8 | 23 | 10 |
| **合计** | **42** | **110** | **60** | **212** | **115** |

---

## 跨 crate Top 10 优先修复项

| # | 问题 | 影响范围 | 严重程度 |
|---|------|---------|---------|
| 1 | execpolicy 默认 fallback 为 Prompt 而非 Deny | 所有工具和网络授权 | HIGH |
| 2 | sandbox Auto 模式静默降级为 Noop（无隔离） | 安全沙箱完整性 | HIGH |
| 3 | hook 执行失败时默认放行工具（fail-open） | Agent 安全执行链 | HIGH |
| 4 | gateway 多处从静态 config 读运行时配置 | 热更新全面失效 | HIGH |
| 5 | chat/skills 热路径同步 DB 查询 usage_counts | 高并发性能 | HIGH |
| 6 | Token 估算全局使用 bytes/4 或 chars/4，中文低估 2-3x | context 溢出/budget 不准 | HIGH |
| 7 | feishu 遗留 webhook.rs 安全缺陷（无认证） | 外部攻击面 | HIGH |
| 8 | network-proxy allow/deny glob 编译失败被静默忽略 | 网络策略失效 | HIGH |
| 9 | read_file 无文件大小上限 + LSP Content-Length 无上限 | OOM/DoS | HIGH |
| 10 | 10+ 处全局 DashMap/HashMap 无容量上限 | 长期运行内存泄漏 | MEDIUM |

---

## 详细问题列表

### 1. xiaolin-core

#### ✅ [HIGH] MessageBus 重放防护在 Mutex poison 时静默失效
- **文件**: `crates/xiaolin-core/src/bus.rs:306-322`
- **问题**: HMAC 校验后重放检测依赖 `seen_ids.lock()`，poison 时 `if let Ok` 跳过，消息仍投递
- **违反规则**: 安全策略 deny-by-default、并发安全
- **修复**: poison 时拒绝投递；使用 `lock().unwrap_or_else(|p| p.into_inner())` + warn

#### ✅ [MEDIUM] 工具权限匹配对 deny/allow 列表做 O(N×M) 线性扫描
- **文件**: `crates/xiaolin-core/src/agent_config.rs:781-808`
- **问题**: `tool_permission()` 对每个工具名遍历 Vec
- **修复**: 预处理为 HashSet

#### ✅ [MEDIUM] 配置文件解析失败时静默回退为空对象并覆盖原文件
- **文件**: `crates/xiaolin-core/src/config_access.rs:114-127`
- **问题**: `json5::from_str` 失败用 `json!({})`，覆盖磁盘文件
- **修复**: 解析失败应 bail 保留原文件

#### ✅ [MEDIUM] BubbleApprovalPort pending map 无界增长
- **文件**: `crates/xiaolin-core/src/tool_runtime.rs:125-159`
- **问题**: DashMap 仅在 resolve/cancel_all 时清理
- **修复**: 加 TTL/容量上限 + 定期 prune

#### ✅ [LOW] history 转换中 JSON 序列化失败静默降级
- **文件**: `crates/xiaolin-core/src/history_compat.rs:101-104, 176-179`
- **问题**: `unwrap_or_default()` 写入空 JSON
- **修复**: warn + 保留原始文本 fallback

---

### 2. xiaolin-agent

#### ✅ [HIGH] Hook 执行失败时默认放行工具
- **文件**: `crates/xiaolin-agent/src/runtime/hook_executor.rs:179-215`
- **问题**: spawn/wait/JSON 解析失败均 return HookResult::allow()
- **修复**: 失败路径返回 block 或可配置 fail-closed 策略

#### ✅ [HIGH] 默认工具策略为 allow-by-default
- **文件**: `crates/xiaolin-core/src/agent_config.rs:575-593`
- **问题**: tools_allow/tools_deny 均空时返回 Allow
- **修复**: 明确产品策略或在 preset 中强制 deny 列表

#### ✅ [MEDIUM] ProcessLlmProvider 子进程无 Drop/shutdown
- **文件**: `crates/xiaolin-agent/src/llm_plugin.rs:1096-1178`
- **修复**: 实现 Drop + kill 子进程

#### ✅ [MEDIUM] AUTO_RECALL_REGISTRY 全局 DashMap 无容量上限
- **文件**: `crates/xiaolin-agent/src/runtime/tool_executor.rs:1468-1523`
- **修复**: 按 session 分区 + LRU

#### ✅ [MEDIUM] CodeGraphCache 全局缓存无 eviction
- **文件**: `crates/xiaolin-agent/src/code_graph.rs:10-40`
- **修复**: LRU max 500 entries

#### ✅ [MEDIUM] MessageQueue 无容量上限
- **文件**: `crates/xiaolin-agent/src/message_queue.rs:30-53`
- **修复**: MAX_QUEUE_SIZE

#### ✅ [MEDIUM] TaskManager 已完成任务永不清理
- **文件**: `crates/xiaolin-agent/src/builtin_tools/task.rs:48-119`
- **修复**: 延迟 GC 或 max 保留条目

#### ✅ [MEDIUM] SESSION_BUDGETS 全局 map 无 session 淘汰
- **文件**: `crates/xiaolin-agent/src/runtime/token_budget.rs:18-35`
- **修复**: session unload 时联动清理

#### ✅ [MEDIUM] 工具参数 JSON 解析失败静默为空对象
- **文件**: `crates/xiaolin-agent/src/runtime/dispatcher.rs:485-486`
- **修复**: 返回 ToolResult::err("invalid arguments JSON")

#### ✅ [MEDIUM] HTTP 响应体读取失败静默为空字符串
- **文件**: `crates/xiaolin-agent/src/llm.rs` 多处
- **修复**: ? 传播或 warn + 明确错误返回

#### ✅ [MEDIUM] Mutex poison 后继续执行工具
- **文件**: `crates/xiaolin-agent/src/runtime/streaming_tool_executor.rs:89-97`
- **修复**: poison 后 abort 整个 executor

#### ✅ [MEDIUM] 工具历史 compact 算法 O(N²)
- **文件**: `crates/xiaolin-agent/src/runtime/tool_executor.rs:1278-1286`
- **修复**: 预建索引或反向单次扫描

#### ✅ [LOW] subagent_manager.as_ref().unwrap() 防御不足
- **文件**: `crates/xiaolin-agent/src/session_bridge.rs:327`

#### ✅ [LOW] DefaultHasher 用于运行时哈希（低风险但需注释）
- **文件**: `crates/xiaolin-agent/src/runtime/cache_break_detection.rs:272-277`

---

### 3. xiaolin-session

#### ✅ [MEDIUM] Schema 迁移错误被 let _ = 静默忽略
- **文件**: `crates/xiaolin-session/src/store.rs:451-459`
- **修复**: 区分 duplicate column 与其他错误

#### ✅ [LOW] 多处 SystemTime 失败使用 unwrap_or_default
- **文件**: `crates/xiaolin-session/src/store.rs` 多处

#### ✅ [LOW] EventLog 队列满时 spawn 失败无反馈
- **文件**: `crates/xiaolin-session/src/event_log.rs:93-100`

---

### 4. xiaolin-session-actor

#### ✅ [MEDIUM] session_approvals 缓存无界增长
- **文件**: `crates/xiaolin-session-actor/src/actor.rs:64-67`

#### ✅ [MEDIUM] relay task 无显式 abort 句柄
- **文件**: `crates/xiaolin-session-actor/src/actor.rs:309-342`

#### ✅ [LOW] get_or_create 对 dead handle 替换不完整
- **文件**: `crates/xiaolin-session-actor/src/manager.rs:70-101`

#### ✅ [LOW] Fanout subscribers 无自动上限
- **文件**: `crates/xiaolin-session-actor/src/fanout.rs:26-81`

---

### 5. xiaolin-protocol

#### ✅ [MEDIUM] parse_request 列表类 op 静默吞掉参数解析错误
- **文件**: `crates/xiaolin-protocol/src/op.rs:891-894`
- **修复**: 改为 serde_json::from_value(params).map_err(...)?

#### [LOW] 已废弃 op 类型仍公开导出
- **文件**: `crates/xiaolin-protocol/src/op.rs:37-52`

---

### 6. xiaolin-gateway

#### ✅ [HIGH] 运行时配置仍从静态 config 快照读取（4 处）
- **文件**: `crates/xiaolin-gateway/src/routes/chat.rs:267`（memory.enabled）
- **文件**: `crates/xiaolin-gateway/src/routes/channel.rs:414-442`（session/group_chat）
- **文件**: `crates/xiaolin-gateway/src/chat_pipeline.rs:843-851`（mcp_servers）
- **文件**: `crates/xiaolin-gateway/src/state/mod.rs:817-884`（evolution 配置）
- **修复**: 统一从 config_live 读取

#### ✅ [HIGH] 每次 chat/skills.list 同步 DB 查询 usage_counts
- **文件**: `crates/xiaolin-gateway/src/chat_pipeline.rs:719-725`
- **修复**: 后台定时刷新 + ArcSwap 缓存

#### ✅ [MEDIUM] Token/字符估算 bytes/4，中文严重低估
- **文件**: `crates/xiaolin-gateway/src/ws/chat.rs:1083-1085` 等多处
- **修复**: 统一用 chars().count() 或 tiktoken

#### ✅ [MEDIUM] memory.rs 摘要截断语义错误（200 字节 ≠ 200 字符）
- **文件**: `crates/xiaolin-gateway/src/routes/memory.rs:191-198`

#### ✅ [MEDIUM] pending_elicitations DashMap 无 TTL/GC
- **文件**: `crates/xiaolin-gateway/src/state/mod.rs:343`

#### ✅ [MEDIUM] 微信登录 LOGIN_SESSIONS.get_mut().unwrap() 可能 panic
- **文件**: `crates/xiaolin-gateway/src/routes/wechat.rs:186`

#### [MEDIUM] 多路径入站消息处理未完全共享
- **文件**: `crates/xiaolin-gateway/src/routes/channel.rs` vs `ws/chat.rs`

#### ✅ [MEDIUM] API Key 鉴权只看启动快照
- **文件**: `crates/xiaolin-gateway/src/routes/health.rs:63`

#### ✅ [MEDIUM] channels.list 启用状态混合静态/动态源
- **文件**: `crates/xiaolin-gateway/src/ws/channels.rs:60`

#### ✅ [LOW] PTY WebSocket serde_json::to_string().unwrap()
- **文件**: `crates/xiaolin-gateway/src/routes/pty.rs:71, 87, 158, 195, 221`

#### ✅ [LOW] Regex::new().unwrap() 每次运行时编译
- **文件**: `crates/xiaolin-gateway/src/chat_pipeline.rs:935`

---

### 7. xiaolin-mcp

#### ✅ [MEDIUM] NEEDS_AUTH_CACHE 只增不减
- **文件**: `crates/xiaolin-mcp/src/lib.rs:2438-2467`

#### ✅ [MEDIUM] WebSocket MCP 断开未发送 Close 帧
- **文件**: `crates/xiaolin-mcp/src/lib.rs:2194-2221`

#### ✅ [MEDIUM] HTTP 错误体 unwrap_or_default() 静默丢弃
- **文件**: `crates/xiaolin-mcp/src/lib.rs:1335, 1355` 等

#### ✅ [LOW] SSE byte_buf 无上限
- **文件**: `crates/xiaolin-mcp/src/lib.rs:1089-1097`

#### ✅ [LOW] naming.rs 工具名对含 __ 的 server_id 有歧义
- **文件**: `crates/xiaolin-mcp/src/naming.rs:49-57`

---

### 8. xiaolin-tools-fs

#### ✅ [HIGH] read_file 读取前无文件大小上限
- **文件**: `crates/xiaolin-tools-fs/src/filesystem.rs:2371-2372`
- **修复**: 读取前 metadata() 检查，超阈值拒绝

#### ✅ [HIGH] git stash 子命令被误判为只读
- **文件**: `crates/xiaolin-tools-fs/src/shell.rs:209-226`
- **修复**: 对 git stash 解析第二参数

#### ✅ [HIGH] FileAccessMode::Full 完全绕过工作区边界
- **文件**: `crates/xiaolin-tools-fs/src/filesystem.rs:210-238`
- **修复**: 增加敏感路径黑名单

#### ✅ [MEDIUM] exec_command 未校验 workdir
- **文件**: `crates/xiaolin-tools-fs/src/exec_command.rs:140-157`

#### ✅ [MEDIUM] enter_worktree 未校验用户指定 path
- **文件**: `crates/xiaolin-tools-fs/src/worktree.rs:139-154`

#### ✅ [MEDIUM] check_stale 在 stat 失败时返回 Fresh
- **文件**: `crates/xiaolin-tools-fs/src/file_state_cache.rs:110-116`

#### ✅ [MEDIUM] search_builtin 无单文件大小限制
- **文件**: `crates/xiaolin-tools-fs/src/filesystem.rs:4116-4119`

#### ✅ [MEDIUM] PtySessionManager 会话表无硬性上限
- **文件**: `crates/xiaolin-tools-fs/src/exec_command.rs:325-401`

#### ✅ [MEDIUM] git_revert_files 静默丢弃删除错误
- **文件**: `crates/xiaolin-tools-fs/src/git.rs:533-536`

#### ✅ [LOW] FileStateCache 无条目上限
- **文件**: `crates/xiaolin-tools-fs/src/file_state_cache.rs:36-50`

#### [LOW] 路径/只读校验逻辑重复且不一致（三套实现）
- **文件**: `crates/xiaolin-tools-fs/src/shell.rs` vs `shell_path_validation.rs` vs `shell_readonly.rs`

---

### 9. xiaolin-tools-network

#### ✅ [HIGH] 搜索引擎全量加载 HTML 无响应体上限
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:583-586, 705-708, 828-831`
- **修复**: 复用 read_response_body_limited

#### ✅ [MEDIUM] http_fetch 描述与实现不一致（4KB vs 5MB）
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:11, 113-116`

#### ✅ [MEDIUM] reqwest::Client::build().unwrap_or_default() 静默降级
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:92-96`

#### ✅ [MEDIUM] SearxngEngine 未对 base_url 做 SSRF 校验
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:446-483`

#### [LOW] strip_html_tags 双倍内存分配
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:1546-1548`

#### ✅ [LOW] truncate_text 注释写 chars 实际按 bytes 截断
- **文件**: `crates/xiaolin-tools-network/src/lib.rs:1682-1696`

---

### 10. xiaolin-tools-browser

#### ✅ [HIGH] 导航 URL 仅校验 scheme，无 SSRF 防护
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:455-463`
- **修复**: 复用 ssrf_check_url

#### ✅ [HIGH] Chrome 启动超时后线程泄漏
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:318-329`

#### [MEDIUM] sandbox(false) 禁用 Chrome 沙箱
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:305-307`

#### [MEDIUM] validate_output_path symlink 绕过风险
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:472-475`

#### [MEDIUM] kill_orphan_chrome 使用 kill -9 可能误杀
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:383-398`

#### [LOW] 全局 Mutex 阻塞所有浏览器操作
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:54-55`

---

### 11. xiaolin-tools-code

#### ✅ [HIGH] LSP 响应 Content-Length 无上限
- **文件**: `crates/xiaolin-tools-code/src/lsp_manager.rs:681-686`
- **修复**: 限制 content_length <= 10MB

#### ✅ [MEDIUM] shutdown_all 不保证子进程回收
- **文件**: `crates/xiaolin-tools-code/src/lsp_manager.rs:360-366`

#### ✅ [MEDIUM] LRU 驱逐 LSP session 时未显式 kill
- **文件**: `crates/xiaolin-tools-code/src/lsp_manager.rs:331-339`

#### ✅ [MEDIUM] 符号索引后台扫描无单文件大小限制
- **文件**: `crates/xiaolin-tools-code/src/symbol_index.rs:291, 398`

#### ✅ [LOW] SymbolIndex::lookup 锁失败静默返回空
- **文件**: `crates/xiaolin-tools-code/src/symbol_index.rs:117-119`

#### ✅ [LOW] notebook 整文件读入内存无上限
- **文件**: `crates/xiaolin-tools-code/src/notebook.rs:101-102`

---

### 12. xiaolin-security

#### ✅ [HIGH] 认证关闭时 API Key 校验恒为通过（fail-open）
- **文件**: `crates/xiaolin-security/src/auth.rs:52-56`

#### ✅ [HIGH] API Key 明文保存在内存快照中
- **文件**: `crates/xiaolin-security/src/auth.rs:22-26`

#### [MEDIUM] WebSocket 允许 query string 传递 API Key
- **文件**: `crates/xiaolin-security/src/auth.rs:127-142`

#### [MEDIUM] Webhook 路径白名单防护不足（无 URL 编码规范化）
- **文件**: `crates/xiaolin-security/src/auth.rs:234-241`

#### [MEDIUM] PermissionProfile::Disabled 默认启用网络
- **文件**: `crates/xiaolin-security/src/permission_profile.rs:179-184`

#### ✅ [MEDIUM] 危险命令策略写入失败被静默忽略
- **文件**: `crates/xiaolin-security/src/dangerous_ops.rs:25-30`

#### ✅ [MEDIUM] SSRF 允许列表写入失败被静默忽略
- **文件**: `crates/xiaolin-security/src/ssrf.rs:11-15`

#### ✅ [MEDIUM] 速率限制桶无容量上限
- **文件**: `crates/xiaolin-security/src/rate_limit.rs:54-56`

#### ✅ [LOW] 安全路径解析中使用 panic!
- **文件**: `crates/xiaolin-security/src/permission_profile.rs:1284-1291`

#### ✅ [LOW] ReadDenyMatcher 路径匹配 O(N²) 嵌套遍历
- **文件**: `crates/xiaolin-security/src/read_deny_matcher.rs:139-156`

---

### 13. xiaolin-sandbox

#### ✅ [HIGH] Landlock/WSL1 不可用时 Auto 模式静默降级为 Noop
- **文件**: `crates/xiaolin-sandbox/src/lib.rs:351-361`

#### ✅ [HIGH] ProxyRouted seccomp 未限制 connect/bind 目标
- **文件**: `crates/xiaolin-sandbox/src/landlock.rs:354-361`

#### [MEDIUM] Landlock BestEffort + PartiallyEnforced 可能误以为已隔离
- **文件**: `crates/xiaolin-sandbox/src/landlock.rs:271-294`

#### ✅ [MEDIUM] 策略序列化失败直接 panic
- **文件**: `crates/xiaolin-sandbox/src/landlock.rs:491-501`

#### [MEDIUM] Legacy Landlock 无法执行 deny-read 时的降级逻辑
- **文件**: `crates/xiaolin-sandbox/src/lib.rs:295-337`

---

### 14. xiaolin-linux-sandbox

#### ✅ [HIGH] CLI 策略 JSON 未经 normalize/校验直接反序列化
- **文件**: `crates/xiaolin-linux-sandbox/src/linux_run_main.rs:195-202`

#### ✅ [HIGH] exec_with_bwrap exec 后 synthetic mount 清理永不执行
- **文件**: `crates/xiaolin-linux-sandbox/src/bwrap.rs:1239-1268`

#### [MEDIUM] Legacy --policy 默认网络策略偏宽松
- **文件**: `crates/xiaolin-linux-sandbox/src/linux_run_main.rs:248-257`

#### [MEDIUM] setup_nftables 存在无效首次 spawn
- **文件**: `crates/xiaolin-linux-sandbox/src/proxy_routing.rs:209-217`

#### [MEDIUM] Bridge 子进程异常退出可能泄漏
- **文件**: `crates/xiaolin-linux-sandbox/src/proxy_routing.rs:488-514`

#### [MEDIUM] glob 展开无 max_depth 时可能扫描整棵树
- **文件**: `crates/xiaolin-linux-sandbox/src/bwrap.rs:654-694`

#### ✅ [LOW] launcher.rs 子进程路径使用 expect
- **文件**: `crates/xiaolin-linux-sandbox/src/launcher.rs:34-36`

---

### 15. xiaolin-execpolicy

#### ✅ [HIGH] 空引擎 / 无匹配规则默认 Prompt 而非 Deny
- **文件**: `crates/xiaolin-execpolicy/src/lib.rs:169-174`
- **修复**: 默认 fallback 改为 forbidden

#### ✅ [HIGH] 网络规则无匹配时默认 Prompt（非 Deny）
- **文件**: `crates/xiaolin-execpolicy/src/lib.rs:447-450`

#### ✅ [MEDIUM] 空 pattern 规则匹配任意命令
- **文件**: `crates/xiaolin-execpolicy/src/matcher.rs:8-10`

#### ✅ [MEDIUM] TOML 加载失败后引擎无「未加载」状态
- **文件**: `crates/xiaolin-execpolicy/src/lib.rs:177-189`

#### ✅ [LOW] get_allowed_prefixes Vec 线性 contains
- **文件**: `crates/xiaolin-execpolicy/src/lib.rs:504-533`

---

### 16. xiaolin-network-proxy

#### ✅ [HIGH] Allow/Deny glob 编译失败被静默忽略
- **文件**: `crates/xiaolin-network-proxy/src/runtime.rs:122-138`
- **修复**: 编译失败时 deny all + error!

#### [MEDIUM] MITM 内层存在独立拦截路径（未统一 evaluate）
- **文件**: `crates/xiaolin-network-proxy/src/mitm.rs`

#### [MEDIUM] NetworkMode::Off 时 evaluate 直接 Allow
- **文件**: `crates/xiaolin-network-proxy/src/runtime.rs`

#### ✅ [MEDIUM] 上游代理存在时跳过 TCP 目标 IP 检查
- **文件**: `crates/xiaolin-network-proxy/src/connect_policy.rs:36-38`

#### [MEDIUM] Allowlist 允许全局通配符 *
- **文件**: `crates/xiaolin-network-proxy/src/policy.rs:171-173`

#### [MEDIUM] MITM CA 私钥落盘未加密
- **文件**: `crates/xiaolin-network-proxy/src/mitm.rs:59-60`

#### ✅ [LOW] dynamic_domains 无 cap
- **文件**: `crates/xiaolin-network-proxy/src/runtime.rs`

---

### 17. xiaolin-evolution

#### ✅ [HIGH] find_similar 全表加载 + 内存评分
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:304-359`
- **修复**: SQL 层初筛或 FTS5

#### ✅ [HIGH] hydrate_skills / row_to_skill N+1 参数查询
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:705-767`
- **修复**: 批量 WHERE skill_id IN (...)

#### ✅ [MEDIUM] extracted_skills 表无全局容量上限
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:98-151`

#### ✅ [MEDIUM] PatternTracker 观测数据无界增长
- **文件**: `crates/xiaolin-evolution/src/skill_extractor.rs:421-468`

#### [MEDIUM] 聚类算法 O(n²)
- **文件**: `crates/xiaolin-evolution/src/skill_extractor.rs:128-142`

#### ✅ [MEDIUM] JSON 解析失败静默跳过无 warn
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:255-261`

#### ✅ [LOW] record_usage 对不存在 skill 静默成功
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:376-385`

#### ✅ [LOW] tokenize 用字节长度过滤 token
- **文件**: `crates/xiaolin-evolution/src/skill_store.rs:814-825`

---

### 18. xiaolin-memory

#### ✅ [HIGH] 语义记忆（facts/relationships）无容量淘汰策略
- **文件**: `crates/xiaolin-memory/src/semantic.rs:265-318`

#### ✅ [HIGH] recall 路径 embedding 失败无日志
- **文件**: `crates/xiaolin-memory/src/manager.rs:172-176`

#### ✅ [MEDIUM] Schema migration 失败被 let _ = 静默吞掉
- **文件**: `crates/xiaolin-memory/src/episodic.rs:105-117`

#### ✅ [MEDIUM] Dream cycle embedding backfill 失败无日志
- **文件**: `crates/xiaolin-memory/src/dreaming.rs:102-130`

#### ✅ [MEDIUM] mark_episodes_dreamed 逐条 UPDATE（N+1）
- **文件**: `crates/xiaolin-memory/src/episodic.rs:135-146`

#### [MEDIUM] 向量搜索候选分片可能漏掉全局最优
- **文件**: `crates/xiaolin-memory/src/episodic.rs:299-363`

---

### 19. xiaolin-context

#### ✅ [HIGH] 全局使用 chars/4 估算 token，中文低估 2-3x
- **文件**: `crates/xiaolin-context/src/compressor.rs:49-52`
- **文件**: `crates/xiaolin-context/src/engine.rs:111-121`

#### ✅ [HIGH] Collapse 用 summary.len()/4（字节长度）
- **文件**: `crates/xiaolin-context/src/collapse.rs:334, 420`

#### ✅ [HIGH] MemoryIngestHook 每次 ingest 触发 DB + 可选 embedding
- **文件**: `crates/xiaolin-context/src/engine.rs:737-794`

#### ✅ [MEDIUM] CachedMicrocompactor 无条目数量上限
- **文件**: `crates/xiaolin-context/src/cached_microcompact.rs:38-42`

#### ✅ [MEDIUM] compress_content 混用字节长度与字符边界
- **文件**: `crates/xiaolin-context/src/cached_microcompact.rs:246-254`

#### ✅ [MEDIUM] Memory ingest 搜索失败无 warn
- **文件**: `crates/xiaolin-context/src/engine.rs:769-794`

#### ✅ [LOW] CompactionHook 使用 std Mutex 可能阻塞 async 路径
- **文件**: `crates/xiaolin-context/src/engine.rs:588-594`

---

### 20. xiaolin-treesitter

#### ✅ [MEDIUM] shell_ast.rs 大量 unwrap() 访问 AST 子节点
- **文件**: `crates/xiaolin-treesitter/src/shell_ast.rs` 多处
- **修复**: 改为 ? + anyhow

#### [LOW] LANG_CACHE DashMap 无显式上限（低影响）
- **文件**: `crates/xiaolin-treesitter/src/parser.rs:7-11`

---

### 21. xiaolin-model-router

#### [MEDIUM] 路由预算检查存在瞬时竞态
- **文件**: `crates/xiaolin-model-router/src/router.rs:232-236`

#### ✅ [MEDIUM] Tier 窗口无匹配时静默放宽约束
- **文件**: `crates/xiaolin-model-router/src/router.rs:80-88`

#### ✅ [MEDIUM] Fixed 策略 preferred 不存在时静默降级
- **文件**: `crates/xiaolin-model-router/src/router.rs:181-187`

#### [MEDIUM] estimate_request 低估非字符串 content
- **文件**: `crates/xiaolin-model-router/src/estimator.rs:212-228`

#### ✅ [LOW] 未知模型费用静默返回 0
- **文件**: `crates/xiaolin-model-router/src/router.rs:130-139`

#### ✅ [LOW] default_usage_charge 每次重建定价表
- **文件**: `crates/xiaolin-model-router/src/estimator.rs:246-257`

#### ✅ [LOW] BudgetTracker.by_model 无界增长
- **文件**: `crates/xiaolin-model-router/src/budget.rs:77-81`

---

### 22. xiaolin-guardian

#### ✅ [HIGH] 整个 crate 未被任何生产代码引用（死代码）
- **文件**: `crates/xiaolin-guardian/src/lib.rs`

#### ✅ [HIGH] max_transcript_tokens 配置从未生效
- **文件**: `crates/xiaolin-guardian/src/lib.rs:22, 420-422`

#### [MEDIUM] Token 估算对 CJK 偏乐观
- **文件**: `crates/xiaolin-guardian/src/lib.rs:422, 516-520`

#### ✅ [MEDIUM] CircuitBreaker 状态 map 无界增长
- **文件**: `crates/xiaolin-guardian/src/lib.rs:183-259`

#### [MEDIUM] JSON 提取用首尾 {} 易误解析
- **文件**: `crates/xiaolin-guardian/src/lib.rs:347-352`

#### [LOW] Allow + Medium 风险未做一致性校验
- **文件**: `crates/xiaolin-guardian/src/lib.rs:383-402`

#### [LOW] build_intent_transcript 大 budget 忽略自定义上限
- **文件**: `crates/xiaolin-guardian/src/lib.rs:541-543`

---

### 23. xiaolin-observe

#### ✅ [HIGH] 双 MetricsCollector 实例导致 metrics 端点数据为空
- **文件**: `crates/xiaolin-observe/src/metrics_collector.rs:342-351`

#### ✅ [MEDIUM] DashMap 指标 key 无界增长
- **文件**: `crates/xiaolin-observe/src/metrics_collector.rs:27-28`

#### ✅ [MEDIUM] 直方图采样 Vec::remove(0) 为 O(n)
- **文件**: `crates/xiaolin-observe/src/metrics_collector.rs:11-16`

#### ✅ [LOW] render_prometheus 每次全量 clone + sort
- **文件**: `crates/xiaolin-observe/src/metrics_collector.rs:151-161`

#### ✅ [LOW] tracing 初始化失败被静默忽略
- **文件**: `crates/xiaolin-observe/src/lib.rs:34-40`

---

### 24. xiaolin-cron

#### ✅ [MEDIUM] fire-and-forget job spawn 无 panic 传播
- **文件**: `crates/xiaolin-cron/src/scheduler.rs:116-129`

#### ✅ [MEDIUM] 执行路径大量 DB 错误被 let _ = 吞掉
- **文件**: `crates/xiaolin-cron/src/scheduler.rs:171, 200-203, 218-220, 229`

#### ✅ [LOW] 无效 cron 表达式无告警
- **文件**: `crates/xiaolin-cron/src/scheduler.rs:139-149`

#### ✅ [LOW] notify_channels JSON 解析失败静默为空
- **文件**: `crates/xiaolin-cron/src/store.rs:452-456`

---

### 25. xiaolin-pty

#### ✅ [MEDIUM] 达到 max_sessions 时不尝试驱逐 idle 会话
- **文件**: `crates/xiaolin-pty/src/manager.rs:30-34`

#### ✅ [LOW] 硬编码会话限制无配置入口
- **文件**: `crates/xiaolin-pty/src/manager.rs:11-12`

#### ✅ [LOW] broadcast 订阅者全部离开后不主动 kill session
- **文件**: `crates/xiaolin-pty/src/session.rs:124-126`

---

### 26. xiaolin-self-iter

#### ✅ [MEDIUM] DiagnosisKind::PromptDrift 从未被诊断（死代码）
- **文件**: `crates/xiaolin-self-iter/src/diagnosis.rs:14-21`

#### ✅ [LOW] 迭代失败时 pass_rate 返回历史最佳而非末轮
- **文件**: `crates/xiaolin-self-iter/src/engine.rs:258-270`

---

### 27. xiaolin-benchmark

#### ✅ [HIGH] LiveExecutor 超时后 agent 任务继续运行（孤儿任务）
- **文件**: `crates/xiaolin-benchmark/src/live.rs:372-421`

#### [MEDIUM] MetricsConfig.thresholds 定义但从未 enforced
- **文件**: `crates/xiaolin-benchmark/src/task.rs:78-87`

#### [MEDIUM] environment.max_turns 未传入 agent
- **文件**: `crates/xiaolin-benchmark/src/live.rs:338-363`

#### [MEDIUM] allowed_shell_patterns 逻辑未验证 shell 命令
- **文件**: `crates/xiaolin-benchmark/src/grader.rs:624-627`

#### ✅ [LOW] ReplayExecutor 缺失 fixture 返回空 events
- **文件**: `crates/xiaolin-benchmark/src/runner.rs:156-162`

---

### 28. xiaolin-app（Tauri 后端）

#### ✅ [HIGH] import_data 未限制导入 blob 大小
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/migration.rs:65-74`

#### ✅ [HIGH] transcribe_audio 未限制 base64 音频大小
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/voice.rs:10-19`

#### ✅ [MEDIUM] Gateway 启动失败时向 IPC 泄露内部错误
- **文件**: `crates/xiaolin-app/src-tauri/src/lib.rs:287-291`

#### ✅ [MEDIUM] http_proxy 错误信息泄露且缺少超时
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/http_proxy.rs:43-63`

#### ✅ [MEDIUM] 应用退出时 Gateway 可能无法 shutdown
- **文件**: `crates/xiaolin-app/src-tauri/src/lib.rs:345-354`

#### ✅ [MEDIUM] Whisper 子进程未在应用退出时跟踪/终止
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/voice.rs:89-117`

#### ✅ [MEDIUM] 原生录音线程在应用退出时无清理
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/audio_capture.rs:66-152`

#### ✅ [MEDIUM] clipboard_write_image 未限制 PNG 大小
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/clipboard.rs:96-102`

#### ✅ [MEDIUM] upload_agent_avatar 缺少源文件校验
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/agent.rs:33-38`

#### ✅ [MEDIUM] Skill ZIP 解压缺少体积/条目数限制
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/skill.rs:78-145`

#### ✅ [LOW] 生产代码中的 expect（Tauri build）
- **文件**: `crates/xiaolin-app/src-tauri/src/lib.rs:342-343`

#### ✅ [LOW] 迁移 IPC 错误携带底层细节
- **文件**: `crates/xiaolin-app/src-tauri/src/commands/migration.rs:55-58`

---

### 29. xiaolin-app（前端 TS/TSX）

#### ✅ [HIGH] CodeBlock 在 early return 之后调用 Hook
- **文件**: `crates/xiaolin-app/src/components/message-stream/MarkdownContent.tsx:116-125`
- **修复**: 将 useConfigStore 移到函数顶部

#### ✅ [MEDIUM] readIdentityFiles 缺少后端 identity 字段
- **文件**: `crates/xiaolin-app/src/lib/transport.ts:58-62`

#### ✅ [MEDIUM] 根组件缺少 ErrorBoundary
- **文件**: `crates/xiaolin-app/src/App.tsx:17-31`

#### [MEDIUM] 会话消息一次性全量加载，无分页
- **文件**: `crates/xiaolin-app/src/lib/transport.ts:218-222`

#### ✅ [MEDIUM] useStreamScroll 存在 stale closure
- **文件**: `crates/xiaolin-app/src/components/message-stream/useStreamScroll.ts:41-57`

#### [MEDIUM] 全局 WS 监听器缺少应用级 teardown
- **文件**: `crates/xiaolin-app/src/lib/store.ts:146-148`

#### [MEDIUM] 流式 segment 依赖 rAF 刷新，hidden 窗口可能卡住
- **文件**: `crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts:312-318`

#### [LOW] 开发环境 (window as any) 挂载调试 API
- **文件**: `crates/xiaolin-app/src/components/message-stream/MessageStream.tsx:84-124`

#### [LOW] SubAgentsTab fetchDefs 缺少 t 依赖
- **文件**: `crates/xiaolin-app/src/components/settings/SubAgentsTab.tsx:26-39`

#### [LOW] AppLayout (getCurrentWindow() as any) 绕过类型
- **文件**: `crates/xiaolin-app/src/components/layout/AppLayout.tsx:28-29`

---

### 30. extensions/feishu

#### ✅ [HIGH] WebSocket 与 Webhook 入站解析未统一
- **文件**: `extensions/feishu/src/ws/transport.rs:171-247` vs `plugin.rs`
- **修复**: 抽取单一 InboundMessage 构建函数

#### ✅ [HIGH] encrypt_key 配置存在但未实现事件解密
- **文件**: `extensions/feishu/src/plugin.rs:32, 75`

#### ✅ [HIGH] 遗留 webhook.rs 存在严重安全缺陷
- **文件**: `extensions/feishu/src/webhook.rs:66-104`
- **修复**: 删除或与 FeishuPlugin.verify_webhook 对齐

#### ✅ [HIGH] Card action 回调绕过去重
- **文件**: `extensions/feishu/src/plugin.rs:448-449`

#### ✅ [MEDIUM] MessageDedup 仅有 TTL、无容量上限
- **文件**: `extensions/feishu/src/messaging/inbound/dedup.rs:5-41`

#### ✅ [MEDIUM] WS 分片缓存 fragment_cache 无容量上限
- **文件**: `extensions/feishu/src/ws/client.rs:31`

#### [MEDIUM] get_bot_open_id 存在惊群与多余 HTTP 客户端
- **文件**: `extensions/feishu/src/plugin.rs:238-250`

#### [MEDIUM] user_access_token 明文存于 channel 配置
- **文件**: `extensions/feishu/src/plugin.rs:44, 89`

#### [MEDIUM] WS 文本解析比 Webhook 更严格，可能导致不一致
- **文件**: `extensions/feishu/src/ws/transport.rs:211-214`

#### [LOW] 多套遗留入口并存
- **文件**: `extensions/feishu/src/lib.rs`

---

### 31. extensions/wechat

#### ✅ [HIGH] 入站消息无去重机制
- **文件**: `extensions/wechat/src/monitor.rs:171-173`

#### [MEDIUM] Bot token 明文落盘
- **文件**: `extensions/wechat/src/auth/credential.rs:6-8`

#### ✅ [MEDIUM] ContextTokenCache 无容量上限
- **文件**: `extensions/wechat/src/plugin.rs:90-98`

#### [MEDIUM] Context token 持久化存在并发写竞态
- **文件**: `extensions/wechat/src/plugin.rs:139-158`

#### [MEDIUM] find_client_for_target 首发消息可能无法 outbound
- **文件**: `extensions/wechat/src/plugin.rs:196-201`

#### [MEDIUM] API 错误日志可能泄露敏感响应体
- **文件**: `extensions/wechat/src/api/client.rs:160-168`

#### [MEDIUM] ReplyCache 淘汰策略非 LRU
- **文件**: `extensions/wechat/src/plugin.rs:54-71`

#### ✅ [LOW] 生产代码中 Regex::new().unwrap()
- **文件**: `extensions/wechat/src/api/client.rs:354`

#### ✅ [LOW] Typing keepalive task 缺少兜底清理
- **文件**: `extensions/wechat/src/plugin.rs:355-385`

---

## 审查通过项（正面记录）

| 领域 | 代表实现 |
|------|---------|
| UTF-8 截断 | `skill.rs:573` 正确使用 `.chars().take(77)`；feishu `floor_char_boundary` |
| 路径白名单 | `filesystem.rs` 的 `ensure_within_workspace` + lexical `..` 处理 |
| Shell 注入 | `shell_security.rs` 正则 + AST 双路径 |
| 子进程超时 | `search_via_ripgrep` 30s timeout；PTY Drop kill |
| SSRF（network） | `http_fetch`/`web_fetch` 使用 `ssrf_check_url` |
| Guardian fail-closed | disabled/unavailable/LLM 失败/超时均 deny |
| 持久化 hash | evolution/context 用 `blake3` |
| ForgetPolicy | episodic memory 有完善的淘汰策略 |
| PTY 子进程清理 | `cleanup_idle_sessions` + Drop + kill 已调用 |

---

## 按类型分布的问题模式

| 问题模式 | 出现次数 | 涉及 crate |
|---------|---------|-----------|
| 无界缓存/集合（DashMap/HashMap 无 LRU/TTL） | 25+ | 几乎所有 |
| 错误静默丢弃（let _ = / unwrap_or_default） | 20+ | agent, gateway, cron, memory |
| 安全策略 fail-open（应为 deny-by-default） | 12 | security, execpolicy, sandbox, hook |
| 静态 config vs config_live 误用 | 8 | gateway |
| Token/字符估算不准（bytes/4 对中文） | 6 | context, gateway, guardian |
| IPC/输入无大小限制 | 6 | app, tools-fs, tools-code |
| 子进程/资源清理不完整 | 5 | agent, browser, sandbox, benchmark |
| 多路径入站解析不统一 | 3 | gateway, feishu |
| N+1 DB 查询 | 2 | evolution, memory |
