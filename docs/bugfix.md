# XiaoLin 全仓库代码审查报告

> 审查日期：2026-06-22
> 审查范围：28 个 Rust crate + 2 个 extension + Tauri 前端
> 审查规则：code-generation-quality.mdc（38 条）+ Rust best practices
> 总计发现：**42 HIGH / 110 MEDIUM / 60 LOW = 212 个问题**
> **已修复：全部 212 个（100%）** | 剩余：0 个

### 修复进度

| 轮次 | Commit | 修复数 | 优先级 |
|------|--------|-------|--------|
| 第一轮 | `887145f` | 42 | 全部 HIGH |
| 第二轮 | `ebea89f` | 43 | MEDIUM |
| 第三轮 | `02729b3` | 30 | LOW |
| 第四轮 | `bc7f651` | 51 | 无界缓存+错误处理+安全+性能+前端 |
| 第五轮 | `8d6a13a` | 48 | 安全+算法+前端+扩展 |

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

#### ✅ [LOW] 已废弃 op 类型仍公开导出
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

#### ✅ [MEDIUM] 多路径入站消息处理未完全共享
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

#### ✅ [LOW] 路径/只读校验逻辑重复且不一致（三套实现）
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

#### ✅ [LOW] strip_html_tags 双倍内存分配
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

#### ✅ FIXED 🔴 [MEDIUM] CDP Chrome sandbox 硬编码 disabled
- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-browser/src/engine/cdp_engine.rs` L274-300
- **问题**：`launch_fresh_browser` 始终 `sandbox(false)`，桌面非 headless 场景也禁用沙箱
- **修复记录**：2026-06-23 `XIAOLIN_CDP_SANDBOX` 环境变量覆盖；默认 headless 关闭、非 headless 启用

#### ✅ FIXED 🟡 [MEDIUM] validate_output_path symlink 绕过风险
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:472-475`

#### ✅ [MEDIUM] kill_orphan_chrome 使用 kill -9 可能误杀
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:383-398`

#### ✅ [LOW] 全局 Mutex 阻塞所有浏览器操作
- **文件**: `crates/xiaolin-tools-browser/src/lib.rs:54-55`

#### ✅ FIXED 🔴 [HIGH] WebView bridge `open_page` / `screenshot` / `list_pages` stub
- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_bridge.rs`
- **问题**：`open_page`/`screenshot` 为 stub；`list_pages` 返回 enumerate 索引而非 UUID pageId
- **修复记录**：2026-06-23 复用 `create_browser_page`；JS canvas/SVG 截图 fallback；`list_pages` 使用真实 `page_id`

#### ✅ FIXED 🔴 [HIGH] user_takeover 未连通后端
- **状态**：✅ FIXED
- **文件**：`browser_bridge.rs`, `webview_engine.rs`, `commands/browser.rs`, `browser-store.ts`
- **问题**：「取回控制」仅改前端 state，Agent 仍继续执行
- **修复记录**：2026-06-23 `AtomicBool` + `browser_request_takeover` IPC；`dispatch_sync` fail-closed

#### ✅ FIXED 🔴 [HIGH] `user_action_blocked` 不在协议白名单
- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_panel.rs`
- **修复记录**：2026-06-23 加入 `ALLOWED_INTERNAL_MESSAGE_TYPES` 并映射到 `browser-user-action`

#### ✅ FIXED 🔴 [HIGH] WebView 交互类 action 为 stub
- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-browser/src/engine/webview_engine.rs`
- **问题**：fill_form/type_text/press_key/drag/select/upload_file/interact 未实现
- **修复记录**：2026-06-23 JS 注入实现全部交互 action

#### ✅ FIXED 🟡 [MEDIUM] CDP drag 路径未 validate_uid
- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-browser/src/engine/cdp_engine.rs` L1254
- **修复记录**：2026-06-23 drag 前对 from_uid/to_uid 调用 validate_uid

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

#### ✅ [MEDIUM] WebSocket 允许 query string 传递 API Key
- **文件**: `crates/xiaolin-security/src/auth.rs:127-142`

#### ✅ [MEDIUM] Webhook 路径白名单防护不足（无 URL 编码规范化）
- **文件**: `crates/xiaolin-security/src/auth.rs:234-241`

#### ✅ [MEDIUM] PermissionProfile::Disabled 默认启用网络
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

#### ✅ [MEDIUM] Landlock BestEffort + PartiallyEnforced 可能误以为已隔离
- **文件**: `crates/xiaolin-sandbox/src/landlock.rs:271-294`

#### ✅ [MEDIUM] 策略序列化失败直接 panic
- **文件**: `crates/xiaolin-sandbox/src/landlock.rs:491-501`

#### ✅ [MEDIUM] Legacy Landlock 无法执行 deny-read 时的降级逻辑
- **文件**: `crates/xiaolin-sandbox/src/lib.rs:295-337`

---

### 14. xiaolin-linux-sandbox

#### ✅ [HIGH] CLI 策略 JSON 未经 normalize/校验直接反序列化
- **文件**: `crates/xiaolin-linux-sandbox/src/linux_run_main.rs:195-202`

#### ✅ [HIGH] exec_with_bwrap exec 后 synthetic mount 清理永不执行
- **文件**: `crates/xiaolin-linux-sandbox/src/bwrap.rs:1239-1268`

#### ✅ [MEDIUM] Legacy --policy 默认网络策略偏宽松
- **文件**: `crates/xiaolin-linux-sandbox/src/linux_run_main.rs:248-257`

#### ✅ [MEDIUM] setup_nftables 存在无效首次 spawn
- **文件**: `crates/xiaolin-linux-sandbox/src/proxy_routing.rs:209-217`

#### ✅ [MEDIUM] Bridge 子进程异常退出可能泄漏
- **文件**: `crates/xiaolin-linux-sandbox/src/proxy_routing.rs:488-514`

#### ✅ [MEDIUM] glob 展开无 max_depth 时可能扫描整棵树
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

#### ✅ [MEDIUM] MITM 内层存在独立拦截路径（未统一 evaluate）
- **文件**: `crates/xiaolin-network-proxy/src/mitm.rs`

#### ✅ [MEDIUM] NetworkMode::Off 时 evaluate 直接 Allow
- **文件**: `crates/xiaolin-network-proxy/src/runtime.rs`

#### ✅ [MEDIUM] 上游代理存在时跳过 TCP 目标 IP 检查
- **文件**: `crates/xiaolin-network-proxy/src/connect_policy.rs:36-38`

#### ✅ [MEDIUM] Allowlist 允许全局通配符 *
- **文件**: `crates/xiaolin-network-proxy/src/policy.rs:171-173`

#### ✅ [MEDIUM] MITM CA 私钥落盘未加密
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

#### ✅ [MEDIUM] 聚类算法 O(n²)
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

#### ✅ [MEDIUM] 向量搜索候选分片可能漏掉全局最优
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

#### ✅ [LOW] LANG_CACHE DashMap 无显式上限（低影响）
- **文件**: `crates/xiaolin-treesitter/src/parser.rs:7-11`

---

### 21. xiaolin-model-router

#### ✅ [MEDIUM] 路由预算检查存在瞬时竞态
- **文件**: `crates/xiaolin-model-router/src/router.rs:232-236`

#### ✅ [MEDIUM] Tier 窗口无匹配时静默放宽约束
- **文件**: `crates/xiaolin-model-router/src/router.rs:80-88`

#### ✅ [MEDIUM] Fixed 策略 preferred 不存在时静默降级
- **文件**: `crates/xiaolin-model-router/src/router.rs:181-187`

#### ✅ [MEDIUM] estimate_request 低估非字符串 content
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

#### ✅ [MEDIUM] Token 估算对 CJK 偏乐观
- **文件**: `crates/xiaolin-guardian/src/lib.rs:422, 516-520`

#### ✅ [MEDIUM] CircuitBreaker 状态 map 无界增长
- **文件**: `crates/xiaolin-guardian/src/lib.rs:183-259`

#### ✅ [MEDIUM] JSON 提取用首尾 {} 易误解析
- **文件**: `crates/xiaolin-guardian/src/lib.rs:347-352`

#### ✅ [LOW] Allow + Medium 风险未做一致性校验
- **文件**: `crates/xiaolin-guardian/src/lib.rs:383-402`

#### ✅ [LOW] build_intent_transcript 大 budget 忽略自定义上限
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

#### ✅ [MEDIUM] MetricsConfig.thresholds 定义但从未 enforced
- **文件**: `crates/xiaolin-benchmark/src/task.rs:78-87`

#### ✅ [MEDIUM] environment.max_turns 未传入 agent
- **文件**: `crates/xiaolin-benchmark/src/live.rs:338-363`

#### ✅ [MEDIUM] allowed_shell_patterns 逻辑未验证 shell 命令
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

#### ✅ [MEDIUM] 会话消息一次性全量加载，无分页
- **文件**: `crates/xiaolin-app/src/lib/transport.ts:218-222`

#### ✅ [MEDIUM] useStreamScroll 存在 stale closure
- **文件**: `crates/xiaolin-app/src/components/message-stream/useStreamScroll.ts:41-57`

#### ✅ [MEDIUM] 全局 WS 监听器缺少应用级 teardown
- **文件**: `crates/xiaolin-app/src/lib/store.ts:146-148`

#### ✅ [MEDIUM] 流式 segment 依赖 rAF 刷新，hidden 窗口可能卡住
- **文件**: `crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts:312-318`

#### ✅ [LOW] 开发环境 (window as any) 挂载调试 API
- **文件**: `crates/xiaolin-app/src/components/message-stream/MessageStream.tsx:84-124`

#### ✅ [LOW] SubAgentsTab fetchDefs 缺少 t 依赖
- **文件**: `crates/xiaolin-app/src/components/settings/SubAgentsTab.tsx:26-39`

#### ✅ [LOW] AppLayout (getCurrentWindow() as any) 绕过类型
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

#### ✅ [MEDIUM] get_bot_open_id 存在惊群与多余 HTTP 客户端
- **文件**: `extensions/feishu/src/plugin.rs:238-250`

#### ✅ [MEDIUM] user_access_token 明文存于 channel 配置
- **文件**: `extensions/feishu/src/plugin.rs:44, 89`
- **修复记录**: 2026-06-23 `from_channel_config` + `persist_config_key` 字段级加密 app_secret/user_access_token 等


#### ✅ [MEDIUM] WS 文本解析比 Webhook 更严格，可能导致不一致
- **文件**: `extensions/feishu/src/ws/transport.rs:211-214`

#### ✅ [LOW] 多套遗留入口并存
- **文件**: `extensions/feishu/src/lib.rs`

---

### 31. extensions/wechat

#### ✅ [HIGH] 入站消息无去重机制
- **文件**: `extensions/wechat/src/monitor.rs:171-173`

#### ✅ [MEDIUM] Bot token 明文落盘
- **文件**: `extensions/wechat/src/auth/credential.rs:6-8`
- **修复记录**: 2026-06-23 凭证文件整体 AES-256-GCM 加密（`XENC:` 前缀，向后兼容明文）

#### BUG-001 🔴 凭证明文落盘（规则 #23）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-core/src/credential_crypto.rs`
- **关联文件**：`config.rs`, `config_access.rs`, `extensions/wechat/src/auth/credential.rs`, `extensions/feishu/src/plugin.rs`
- **问题**：飞书 app_secret/user_access_token、微信 bot token、LLM API keys 等以明文 JSON 写入磁盘
- **影响**：本地文件读取即可获取全部 channel/LLM 凭证
- **建议**：机器 ID 派生密钥 + AES-256-GCM 字段级/文件级加密，`XENC:` 前缀区分已加密值
- **相关规则**：code-generation-quality #23
- **修复记录**：2026-06-23 新增 `credential_crypto` 模块；`load_config`/`persist_config_key` 透明加解密；微信凭证文件加密；飞书 channel 敏感字段解密

#### BUG-002 🔴 SSRF DNS Rebinding（规则 #41）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-security/src/ssrf.rs`
- **关联文件**：`crates/xiaolin-tools-network/src/lib.rs`, `crates/xiaolin-gateway/src/lib.rs`
- **问题**：`ssrf_check_parsed_url` 校验时解析 DNS 并检查私有 IP，但后续 `reqwest` 连接会再次独立解析，攻击者可在检查通过后切换 DNS 到内网 IP
- **影响**：`http_fetch`/`web_fetch`/Searxng/cron webhook 等出站 HTTP 工具可被 DNS rebinding 绕过 SSRF 防护
- **建议**：SSRF 校验返回已验证 IP，`build_pinned_client` 通过 `resolve_to_addrs` 固定 DNS 解析结果
- **相关规则**：code-generation-quality #41
- **修复记录**：2026-06-23 新增 `ssrf_check_*_pinned` + `build_pinned_client`；network/gateway 调用方改为 per-request pinned client

#### BUG-003 🔴 加密失败静默回退明文（规则 #21、#23）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-core/src/credential_crypto.rs` L156-165
- **关联文件**：`extensions/wechat/src/auth/credential.rs`
- **问题**：`maybe_encrypt_credential` 与微信 `save_credential` 在 AES 加密失败时仍写入明文凭证
- **影响**：加密机制故障时凭证以明文落盘，用户无感知
- **建议**：加密失败返回 `Err`，由调用方决定是否中止保存
- **相关规则**：code-generation-quality #21、#23
- **修复记录**：2026-06-23 `maybe_encrypt_credential` 改为 `Result<String>`；`encrypt_config_secrets` 传播错误；微信凭证保存失败即返回 Err

#### BUG-004 🟡 解密失败注入密文到运行时（规则 #28）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-core/src/credential_crypto.rs` L177-181；`extensions/feishu/src/plugin.rs` L67-72
- **问题**：`decrypt_config_secrets` / 飞书 `decrypt_secret_field` 解密失败时将 `XENC:...` 密文当作 secret 注入，API 调用静默失败
- **影响**：配置损坏或密钥变更后 channel/LLM 调用失败且难以诊断
- **建议**：解密失败时设为空字符串并 `tracing::error!`
- **相关规则**：code-generation-quality #28
- **修复记录**：2026-06-23 R3 复审：`decrypt_config_secrets` 解密 `XENC:` 失败返回 `Err` 阻断启动；飞书 `decrypt_secret_field` 失败返回 `None` 跳过 channel

#### BUG-006 🔴 Feishu webhook 无 encrypt_key 时跳过签名校验（规则 #44）

- **状态**：✅ FIXED
- **文件**：`extensions/feishu/src/webhook_security.rs` L113-122
- **问题**：`verify_lark_webhook_headers` 在 `encrypt_key` 未配置时 `return Ok(())`，回退到 body 内 token 比对，攻击者可伪造 webhook
- **影响**：未配置 encrypt_key 的部署可被伪造/重放 webhook 事件
- **建议**：默认 fail-closed；仅 `allow_insecure_webhook=true` 时允许 token-only 校验并 warn
- **相关规则**：code-generation-quality #44
- **修复记录**：2026-06-23 R3 复审：新增 `allow_insecure_webhook` 参数，默认要求 encrypt_key

#### BUG-007 🟡 IPC/API 错误消息泄露内部细节

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-gateway/src/routes/channel.rs` L54-55；`crates/xiaolin-app/src-tauri/src/commands/clipboard.rs` L136-168
- **问题**：webhook 解析与 clipboard 写入错误将内部解析/IO 细节返回给前端
- **影响**：信息泄露，违反规则 #43
- **建议**：用户侧固定文案，详情写 `tracing::warn!`
- **相关规则**：code-generation-quality #43
- **修复记录**：2026-06-23 R3 复审：统一为 `"invalid webhook payload"` / `"invalid image data"` / `"failed to write clipboard"`

#### BUG-008 🟡 skill 目录复制 symlink 绕过

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/commands/skill.rs` L291-304
- **问题**：`copy_dir_recursive` 使用 `entry.file_type()` 跟随 symlink，可复制允许根目录外文件
- **影响**：恶意 zip/目录可通过 symlink 逃逸 skills 目录白名单
- **建议**：使用 `symlink_metadata`，遇到 symlink 跳过或拒绝
- **修复记录**：2026-06-23 R3 复审：改用 `symlink_metadata` 并跳过 symlink

#### BUG-005 🟡 read_live_field deny list fail-open

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-gateway/src/ws/skills.rs`；`crates/xiaolin-gateway/src/state/mod.rs`
- **问题**：`read_live_field("deny")` 反序列化失败时返回空 Vec，被 deny 的 skill 可能被错误启用
- **影响**：损坏的 live config 导致 deny list 失效
- **建议**：反序列化失败时 warn 并回退静态 `config.skills.deny`
- **修复记录**：2026-06-23 新增 `read_live_field_or_warn`，deny list 三处调用改为静态回退

#### BUG-009 🔴 CDP Chrome sandbox 硬编码 disabled

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-browser/src/engine/cdp_engine.rs` L274-300
- **问题**：`launch_fresh_browser` 始终 `sandbox(false)`，桌面非 headless 场景也禁用 Chrome 沙箱
- **影响**：桌面用户浏览器进程缺少 OS 级沙箱隔离
- **建议**：`XIAOLIN_CDP_SANDBOX` 环境变量；默认 headless 关闭、非 headless 启用
- **修复记录**：2026-06-23 实现环境变量覆盖与条件 warn 日志

#### BUG-010 🔴 BrowserEngine 缺少 action 能力查询

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-tools-browser/src/engine/mod.rs`；`webview_engine.rs`；`lib.rs`
- **问题**：WebView 引擎不支持 drag/pdf/emulate 等 action，但 `execute_action` 无前置检查，返回 stub 错误
- **影响**：Agent 调用不支持的 action 时错误信息不明确
- **建议**：`BrowserEngine::supported_actions()` + `BrowserTool::execute` 前置 capability check
- **修复记录**：2026-06-23 trait 默认方法 + WebView 覆盖 + execute 前置校验

#### BUG-011 🔴 syncFromBackend 覆盖前端独有状态

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src/lib/stores/browser-store.ts` L332-349
- **问题**：`syncFromBackend` 全量替换 `pages`，丢失 `agentControlled` 和 `faviconUrl`
- **影响**：后端同步后 Agent 控制状态与 favicon 闪烁/丢失
- **建议**：merge 模式保留前端独有字段
- **修复记录**：2026-06-23 merge 时保留 `agentControlled` 和 `faviconUrl`

#### BUG-012 🔴 加载失败页无重试入口

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src/components/browser/BrowserPlaceholder.tsx` L90-108
- **关联文件**：`i18n/locales/zh/browser.json`；`i18n/locales/en/browser.json`
- **问题**：failed 状态 overlay 设置 `pointerEvents: none`，用户无法重试加载
- **影响**：页面加载失败后只能关闭重开标签
- **建议**：添加重试按钮调用 `browserReload`
- **修复记录**：2026-06-23 移除 pointerEvents 限制，添加 retry 按钮与 i18n


#### BUG-013 🔴 TaskManager::stop 持锁调用 prune 导致 DashMap 自死锁（规则 #45）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/builtin_tools/task.rs` L224-243
- **关联文件**：`crates/xiaolin-agent/src/builtin_tools/task.rs` L61-74（`prune_oldest_completed_tasks_map`）
- **问题**：`stop()` 通过 `tasks.get_mut(task_id)` 取得 DashMap 分片写锁 `entry`，在 `entry` 仍存活时调用 `prune_oldest_completed_tasks_map(&self.tasks)`，后者 `tasks.iter()` 需锁定所有分片（含 `entry` 持有的分片）→ 自死锁。
- **影响**：`stop_cancels_running_task` 单测**永久挂起**，导致整个 `xiaolin-agent` lib 测试套件无法完成（多线程下挂起在末尾，观测为「测试极慢」，实为死锁，进程 0.1% CPU 等锁 390s+）。生产环境中任何 `task_stop` 调用都可能挂起调用线程。
- **建议**：在 block 内完成对 `entry` 的修改，**先释放分片锁**再调用 prune。
- **相关规则**：新增规则 #45
- **修复记录**：2026-06-24 将 `stop()` 重构为先在内层作用域修改并返回 `cancelled` bool（drop `entry`），再在锁外调用 `prune_oldest_completed_tasks_map`。`stop_cancels_running_task` 由挂起恢复为 0.05s 通过，整套 task 测试 6s 完成。


#### ✅ [MEDIUM] ContextTokenCache 无容量上限
- **文件**: `extensions/wechat/src/plugin.rs:90-98`

#### ✅ [MEDIUM] Context token 持久化存在并发写竞态
- **文件**: `extensions/wechat/src/plugin.rs:139-158`

#### ✅ [MEDIUM] find_client_for_target 首发消息可能无法 outbound
- **文件**: `extensions/wechat/src/plugin.rs:196-201`

#### ✅ [MEDIUM] API 错误日志可能泄露敏感响应体
- **文件**: `extensions/wechat/src/api/client.rs:160-168`

#### ✅ [MEDIUM] ReplyCache 淘汰策略非 LRU
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
| SSRF（network） | `http_fetch`/`web_fetch`/Searxng/webhook 使用 DNS pinning（`ssrf_check_*_pinned` + `build_pinned_client`） |
| Guardian fail-closed | disabled/unavailable/LLM 失败/超时均 deny |
| 持久化 hash | evolution/context 用 `blake3` |
| ForgetPolicy | episodic memory 有完善的淘汰策略 |
| PTY 子进程清理 | `cleanup_idle_sessions` + Drop + kill 已调用 |

---

#### BUG-E2E-1 🔴 创建 Browser 子 WebView 导致 IPC 死锁

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs` L228-292
- **问题**：`create_browser_page` 在 `with_manager` 持有 `BrowserPanelState` mutex 的闭包内调用 `window.add_child()`。`add_child()` 需要 GTK main thread 参与，而新 WebView 的 `on_navigation` 回调也在 GTK main thread 上同步触发并尝试获取同一个 mutex → 经典死锁。第一个页面创建成功（因为还没有回调竞争），第二个页面永久阻塞所有 IPC。
- **影响**：创建第一个 browser 页面后，所有后续 Tauri IPC 调用永久挂起
- **修复记录**：将 `window.add_child()` 从 mutex 持有范围内移出。先释放 lock → 创建 WebView → 再重新获取 lock 添加 page。
- **相关规则**：code-generation-quality #4 (State 字段初始化)

#### BUG-E2E-2 🔴 IPC 创建页面不通知前端 Store

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs` + `crates/xiaolin-app/src/lib/stores/browser-store.ts`
- **问题**：`create_browser_page` 后端成功创建页面但未 emit 事件通知前端。前端 `browser-store` 仅在 `openPage()` 调用时更新，Agent/外部 IPC 直接调用时 Store 不同步 → Browser Tab 不出现。
- **影响**：通过 Agent 工具或外部 IPC 创建的 browser 页面在 UI 中不可见
- **修复记录**：后端 `create_browser_page` 和 `browser_close_page` 分别 emit `browser-page-created` 和 `browser-page-closed` 事件。前端在 `initBrowserEvents` 中监听这两个事件并更新 Store。

#### BUG-E2E-3 🟡 ResizeObserver loop 错误导致 Vite dev server 崩溃

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src/main.tsx`
- **问题**：Browser 占位组件的 `ResizeObserver` 在某些 layout 场景下触发 "ResizeObserver loop completed with undelivered notifications" 错误。Vite HMR 客户端将其当作未处理错误，导致 dev server 进程退出。
- **影响**：开发环境不稳定
- **修复记录**：在 `main.tsx` 入口添加全局 `error` 事件监听器，拦截并阻止 ResizeObserver loop 错误的传播。

#### BUG-E2E-4 🔴 地址栏输入 URL 回车后始终导航到 example.com

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src/components/browser/BrowserAddressBar.tsx` L85-95
- **问题**：`handleSubmit` 的 `useCallback` 依赖数组缺少 `inputValue`。由于 `navigate`（zustand 稳定引用）和 `pageId` 在用户输入时不变，闭包捕获的 `inputValue` 始终是页面打开时的初始 URL（`https://example.com`），用户输入的新 URL 被忽略。
- **影响**：Browser 完全无法导航到用户输入的 URL
- **建议**：将 `inputValue` 加入 `useCallback` 依赖数组
- **相关规则**：无新规则（React hooks 基础错误，已有 ESLint `exhaustive-deps` 规则覆盖）
- **修复记录**：`handleSubmit` 依赖数组 `[pageId, navigate]` → `[pageId, inputValue, navigate]`

#### BUG-E2E-5 🔴 browser_hide_all_pages IPC 挂起导致布局切换失败

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs` L574-592
- **问题**：`browser_hide_all_pages` 和 `browser_show_page` 在循环中对每个页面独立调用 `apply_webview_layout_gtk`，每次都做 `window.run_on_main_thread(closure)` + `rx.recv()` 阻塞往返。当多页面同时存在时，GTK 主线程调度时序问题导致 `rx.recv()` 永远等不到信号。
- **影响**：Fullwidth ↔ Panel 布局切换完全失败（按钮点击无响应）
- **建议**：将多页面位置更新合并为一次 `run_on_main_thread` 调度
- **相关规则**：无新规则（已有规则 #38 覆盖线程同步模式）
- **修复记录**：新增 `apply_webview_layouts_gtk_batch` 将所有页面位置更新合并到单次 GTK 主线程调度中，`browser_show_page` 和 `browser_hide_all_pages` 改用批量 API

#### BUG-E2E-6 🔴 Linux 125% 缩放下 Browser WebView 定位严重偏移

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs` L64-92
- **关联文件**：`crates/xiaolin-app/src-tauri/src/browser_gtk.rs`, `crates/xiaolin-app/src/components/browser/BrowserPlaceholder.tsx`
- **问题**：在 fractional DPR (如 1.25x) 下，前端 `getBoundingClientRect()` 返回 CSS 像素坐标，但 Linux 的 `GtkFixed::move_()` 使用物理像素坐标。直接传递 CSS 坐标导致 WebView 偏移 `(1 - 1/DPR)` 倍距离（125% 下约偏 200+ CSS 像素）。
- **影响**：Linux fractional scaling 环境下 Browser WebView 完全不在正确位置
- **建议**：前端传递 `devicePixelRatio`，Linux GTK 路径乘以 DPR 转换为物理坐标
- **相关规则**：无新规则（平台特定 bug，非通用模式）
- **修复记录**：`browser_resize_webview` IPC 新增 `scaleFactor` 参数，`apply_webview_layout_gtk` 对坐标乘以 scale_factor

#### BUG-E2E-7 🔴 Browser WebView Cookie 无法设置/持久化（Linux WebKitGTK）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_gtk.rs`（FFI cookie 配置）
- **关联文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs`（时序修复 + 环境变量代理）
- **问题**：在 Linux WebKitGTK 2.52 环境下，Browser 子 WebView 无法接收或持久化 Cookie
- **影响**：所有依赖 Cookie 的站点（登录态、会话保持）在 Browser 面板中不可用
- **根因**：两个独立问题叠加：
  1. **Rust crate API 失效**：`webkit2gtk` 2.0.2 crate 的 `WebsiteDataManager::cookie_manager().set_persistent_storage()` 在 WebKitGTK 2.52 上被静默忽略（该 API 自 2.40 起废弃，GTK3 API 无 NetworkSession 替代）
  2. **时序问题**：`wry` 的 `WebviewBuilder` 在 `add_child()` 时立即开始加载页面，cookie 配置在首次导航之后才执行，导致 WebKitGTK 忽略 `set_persistent_storage` 调用
  3. **预防性修复**：`builder.proxy_url()` 内部调用废弃的 `set_network_proxy_settings`，在 WebKitGTK 2.52 上会破坏 cookie jar（proxy_mode 为 "none" 时未触发，但 XiaolinProxy 模式下会触发）
- **修复方案**：
  1. `browser_gtk.rs` 新增 `configure_webview_cookies()` 通过 FFI 直接调用 `webkit_web_context_get_cookie_manager` + `webkit_cookie_manager_set_persistent_storage(SQLite)` + `webkit_cookie_manager_set_accept_policy(Always)`
  2. `commands/browser.rs` Linux 路径先加载 `about:blank`，在 GTK 主线程上配置 cookie 后再 `webview.navigate()` 到目标 URL
  3. `commands/browser.rs` Linux 不使用 `builder.proxy_url()`，改为 `std::env::set_var("http_proxy"/"https_proxy")` 避免触发废弃 API
- **验证结果**：example.com、www.baidu.com 上 JS cookie + 服务器 Set-Cookie + SQLite 持久化均正常
- **相关规则**：无新规则（平台特定 API 废弃兼容性问题，非通用模式）
- **修复记录**：2026-06-23 FFI cookie 配置 + about:blank 时序修复 + 环境变量代理

#### BUG-E2E-8 🔴 Browser WebView 自定义协议 fetch 失败（Linux WebKitGTK）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_panel.rs`（BROWSER_INIT_SCRIPT send 函数）
- **关联文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs`（新增 browser_webview_notify 命令）
- **问题**：在 Linux WebKitGTK 上，Browser 子 WebView 中 `fetch('xiaolin-internal://callback', ...)` 返回 "Load failed"，导致所有 `__XIAOLIN__.notify()` 调用失败
- **影响**：选中文本发送给 Agent、console 日志转发、network 监控、ready 通知等全部不可用
- **根因**：WebKitGTK 的 `webkit_web_context_register_uri_scheme` 注册的自定义 scheme 不支持从 https:// origin 通过 Fetch API 访问。注册为 CORS-enabled 也无效（WebKitGTK 2.52 的已知限制）。wry/Tauri 的自定义协议实现依赖 `register_uri_scheme`，在 Linux 上对子 WebView 完全失效
- **修复方案**：
  1. 新增 Tauri IPC 命令 `browser_webview_notify`，功能等同于 `handle_xiaolin_internal_protocol` 的事件分发逻辑
  2. 修改 `BROWSER_INIT_SCRIPT` 的 `send()` 函数：优先使用 `__TAURI_INTERNALS__.invoke('browser_webview_notify', ...)` 通过 WebKitGTK 原生 message handler (window.webkit.messageHandlers) 通信，不依赖自定义协议
  3. 保留 `fetch('xiaolin-internal://callback')` 作为 fallback（macOS/Windows 可能仍需要）
- **验证结果**：`notify('selection', {action:'ask'/'quote'})` → Chat 输入框正确接收引用文本
- **相关规则**：无新规则（平台 WebView 引擎限制，非通用代码模式）
- **修复记录**：2026-06-23 新增 browser_webview_notify IPC 命令 + BROWSER_INIT_SCRIPT send() 改用 Tauri IPC

#### BUG-E2E-9 🔴 Browser 导航过滤阻止 about:blank + 序列化 panic

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_panel.rs`（is_navigation_allowed）
- **关联文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs`（on_navigation emit json）
- **问题**：
  1. `is_navigation_allowed` 未允许 `about:` scheme，WebKitGTK 在页面跳转过程中会触发 `about:blank` 导航，被过滤器阻止后导致导航失败
  2. `on_navigation` 回调中 `serde_json::json!()` 直接序列化 `PageLoadState::Failed(String)`，该 tagged enum 无法被 `json!` 正确序列化，导致 `unwrap()` panic，应用崩溃
- **影响**：百度等使用中间 about:blank 跳转的站点无法正常导航，且导致整个应用崩溃退出
- **修复方案**：
  1. 在 `is_navigation_allowed` 中添加 `"about" => true` 允许 about:blank
  2. 将 `json!()` 中的 `PageLoadState::Failed(...)` 替换为手写的 JSON 对象 `{"state": "failed", "error": "..."}`
- **相关规则**：无
- **修复记录**：2026-06-23 允许 about scheme + 修复 json 序列化 panic

#### BUG-E2E-10 🔴 Browser target="_blank" 链接不触发新 tab（Linux WebKitGTK）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_panel.rs`（BROWSER_INIT_SCRIPT）
- **关联文件**：`crates/xiaolin-app/src-tauri/src/commands/browser.rs`（on_new_window handler）
- **问题**：在 Linux WebKitGTK 上，用户点击 `target="_blank"` 链接时，WebKitGTK 的 `create` 信号不触发，导致 `on_new_window` handler 从未被调用。但 `window.open()` 能正确触发该信号。
- **影响**：百度首页的"新闻"、"hao123" 等导航链接无法在新 tab 中打开，用户体验严重受损
- **根因**：WebKitGTK 对 `target="_blank"` 链接点击的 `create` 信号触发机制与 `window.open()` 不同。在 wry 的 `connect_decide_policy` 中，`NewWindowAction` 类型返回 `false`（使用默认策略），但 WebKitGTK 2.52 可能未正确传递该决策到 `create` 信号。`window.open()` 直接触发 `create` 信号绕过了 `decide-policy` 流程。
- **修复方案**：在 `BROWSER_INIT_SCRIPT` 中添加 document-level click 事件拦截器（capturing phase），当点击的目标是 `<a target="_blank">` 链接时，`preventDefault()` 阻止默认行为，改用 `window.open(href, '_blank')` 打开。由于 `window.open()` 路径已确认工作，这确保所有 `target="_blank"` 链接都能正确触发新 tab 创建。
- **验证结果**：百度首页"新闻"(news.baidu.com)、"hao123"(www.hao123.com)、自定义注入的 target=_blank 链接均成功在新 tab 打开
- **相关规则**：无新规则（平台 WebView 引擎行为差异，通过 JS 层 polyfill 解决）
- **修复记录**：2026-06-23 BROWSER_INIT_SCRIPT 添加 target=_blank click 拦截 → window.open() 转换

#### BUG-E2E-12 🔴 浏览器代理阻止 localhost 导致 Lazy Import 崩溃

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_gtk.rs` L249
- **关联文件**：`crates/xiaolin-network-proxy/src/runtime.rs`（`host_blocked` 中 `is_loopback_host` 检查）
- **问题**：浏览器代理通过 `configure_webview_proxy` FFI 设置在 WebKitWebContext 上。由于 Linux WebKitGTK 上所有 WebView（含主 app WebView 和浏览器子 WebView）共享同一个 WebContext，代理会拦截主 WebView 的请求。在开发模式下，Vite dev server 在 localhost 提供 lazy-loaded chunks，代理以 `loopback_blocked` 拒绝这些请求，导致 Settings、Plugins、Automation 等 lazy import 的视图崩溃。
- **影响**：开发模式下打开设置面板/插件/自动化视图时应用崩溃，进入 Error Boundary
- **修复方案**：在 `webkit_network_proxy_settings_new` 的 `ignore_hosts` 参数中添加 `localhost`、`127.0.0.1`、`::1`，使 loopback 地址的流量绕过代理直连。生产模式不受影响（主 WebView 通过 custom protocol 加载，不涉及 localhost）。
- **修复记录**：2026-06-23 browser_gtk.rs 添加 localhost/127.0.0.1/::1 到 proxy ignore_hosts

#### BUG-E2E-11 🔴 BrowserNetworkSettings 对话框被原生 WebView 遮挡

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src/components/browser/BrowserTabContent.tsx`
- **关联文件**：`crates/xiaolin-app/src/components/browser/BrowserNetworkSettings.tsx`
- **问题**：`BrowserNetworkSettings` 使用 `fixed inset-0 z-[60]` 定位的 HTML 对话框，但 Tauri WebView 是原生 OS 层渲染，始终覆盖在 HTML 内容之上，导致网络设置对话框被 WebView 完全遮挡不可见。
- **影响**：用户无法看到和操作网络设置对话框
- **建议**：在打开 HTML 对话框/modal 时，必须先隐藏原生 WebView
- **修复记录**：2026-06-23 在 BrowserPanelBody 中添加 useEffect 监听 networkSettingsOpen，打开时 hideAllPages()、关闭时 showActivePage()

#### BUG-E2E-13 🔴 网络代理配置变更后已有 WebView 不热更新

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-app/src-tauri/src/browser_network.rs` L157-169
- **关联文件**：`crates/xiaolin-app/src-tauri/src/browser_gtk.rs`（`reapply_webview_proxy`）、`crates/xiaolin-app/src-tauri/src/browser_panel.rs`（`webview_labels`）
- **问题**：用户在 BrowserNetworkSettings 修改代理/host mapping 后，内置代理（`NetworkProxyState`）已热更新，但 WebView 侧代理仅在 `create_browser_page` 时通过 `configure_webview_proxy` 设置。Linux 上每个 browser 子 WebView 有独立 WebContext，已有页面继续使用旧代理。
- **影响**：已打开的浏览器 tab 在修改代理模式或自定义代理 URL 后仍走旧路由，需关闭重开才能生效
- **修复方案**：`apply_config` 完成后遍历 `BrowserPanelState` 中所有 webview label，在 GTK 主线程对每个调用 `reapply_webview_proxy`（支持 Direct/System/Custom 三种模式）
- **修复记录**：2026-06-23 apply_config 后 reconfigure_open_webview_proxies + browser_gtk reapply_webview_proxy

#### BUG-014 🔴 active_runs 动态状态注入 system prompt 导致每轮 cache 失效

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/prompt_builder.rs`、`crates/xiaolin-agent/src/runtime/mod.rs`、`crates/xiaolin-agent/src/session_bridge.rs`、`crates/xiaolin-agent/src/runtime/agent_context.rs`
- **问题**：`build_subagent_prompt_block` 把活跃 subagent 的 `elapsed_ms`（每秒变化）拼进 delegation guidance，经 `append_prompt` → `push_system_messages_from_prompt` 并入 Tier-2 system message。主 agent 每有活跃 subagent 时，system prompt 字节每轮变化 → provider 自动前缀缓存每轮 miss，成本显著上升。
- **影响**：有活跃 subagent 时主 agent 的 Tier-2 缓存命中率掉到 ~0，多轮对话成本成倍增加
- **修复方案**：剥离 active_runs 出 guidance（guidance 对同一 policy byte-stable）；active_runs 改为 `build_active_runs_context` 生成，经 `AgentContext.active_runs_context` + `inject_user_context` 注入到最后一条 user message 的 `<system_context>`，保持 system prefix byte-stable。reactive loop 每轮重算最新 elapsed（走 user context 不破坏 system 缓存）。
- **相关规则**：prompt-cache D1/D3（零污染）
- **修复记录**：2026-06-24 subagent-optimization Phase 1；新增 4 单测验证 guidance 跨调用 byte-identical

#### BUG-015 🟡 subagent parent context 作为 System role 污染可共享 Tier-2

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/subagent_manager.rs` L882-895
- **问题**：`run_subagent` 的 "Context from parent agent" 作为 `Role::System` message，经 `merge_leading_system_into_tier2` 并入 subagent 的 Tier-2。parent context 是 per-spawn 动态内容，使同类型 subagent 本可共享的 Tier-2 失效。
- **影响**：同类型 subagent 无法复用 Tier-2 缓存
- **修复方案**：parent context 合并进 task 的 `Role::User` message（语义上属于任务输入），不再进 system role
- **相关规则**：prompt-cache D3
- **修复记录**：2026-06-24 subagent-optimization Phase 1

#### BUG-016 🟡 active runs 注入的 elapsed_ms 运行中恒为 0

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/session_bridge.rs`（`build_active_runs_context`）
- **问题**：`SubAgentRun.elapsed_ms` 仅在完成时写入，运行中为 `None`。`build_active_runs_context` 用 `elapsed_ms.unwrap_or(0)`，导致注入给主 agent 的活跃 subagent 进度永远显示 0s elapsed，进度感知失效。
- **影响**：主 agent 在 reactive loop 中无法感知 worker 真实耗时，进度注入形同虚设
- **修复方案**：运行中 worker 从 `created_at` 实时派生 elapsed（`now_ms.saturating_sub(created_at)`）；同时新增运行时字段 `current_tool`，forwarder 在 ToolExecuting/ToolResult 增量更新 tool_calls_made + current_tool
- **修复记录**：2026-06-24 subagent-optimization Phase 2；新增 `active_runs_context_shows_current_tool` 单测

#### BUG-017 🔴 post_tool microcompact 无保护窗口清掉 read_file 全文

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/post_tool.rs` L93-106；`crates/xiaolin-agent/src/runtime/tool_executor.rs` L220-248, L848-860
- **关联文件**：`crates/xiaolin-agent/src/runtime/unified_compact.rs`（已有 `microcompact_tool_results_with_protection` 但未在 post_tool 使用）
- **问题**：每轮工具执行后 `post_tool_processing` 调用无保护的 `microcompact_tool_results`，`read_file`（FullRetain）在超过 `base_keep+2`（128K 上下文仅 6 次）后被 `[faded]`/`[recall-available]` 替换，LLM 失去文件内容被迫反复 read_file；而 `unified_compact` 路径有 3 轮 iteration 保护窗口，post_tool 路径完全未启用。
- **影响**：排查类任务（并行读多文件 + 验证）在同一 turn 内陷入重读循环，无法基于已读内容推进
- **建议**：post_tool 改用 `microcompact_tool_results_with_protection`；`read_file` 单独放宽 FullRetain 窗口（full +8、preview +6、faded 2000 chars）
- **修复记录**：2026-06-24 post_tool 启用 `compute_protected_indices` + `read_file` 专用 `full_retain_tiers`；新增 `read_file_gets_wider_full_retain_window` 单测；2026-06-24 二次调大全系工具保留窗口（base_keep、FullRetain/Summarize/Ephemeral 分层、保护轮次 5）

#### BUG-018 🔴 跨 turn 加载历史时 read_file 结果未进入 LLM 上下文

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-core/src/history_compat.rs`（`expand_assistant_tool_outputs`、`tool_calls_for_persistence`）；`crates/xiaolin-gateway/src/routes/session.rs`（`resolve_session_context`）
- **关联文件**：`crates/xiaolin-gateway/src/ws/chat.rs`（`enriched_tool_calls_json` 仅 UI 持久化）；`crates/xiaolin-context/src/compressor.rs`（`sanitize_tool_call_pairing` 剥离无配对的 tool_calls）
- **问题**：Tool 结果只存在 assistant 的 `tool_calls_json.output` 中，加载后无 `Role::Tool` 消息；`sanitize_tool_call_pairing` 剥离孤儿 tool_calls，LLM 看不到上轮 read_file 内容。WS 双写 history 时 `chat_message_to_history` 不读 `enriched_tool_calls_json`，且 `resolve_session_context` 在 history 非空时完全跳过 messages 表。
- **影响**：新 turn / 刷新后 agent 反复 read_file，UI 有输出但模型上下文为空
- **修复方案**：`expand_assistant_tool_outputs` 合成 Tool 消息；history 无 ToolUse 时回退 messages 表；`chat_message_to_history` 支持 enriched JSON
- **修复记录**：2026-06-24 实现 expand + history 回退 + 双写修复；新增 2 个 history_compat 单测

#### BUG-019 🔴 截断工具输出写入 /tmp 导致 read_file 沙箱拒绝

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/tool_executor.rs`（`truncated_output_dir`、`save_truncated_output`）
- **问题**：超长工具结果被截断后全文保存到 `/tmp/xiaolin_truncated`，提示 agent 用 `read_file` 取回；该路径不在沙箱允许目录内，agent 陷入「截断 → read_file 失败 → 重试」陷阱。
- **影响**：shell/grep 等大输出任务无法取回完整内容，反复失败空转
- **修复方案**：改为 `resolve_state_dir()/data/truncated`（与 read_file 白名单一致）
- **修复记录**：2026-06-24 迁移截断落盘目录；新增 `truncated_output_dir_is_under_state_dir` 单测

#### BUG-020 🔴 read_file 同路径不同 offset 绕过重复检测

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/tool_executor.rs`（`tool_repetition_key`）；`crates/xiaolin-agent/src/runtime/query_state.rs`（`record_tool_call`）
- **问题**：重复检测用 `tool_name\0arguments` 精确匹配，同一文件分段 read（不同 offset/limit）不计入重复，线上出现单文件 30+ 次 read。
- **影响**：排查类任务在单 turn 内反复读同一文件，无法推进
- **修复方案**：`read_file` 按 path 归一化；grep/search 按 pattern+path；shell 按 command
- **修复记录**：2026-06-24 实现 `tool_repetition_key`；新增 `read_file_repetition_key_ignores_offset`、`record_tool_call_same_path_different_offset_counts_together` 单测

#### BUG-021 🟡 只读空转无进展迭代停止

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/query_state.rs`；`crates/xiaolin-agent/src/runtime/post_tool.rs`
- **问题**：连续多轮仅 read/search 无 edit/shell 时无硬性止损，会话可跑 100+ 轮无进展。
- **影响**：长任务空转浪费 token 与时间
- **修复方案**：`record_iteration_progress` + `check_no_progress_stall`（12 轮 warn、25 轮 force_stop）；post_tool 注入 guidance
- **修复记录**：2026-06-24 实现无进展计数与止损；新增 `no_progress_stall_warns_then_force_stops` 单测

#### BUG-022 🔴 子 agent 可嵌套 spawn 导致 sync 链卡死

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/subagent.rs`；`crates/xiaolin-agent/src/runtime/prompt_builder.rs`；`crates/xiaolin-core/src/agent_config.rs`
- **问题**：`current_depth >= 1` 时仍将 `SubAgentTool` 注入子 registry，explore 子 agent 可再 spawn explore/shell；配合 `background:false` 形成 `spawn_and_wait` 同步链，底层 read 空转时整棵子树与主 turn 挂起（会话 `new-1782316465028-ilvswq`：depth 1→2→3 套娃）。
- **影响**：主 agent turn 长时间无 `turn_end`，UI 表现为卡死
- **修复方案**：`current_depth >= 1` 硬拒绝 `spawn_subagent`；移除子 registry 动态注入；子 agent prompt 明确禁止嵌套；`explore`/`research` 默认 `background: true` 避免 Main sync 阻塞
- **修复记录**：2026-06-24 实现嵌套禁止 + explore/research 默认 background；新增 `spawn_subagent_nested_denied_at_depth_one` 单测

#### BUG-023 🔴 `iteration_msg_boundaries` 索引在压缩删消息后失效，保护集为空

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/query_state.rs`；`crates/xiaolin-agent/src/runtime/iteration_check.rs`；`crates/xiaolin-agent/src/runtime/tool_executor.rs`；`crates/xiaolin-agent/src/runtime/unified_compact.rs`；`crates/xiaolin-agent/src/runtime/query_deps.rs`；`crates/xiaolin-agent/src/runtime/post_tool.rs`
- **问题**：`iteration_msg_boundaries` 存的是 push 时刻的 `messages.len()` 位置索引。`unified_pre_query_compact` 内部多步（ContentFilterHook `retain_mut`/`remove`、`pipeline.pre_query_compact`、`collapse::project`、LLM autocompact）会删除消息，使 `messages.len()` 变小，存储的索引失效。`compute_protected_indices` 用 `boundaries[len-N].0` 作为 `protect_from_boundary`：
  - 索引 > 当前 `messages.len()` → 保护集为空
  - 索引指向压缩前较后位置 → 保护集太小，最近读的文件没被保护

  叠加 `cache_window_for_occupancy` ≥90% 占用 2 分钟地板，刚读完的 tool 结果立即被 `time_based_microcompact` 打成 `[time-compacted]`，agent 看到 recall 提示 → 重读 → 再被压缩 → 死循环。会话 `chat_history_1782314914329.md`（1831 行）和 `chat_history_1782314627522.md`（1575 行）即此症状。
- **影响**：agent 反复读同一批文件，永不进入 edit/write 阶段；token 与时间浪费；用户体验上"任务永不完成"
- **修复方案**：
  1. boundary 元组从 `(usize, Instant)` 扩展为 `(usize, Instant, Option<String>)`，第三项是 push 时刻最近 Tool 消息的 `tool_call_id`（stable anchor）
  2. `compute_protected_indices` 优先按 anchor 在当前（已压缩）Vec 里重新定位；anchor 被蒸发时顺序向后找下一个 boundary 的 anchor；都查不到回退到 clamp 后的位置索引
  3. `iteration_check.rs` 在 push 时捕获 anchor
  4. 所有调用点签名同步更新
- **修复记录**：2026-06-25 实现稳定 anchor 重定位 + 向后回退；新增 `compute_protected_indices_resolves_anchor_after_compaction`、`compute_protected_indices_falls_back_when_anchor_evicted`、`t7_protected_reads_survive_pre_query_compact` 集成测试

#### BUG-024 🟡 截断措辞触发 agent 重读（"truncated" → "omitted from this view"）

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-context/src/engine.rs`；`crates/xiaolin-agent/src/runtime/tool_executor.rs`
- **问题**：多层截断栈（工具执行 → ToolResultStorage → time_based_microcompact → microcompact_tool_results_with_protection → ContentFilterHook → post_tool 第二轮 microcompact）中，`ContentFilterHook` 用 `…(N chars truncated)` 措辞，`fade_to_preview` 用 `[N more chars faded. ...]`，`[summarized]` marker 体不带恢复提示。LLM 看到"truncated"/"faded"判断"内容被截断了，很影响质量"，重新读文件——加剧 BUG-023 的循环。
- **影响**：agent 对自身上下文失去信任，倾向于重新读文件，加剧读文件死循环
- **修复方案**：
  1. `ContentFilterHook` 截断通知：`{truncated}\n…({removed} chars truncated)` → `{truncated}\n…({removed} chars omitted from this view; use read_file with offset/limit to see the full content)`
  2. `fade_to_preview`：`[{remaining} more chars faded. Original: ...]` → `[preview only — {remaining} more chars available via read_file. Original: ...]`
  3. `[summarized]` marker 末尾追加 `\n[summary of older tool result — use read_file/grep to re-fetch if needed]`
- **修复记录**：2026-06-25 改文案为"可恢复"措辞；新增 `fade_to_preview_wording_mentions_recoverable` 单测；更新 `content_filter_truncates_long_tool_result` 断言

#### BUG-025 🟡 `microcompact_tool_results_with_protection` 跳过表不全，产生双重标记

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/tool_executor.rs`
- **问题**：`microcompact_tool_results_with_protection` 的跳过表只含 `ONELINER_MARKER`/`FADED_MARKER`/`RECALL_HINT_MARKER`/`TOOL_RESULT_CLEARED_MESSAGE`，缺 `TIME_COMPACTED_MARKER`、`[summarized]`、`[superseded`。`time_based_microcompact` 把结果打成 `[time-compacted] ...` 后，`microcompact_tool_results_with_protection` 再次处理同一消息，把 `[time-compacted]` 当作普通文本截断，产生 `[faded] [time-compacted] ...` 这种嵌套/双重标记。LLM 解读为"数据多次丢失"，重读。
- **影响**：上下文中出现 `[faded] [time-compacted] § ...` 双重标记，LLM 失去信任，重读文件
- **修复方案**：
  1. 新增共享 helper `is_already_compacted(text)`，覆盖所有 marker：`[oneliner]`/`[faded]`/`[time-compacted]`/`[summarized]`/`[recall-available]`/`[superseded`/`[Old tool result content cleared]`（即 `TOOL_RESULT_CLEARED_MESSAGE`）
  2. `microcompact_tool_results_with_protection`、`time_based_microcompact_with_protection`、`collect_eviction_manifest` 三处跳过检查统一调用 helper
  3. `engine.rs::ContentFilterHook::on_assemble` 复制一份同样逻辑（避免 `xiaolin-context` 反向依赖 `xiaolin-agent`），注释指向 `tool_executor::is_already_compacted` 作为 source of truth
- **修复记录**：2026-06-25 抽 `is_already_compacted` helper + 三处调用点统一；新增 `microcompact_skips_time_compacted_and_summarized`、`content_filter_skips_already_compressed` 单测

#### BUG-026 🟡 `cache_window_for_occupancy` ≥90% 地板 2 min 太激进

- **状态**：✅ FIXED
- **文件**：`crates/xiaolin-agent/src/runtime/tool_executor.rs`；`crates/xiaolin-agent/src/runtime/unified_compact.rs`
- **问题**：`cache_window_for_occupancy` 在 ≥90% 占用时返回 2 min 窗口。2 min 短于 LLM round-trip + 工具执行时间，刚读完的 tool 结果在下一轮 pre_query_compact 就超过窗口，被 `time_based_microcompact` 打成 `[time-compacted]`。叠加 BUG-023（保护集为空），形成"读 → 压缩 → recall → 重读"循环。
- **影响**：高占用时最近 read_file 结果立即被压缩，agent 不得不重读
- **修复方案**：
  1. ≥90% 占用地板从 2 min 提到 5 min（300s），和 <90% 档一致
  2. `unified_pre_query_compact` Step 0 智能跳过：计算 cutoff 后，若所有 cutoff 之前的 Tool 消息都在保护集里，整个 Step 0 跳过（不会浪费时间遍历 Vec）
- **修复记录**：2026-06-25 提高地板 + Step 0 智能跳过；新增 `cache_window_floor_is_5min_at_high_occupancy` 单测

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
