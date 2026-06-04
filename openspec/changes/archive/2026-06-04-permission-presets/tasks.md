## 1. 后端权限预设模型

- [x] 1.1 在 `xiaolin-core/src/agent_config.rs` 中定义 `PermissionPreset` 结构体（id, name, description, behavior_override）
- [x] 1.2 定义 `BehaviorOverride` 结构体（approval_strategy, file_access, tools_ask, tools_deny，全部 Option）
- [x] 1.3 实现 `resolve_behavior(preset, global_config) → BehaviorConfig` 合并函数
- [x] 1.4 定义 4 个内置预设常量：suggest, auto-edit, full-auto, plan-only
- [x] 1.5 实现 `PermissionPresetRegistry`：内置预设 + 用户自定义预设加载

## 2. Per-session 权限覆盖

- [x] 2.1 在 `SessionHandle` 或关联状态中增加 `permission_override: Option<String>`（preset_id）
- [x] 2.2 实现 `PermissionResolver` trait：`resolve(session_id) → BehaviorConfig`
- [x] 2.3 修改 `derive_approval_strategy()` 路径：从直接读 `config.behavior` 改为通过 `PermissionResolver`
- [x] 2.4 修改 `ToolOrchestrator` 审批路径：通过 `PermissionResolver` 获取有效 BehaviorConfig
- [x] 2.5 确保正在执行的 turn 不受中途权限变更影响（turn-level snapshot）

## 3. 权限 WS API

- [x] 3.1 在 `xiaolin-gateway/src/ws/` 新增 `permissions.rs` handler 模块
- [x] 3.2 实现 `permissions.get { session_id }` → 返回当前预设 + 可用预设列表
- [x] 3.3 实现 `permissions.set { session_id, preset_id }` → 设置覆盖并广播事件
- [x] 3.4 实现 `permissions.set { session_id, preset_id: null }` → 清除覆盖
- [x] 3.5 实现 `permissions.changed` WS 广播事件
- [x] 3.6 在 WS dispatcher 中注册 `permissions.*` handler

## 4. 前端权限 Store

- [x] 4.1 创建 `usePermissionStore` (Zustand)：per-session 权限状态 + 预设列表
- [x] 4.2 实现初始化逻辑：session 切换时调用 `permissions.get` 获取当前状态
- [x] 4.3 实现 `setPreset(sessionId, presetId)` action：调用 WS API + 乐观更新
- [x] 4.4 监听 `permissions.changed` WS 事件更新 store

## 5. PermissionSelector 组件

- [x] 5.1 创建 `PermissionSelector` React 组件：当前预设名称 + 下拉触发器
- [x] 5.2 实现下拉菜单：预设列表 + 描述 + 选中标记 + "自定义..." 入口
- [x] 5.3 实现 "Full auto" 选择时的确认对话框
- [x] 5.4 实现权限模式视觉指示器（Full-auto 橙色警告，Plan-only 蓝色标记）
- [x] 5.5 集成到 InputBar 工具栏，位于附加按钮右侧

## 6. 审批卡片增强

- [x] 6.1 修改 `ApprovalCard` 组件：增加操作类型标签 + 目标路径/命令预览
- [x] 6.2 实现风险等级视觉编码（safe=绿, caution=黄, danger=红 左侧边框和标签）
- [x] 6.3 实现 "本次全部批准" 按钮逻辑（session-scoped auto-approve）
- [x] 6.4 优化审批卡片布局：从简单按钮组升级为详情+按钮的卡片样式

## 7. SecurityTab 同步

- [x] 7.1 修改 `SecurityTab`：显示当前全局默认预设
- [x] 7.2 确保 SecurityTab 的模式切换与预设体系一致（inferMode/applyMode 使用相同的预设 ID）
- [x] 7.3 在 SecurityTab 增加 "活跃 session 覆盖" 提示（当有 session 使用非默认预设时）

## 8. 验证

- [x] 8.1 验证预设切换后 Agent 审批行为正确（suggest → auto-edit → full-auto → plan-only）
- [x] 8.2 验证 mid-turn 权限变更不影响当前 turn
- [x] 8.3 验证 session 关闭后覆盖重置
- [x] 8.4 运行 `cargo clippy -- -D warnings` 确认零警告
