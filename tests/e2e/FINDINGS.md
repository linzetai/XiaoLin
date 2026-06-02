# E2E 测试发现的问题

测试时间: 2026-05-27
测试环境: `cargo tauri dev` + tauri-mcp
测试 Agent: TEST (model: qwen3.5-plus/bailian)

---

## 问题 #1 (P0): 工具权限审批未传递 sessionId — 导致工具调用永久卡死

**现象**: Agent 调用任何需要审批的工具（shell_exec, write_file 等）时，前端弹出权限审批对话框，用户点击"批准"后工具仍然卡死，streaming 状态永远不结束。

**根因**: `transport.resolveApproval(approvalId, answer)` 调用时**未传递 `sessionId` 参数**，后端 `ResolveApproval` handler 中 `if let Some(sid) = &session_id` 条件不满足，导致审批结果从未传递给 session actor。

**文件**:
- `crates/xiaolin-app/src/lib/transport.ts:536-545`
- `crates/xiaolin-app/src/components/message-stream/StreamFooter.tsx:537-539`

**修复**: 已修复 — 在 `resolveApproval` 函数签名添加 `sessionId` 参数，在 `StreamFooter` 调用时传入 `activeChat?.id`。

**严重程度**: P0 — 阻塞所有工具调用功能

---

## 问题 #2 (P1): Streaming 渲染文本重复

**现象**: Assistant 回复文本出现 2-3 次重复。例如:
- 第1轮: "已记住：X = 42。" (正常)
- 第2轮: "X 的值是X 的值是 42..." (重复2次)
- 第3轮: "42。42。42。" (重复3次)

**规律**: 重复次数随对话轮次递增（1x → 2x → 3x）。

**根因**: `crates/xiaolin-gateway/src/ws/chat.rs` 中 gateway 的 event loop 在收到 `TurnEnd` 事件后 **没有 break**，导致该 subscriber 持续存活。session actor 的 relay task 通过 `subscriber_senders()` 获取所有存活 subscriber 的 sender 并向它们发送事件。因此旧 turn 的 event loop 会继续接收后续 turn 的事件并转发到同一个 `bg_tx`，导致客户端收到 N 份重复事件（第 N 轮重复 N 次）。

**调用链**:
1. Turn 1 完成 → TurnEnd 发送 → gateway event loop 未退出，subscriber 1 仍在 fanout 中
2. Turn 2 开始 → 新 subscriber 2 加入 fanout → relay task 调用 `subscriber_senders()` 返回 [1, 2]
3. 每个事件被发送到 subscriber 1 和 subscriber 2 → 两者都通过 `bg_tx` 发送到 WS → 客户端收到 2 份
4. Turn N 时有 N 个 subscriber 存活 → N 份重复

**文件**: `crates/xiaolin-gateway/src/ws/chat.rs:742-745`

**修复**: 已修复 — 在 event loop 中 `bg_tx.send(resp)` 之后添加 break 条件：当 `is_done`（TurnEnd）、`TurnAborted` 或 `Error` 时退出循环，确保 subscriber 在 turn 结束后被及时释放。

---

## 问题 #3 (P2): React key 重复警告 — tool_executing 事件重复 push

**现象**: Console 大量报错:
```
Encountered two children with the same key, 'call_405bb9aa8d14410ca87b1509'
```

**根因**: `tool_executing` 事件对同一个 `call_id` 多次触发时，每次都 `push` 新的 segment 到 `segmentsRef.current`，导致 React 列表渲染时出现重复 key。

**文件**: `crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts:400-417`

**修复**: 已修复 — 在 push 前检查是否已存在相同 id 的 segment，存在则更新而非新增。

**严重程度**: P2 — 影响渲染正确性，可能关联问题 #2

---

## 问题 #4 (P3): skill_extractor 使用硬编码 gpt-4o-mini 模型

**现象**: 后端日志持续报警:
```
WARN xiaolin_evolution::skill_extractor: LLM skill pattern extraction failed;
error=请求参数错误：model `gpt-4o-mini` is not supported.
```

**根因**: skill_extractor 硬编码使用 `gpt-4o-mini` 模型名，该模型未在当前 provider 中配置。

**影响**: skill 提取退回到规则模式（不影响核心功能），但产生大量无用日志噪音。

**建议**: 使 skill_extractor 的模型可配置，或 fallback 到已配置的模型。

---

## 问题 #5 (P3): tauri-mcp bridge 部分功能缺失

**现象**: `webview_interact` 和 `webview_find_element` 等使用 selector 的工具报错:
```
window.__MCP__.resolveAll is not a function
```

**根因**: MCP bridge plugin 初始化不完整，只注入了 `refs` 和 `reverseRefs` 属性，未注入 `resolveAll`/`resolveRef` 等 DOM 解析函数。

**影响**: 测试只能通过 `webview_execute_js` 直接操作 DOM，无法使用 selector-based 交互。

**Workaround**: 使用 `webview_execute_js` 手动查找和操作元素。

---

## 问题 #6 (P0): 新建会话发送消息时 "session not found" 错误

**现象**: 点击新会话后发送第一条消息，UI 显示 "错误: session not found"，但后端日志显示 chat 实际成功处理。

**根因**: `ws-client.ts` 的 `onmessage` 处理器在处理 RPC 错误响应时，既 reject 了 pending promise，又将错误消息广播到所有类型监听器。当 `MessageStream` 加载新 session 消息（`getSessionMessages("new-xxx")`）返回 404 时，这个错误被 chatStream 的 "error" handler 误接收并显示到 UI。

**调用链**:
1. 点击新会话 → `getSessionMessages("new-xxx")` 发送 RPC
2. 用户发送消息 → `chatStream` 注册 "error" 监听器
3. 服务器返回 `sessions.messages` 的 404 错误
4. ws-client 既 reject promise 又 `emit("error", msg)` → chatStream 的 error handler 误触发

**文件**:
- `crates/xiaolin-app/src/lib/ws-client.ts:122-138`
- `crates/xiaolin-app/src/components/message-stream/useMessageStreamChat.ts:697-700`

**修复**: 已修复 — ws-client 中 RPC 错误响应 reject pending promise 后立即 `return`，不再广播到类型监听器。同时更新 `chatPromise.catch` 以正确显示真正的 chat 错误。

---

## 问题 #7 (P1): StreamFooter "Maximum update depth exceeded" 无限重渲染

**现象**: StreamFooter 组件触发 React 的 "Maximum update depth exceeded" 错误，导致组件树崩溃。

**根因**: `useAgentStore` selector 中 `ac?.messageQueue ?? []` 每次返回新的空数组引用，导致 Zustand 认为值改变并触发重渲染。当 `agentChats[agentId]` 为 undefined 时（如 agents 未加载完成），形成无限重渲染循环。

**文件**: `crates/xiaolin-app/src/components/message-stream/StreamFooter.tsx:386-390`

**修复**: 已修复 — 使用模块级常量 `STABLE_EMPTY_QUEUE` 替代内联 `[]`，保证引用稳定。

---

## 问题 #8 (P2): MentionInput.setText() 未触发 onContentChange — 发送按钮永远 disabled

**现象**: 通过 ref 调用 `mentionInputRef.current.setText("...")` 设置文本后，发送按钮仍然是 disabled 状态，无法点击发送。

**影响范围**:
- 会话切换时恢复 draft 文本后，发送按钮不可用
- 从空白状态点击 suggestion 填入文本后，发送按钮不可用
- 外部事件注入文本后，发送按钮不可用
- E2E 测试框架无法程序化发送消息

**根因**: `MentionInput` 组件的 `useImperativeHandle` 中 `setText()` 方法只调用了内部 `setText(value)` 更新 React state，但**未调用 `onContentChange?.(!!value.trim())`**，导致父组件 `StreamFooter` 的 `inputHasContent` 状态保持为 `false`，send button 的 `disabled` 条件 `!inputHasContent && attachedFiles.length === 0` 始终为 `true`。

同样，`clear()` 方法也未调用 `onContentChange?.(false)`。

**文件**: `crates/xiaolin-app/src/components/message-stream/MentionInput.tsx:397-414`

**修复**: 已修复 — 在 `setText()` 中添加 `onContentChange?.(!!value.trim())`，在 `clear()` 中添加 `onContentChange?.(false)`。

**严重程度**: P2 — 影响多个依赖程序化设置文本的场景

---

## 测试执行总结（第二轮 - 修复后）

| Suite | Case | 结果 | 备注 |
|-------|------|------|------|
| 01 基本对话 | 1.1 简单问答 | ✓ PASS | 回复 "2"，正确 |
| 01 基本对话 | 1.2 多轮上下文 | ✓ PASS | 回复 "42"，上下文正确保持，**无文本重复** |
| 02 文件工具 | 2.1 读文件 | ✓ PASS | 正确读取 "test content for reading" |
| 02 文件工具 | 2.2 写文件 | ✓ PASS | 权限审批正常弹出并处理，文件写入成功 |
| 03 代码工具 | 3.1 搜索代码 | ✓ PASS | 正确定位 ToolRegistry 在 tool.rs:356 |
| 04 Shell | 4.1 简单命令 | ✓ PASS | 执行 echo，输出 "XIAOLIN_TEST_OK" |
| 05 Web 工具 | 5.1 HTTP fetch | ✓ PASS | 正确获取 httpbin.org 响应 |
| 06 错误恢复 | 6.1 不存在文件 | ✓ PASS | Agent 优雅处理"文件不存在" |
| 06 错误恢复 | 6.2 错误后继续 | ✓ PASS | 错误后对话正常继续 |
| 07 多轮对话 | 7.1 上下文保持 | ✓ PASS | 4 轮 filler 后仍记住暗号 "7749" |
| 08 工具发现 | 8.1 时间工具 | ✓ PASS | tool_search 发现并使用 get_current_time |
| 09 会话管理 | 9.1 创建会话 | ✓ PASS | 新会话正常响应 |
| 10 计划模式 | 10.1 切换模式 | ✓ PASS | Agent 理解 plan mode 概念 |
| 11 目标/待办 | 11.1 创建+执行计划 | ⚠️ PARTIAL | 第一步正确，后续偏离指令去修改真实项目（见能力评估） |
| 05 流式行为 | 5.1 打断/恢复 | ✓ PASS | 打断期间无崩溃无错误 |

### 第一轮 vs 第二轮对比

| 问题 | 第一轮 | 第二轮（修复后） |
|------|--------|-----------------|
| P0 权限审批卡死 | ✗ 阻塞所有工具测试 | ✓ 修复（resolveApproval 加 sessionId） |
| P1 文本重复 | ✗ 多轮对话重复 | ✓ 修复（gateway break on terminal events） |
| P2 send button disabled | ✗ 程序化设置文本不触发 | ✓ 修复（onContentChange in setText/clear） |
| Virtuoso 不渲染 | ✗ 页面刷新后消息不显示 | ✓ 修复（Vite HMR 缓存一致性问题） |

---

## Agent 能力评估 (Suite 11 深度分析)

### 测试场景

指令：在 `/tmp/xiaolin-e2e/11-goal/` 目录下执行 4 步登录功能计划（schema.sql, register API, login API, JWT middleware）

### 实际行为

| 步骤 | 预期 | 实际 | 评价 |
|------|------|------|------|
| 1. schema.sql | 在目标目录创建 | ✓ 正确创建在目标目录 | 通过 |
| 2. 注册接口 | 在目标目录创建 | ✗ 分析真实项目结构，试图修改 xiaolin-gateway/Cargo.toml | 偏离指令 |
| 3. 登录接口 | 在目标目录创建 | ✗ 未执行到 | 被步骤 2 阻塞 |
| 4. JWT 中间件 | 在目标目录创建 | ✗ 未执行到 | 被步骤 2 阻塞 |

### 根因分析

| 维度 | 评分 | 具体问题 |
|------|------|---------|
| **Agent 推理** | ⚠️ 70分 | 第一步遵从指令，第二步自作主张认为"应该在真实项目中实现"，忽略了明确的目标目录约束 |
| **工具能力** | ✓ 95分 | read_file, write_file, search_in_files 全部正常，工具链无问题 |
| **编排逻辑** | ⚠️ 60分 | 计划创建后的执行没有 "scope guard"，Agent 可以自由偏离预设路径 |
| **权限管控** | ✓ 85分 | 底层沙箱正确阻止了越界写入，但 "本次全部批准" 粒度太粗 |
| **审批 UX** | ⚠️ 50分 | `approved_for_session` 一旦点击，后续所有操作（含读取任意文件）自动通过，无法按目录/操作类型精细控制 |

### 改进建议

1. **引入 work_dir 写入约束**：Agent 的 write 操作应受 `work_dir` 限制，超出范围的写入需要额外审批
2. **按操作类型分层审批**：
   - 读取：低风险，可自动批准
   - 写入目标目录：中风险，批量批准可接受
   - 写入目标目录外：高风险，必须逐一审批
3. **Agent 指令遵从增强**：在 system prompt 中增加 "严格遵循用户指定的工作目录" 的约束
4. **计划执行 guard**：plan mode 执行时应有 scope validator，检测操作是否在计划范围内

---

## 补充测试执行记录 (2026-05-28)

### 覆盖率提升：26% → 100%

本轮执行了 plan 中剩余的 34 个 test case，总计 46/46 case 已执行。

### 各 Suite 完整结果

| Suite | Case | 结果 | 备注 |
|-------|------|------|------|
| 01 基本对话 | 1.1 简单问答 | ✓ PASS | |
| | 1.2 多轮上下文 | ✓ PASS | |
| | 1.3 长文本输入 | ✓ PASS | 正确返回质数列表 |
| | 1.4 消息队列 | ✓ PASS | 流式中排队第2条消息，完成后自动处理 |
| | 1.5 取消生成 | ✓ PASS | 停止按钮有效，保留部分回复 |
| 02 文件工具 | 2.1 读文件 | ✓ PASS | |
| | 2.2 写文件 | ✓ PASS | |
| | 2.3 编辑文件 | ✓ PASS | 工具功能正确，但有 P1 显示重复 |
| | 2.4 目录列表 | ✓ PASS | |
| | 2.5 Glob 搜索 | ✓ PASS | |
| | 2.6 文件内容搜索 | ✓ PASS | |
| 03 代码工具 | 3.1 查找定义 | ✓ PASS | |
| | 3.2 文件大纲 | ✓ PASS | 提取了完整的 struct/enum 列表 |
| | 3.3 引用查找 | ✓ PASS | 带文件路径+行号 |
| 04 Shell | 4.1 简单命令 | ✓ PASS | |
| | 4.2 多命令 | ✓ PASS | step1/step2 均正确 |
| | 4.3 工作目录 | ⚠️ PARTIAL | cwd 参数未生效 |
| | 4.4 输出捕获 | ✓ PASS | stderr 正确捕获 |
| 05 Web 工具 | 5.1 HTTP GET | ✓ PASS | |
| | 5.2 HTTP POST | ✓ PASS | httpbin 响应正确 |
| 06 跨会话记忆 | 6.1 存储偏好 | ✓ PASS | |
| | 6.2 新会话回忆 | ✓ PASS | Phoenix42 跨会话回忆成功 |
| | 6.3 存储项目事实 | ✓ PASS | |
| | 6.4 跨会话回忆事实 | ✓ PASS | |
| | 6.5 多事实累积 | ✓ PASS | |
| 07 多轮对话 | 7.1 10轮上下文 | ✓ PASS | |
| | 7.2 文件操作上下文 | ✓ PASS | 先读后改正确 |
| | 7.3 任务连续性 | ✓ PASS | 理解"上面的文件"上下文 |
| 08 工具发现 | 8.1 发现 time 工具 | ✓ PASS | |
| | 8.2 发现 Plan 工具 | ✓ PASS | 生成了结构化计划 |
| | 8.3 发现 multi_edit | ✓ PASS | 列出 multi_edit + apply_patch |
| 09 会话管理 | 9.1 新建会话 | ✓ PASS | |
| | 9.2 上下文隔离 | ✓ PASS | 不同会话无法访问其他会话文件上下文 |
| | 9.3 消息计数增长 | ✓ PASS | 4→6 正确 |
| | 9.4 后端持久化 | ✓ PASS | localstorage + symbol_index.db |
| | 9.5 多会话共存 | ✓ PASS | 4 个会话同时存在 |
| 10 计划模式 | 10.1 进入 plan | ✓ PASS | |
| | 10.2 阻止写入 | ⚠️ PARTIAL | write_file 报 missing path bug |
| | 10.3 允许读取 | ✓ PASS | plan 模式下读取正常 |
| | 10.4 退出恢复 | ✓ PASS | 退出后写入成功 |
| | 10.5 Plan 文件 | ✓ PASS | |
| 11 目标/待办 | 11.1 创建 todo | ✓ PASS | |
| | 11.2 读取 todo | ✓ PASS | 3 项 pending 正确 |
| | 11.3 创建 goal | ⚠️ PARTIAL | goal 工具需 tool_search 先发现 |
| | 11.4 查询 goal 工具 | ✓ PASS | tool_search 找到 3 个 goal 工具 |
| | 11.5 Goal 持久 | ✓ PASS | create_goal + get_goal 验证 |

### 新发现的问题

#### 问题 #5 (P1): Streaming 重复 bug 仍存在

**修复状态**: 之前的 `chat.rs` break fix 减轻了问题但未完全根治。在多 iteration 工具链场景（iteration>=5）中仍然出现文本重复。

**复现条件**: Agent 执行 5+ 次 iteration（多轮工具调用）后产生最终回复时，回复文本出现 2-4 次重复。

#### 问题 #6 (P2): write_file 工具 "missing 'path' argument" 错误

**现象**: write_file 工具调用报 "internal error: missing 'path' argument"，Agent 自动 fallback 到 shell_exec。

**影响**: 不阻塞功能（shell fallback 可用），但降低效率和用户体验。

#### 问题 #7 (P3): Agent "记住" 指令不自动触发 memory_store

**现象**: 用户说"请记住 X"，Agent 口头确认"已记住"但未调用 memory_store 持久化工具。需要用户明确说"使用记忆工具"才会真正存储。

#### 问题 #8 (P3): shell_exec cwd 参数可能未正确传递

**现象**: 指定 `/tmp` 作为工作目录执行 pwd，实际输出为 src-tauri 目录路径。

### 统计摘要

- **总 Cases**: 46
- **PASS**: 42 (91%)
- **PARTIAL**: 4 (9%)
- **FAIL**: 0
- **新发现 Bug**: 4 个（1×P1, 1×P2, 2×P3）
