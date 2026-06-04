## 1. 后端计算等级模型

- [ ] 1.1 在 `xiaolin-core` 新增 `ComputeLevel` 枚举（`low` / `medium` / `high` / `extra_high` / `max`）及 `label()` / `description()` 方法
- [ ] 1.2 实现 `ComputeLevel::to_tier() → ComplexityTier` 映射（Low→Tiny … Max→Frontier）
- [ ] 1.3 定义全局默认常量 `DEFAULT_COMPUTE_LEVEL = High`
- [ ] 1.4 实现 `ComputeLevelResolver`：`resolve(session_id) → ComplexityTier`（`session_override.unwrap_or(global_default)`）

## 2. Per-session 算力覆盖

- [ ] 2.1 在 Session 关联状态中增加 `compute_level_override: Option<ComputeLevel>`（仅内存）
- [ ] 2.2 修改模型路由路径：构建 `RouteTierConstraints` 时 `agent_min_tier` 来自 `ComputeLevelResolver.resolve(session_id)`
- [ ] 2.3 确保正在执行的 turn 使用 turn 开始时的 min_tier snapshot（中途 `compute_level.set` 不影响当前 turn）
- [ ] 2.4 Session 关闭时清除 `compute_level_override`

## 3. 计算等级 WS API

- [ ] 3.1 在 `xiaolin-gateway/src/ws/` 新增 `compute_level.rs` handler 模块
- [ ] 3.2 实现 `compute_level.get { session_id }` → 返回 `{ level, level_label, is_override, levels }`
- [ ] 3.3 实现 `compute_level.set { session_id, level }` → 设置覆盖；`level: null` 清除覆盖
- [ ] 3.4 实现 `compute_level.changed` WS 广播事件
- [ ] 3.5 在 WS dispatcher 中注册 `compute_level.*` handler

## 4. 前端计算等级 Store

- [ ] 4.1 创建 `useComputeLevelStore` (Zustand)：per-session 等级 + 可用档位列表
- [ ] 4.2 实现 session 切换时调用 `compute_level.get` 初始化
- [ ] 4.3 实现 `setLevel(sessionId, level)` action：调用 WS API + 乐观更新；监听 `compute_level.changed`

## 5. ComputeLevelSelector 组件

- [ ] 5.1 创建 `ComputeLevelSelector`：⚡ 图标 + 当前标签 + 下拉触发器
- [ ] 5.2 实现下拉菜单：5 档名称 + 描述 + ✓ 选中标记
- [ ] 5.3 实现 Max 档位确认对话框
- [ ] 5.4 实现 Max 档位琥珀色警告指示器（选择器样式 + InputBar 底部提示行）

## 6. InputBar 集成

- [ ] 6.1 在 `StreamFooter` / InputBar 工具栏中，将 `ComputeLevelSelector` 置于 `ModelSelector` 右侧
- [ ] 6.2 验证 session 切换时选择器与 store 同步；窄屏工具栏布局不破坏 chip 顺序

## 7. 验证

- [ ] 7.1 验证各档位切换后 `agent_min_tier` 正确传入 ModelRouter（Low→Tiny … Max→Frontier）
- [ ] 7.2 验证 mid-turn 算力变更不影响当前 turn 路由
- [ ] 7.3 验证 session 关闭后 override 重置
- [ ] 7.4 运行 `cargo clippy -- -D warnings` 与前端 typecheck 确认零警告
