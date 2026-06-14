# 设计: 修复 Agent 长程任务问题

## 变更范围

### 后端 (Rust)

| 模块 | 文件 | 变更类型 |
|------|------|----------|
| Goal 引擎 | `crates/xiaolin-agent/src/runtime/` | 终止条件修复 |
| Sandbox 文件系统 | `crates/xiaolin-sandbox/` 或相关 | 缓存同步 |
| Shell 工具 | `crates/xiaolin-agent/src/tools/` | execute_command 路径 |
| Sub-agent | `crates/xiaolin-agent/src/subagent/` | 输出截断处理 |
| Terminal | `crates/xiaolin-agent/src/tools/terminal_*` | 超时优化 |

### 前端 (TypeScript/React)

| 模块 | 文件 | 变更类型 |
|------|------|----------|
| 任务进度 | `src/components/` 相关 | 进度计数器同步 |

## 设计原则

1. **Goal 终止**: 宁可提前终止也不要无限循环。当检测到循环（相同操作重复 3+ 次失败）时强制终止并报告
2. **文件可见性**: 写入即可见 — 任何 write_file/sub-agent 写入后立即反映在后续的 glob/list_dir 中
3. **Shell 稳定性**: execute_command 应使用与 terminal_input 相同的 shell 发现逻辑
4. **Sub-agent 截断**: 宁可分多次写也不要截断 — 检测到输出被截断时自动续写

## 测试方案

使用与本次相同的 TaskFlow 开发任务进行回归测试：
1. Goal 模式应在 10 分钟内自然完成
2. 构建成功后不应继续验证循环
3. 所有文件写入后立即可通过 glob 查找
4. npm install 等 shell 命令应一次成功
