## 1. 后端: 协议与路由

- [x] 1.1 在 `xiaolin-protocol/src/op.rs` 新增 `McpDetail` 和 `ChannelsDetail` ClientOp 变体
- [x] 1.2 在 `xiaolin-gateway/src/ws/mod.rs` 添加消息路由和 capabilities 声明

## 2. 后端: MCP 详情接口

- [x] 2.1 在 `ws/mcp.rs` 实现 `handle_mcp_detail`：从 config_live 读取配置、从 mcp_handles 获取 tool 列表、从 mcp_status 读取状态，组装返回
- [x] 2.2 env 环境变量值做掩码处理（前4字符 + "****"）

## 3. 后端: Channel 详情接口

- [x] 3.1 在 `ws/channels.rs` 实现 `handle_channels_detail`：从 registry 获取 plugin 元数据和 tools、从 config 读取脱敏配置
- [x] 3.2 敏感字段掩码处理（app_secret、encrypt_key、token 仅显示前4字符 + "****"）

## 4. 前端: API 层

- [x] 4.1 在 `transport.ts` 新增 `McpDetailResult` 和 `ChannelDetailResult` 类型定义
- [x] 4.2 在 `transport.ts` 新增 `mcpDetail(id)` 和 `channelsDetail(id)` WS 函数
- [x] 4.3 在 `api.ts` 导出对应 API 封装

## 5. 前端: 详情弹窗组件

- [x] 5.1 实现 `McpDetailModal` 组件：配置区 + tool 列表 + 错误区 + 操作栏
- [x] 5.2 实现 `ChannelDetailModal` 组件：元数据 + 能力 + 配置 + tools + 操作栏
- [x] 5.3 修改 `McpCard` 和 `ChannelCard`：添加点击事件（打开详情），操作按钮阻止冒泡

## 6. 验证

- [x] 6.1 `cargo clippy -- -D warnings` 零警告
- [x] 6.2 `pnpm tsc --noEmit` 零错误
- [x] 6.3 启动 dev 并通过 MCP 截图验证两个详情弹窗的交互
