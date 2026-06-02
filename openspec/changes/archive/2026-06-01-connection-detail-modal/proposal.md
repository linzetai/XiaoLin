## Why

连接页（Connections）的 MCP 服务器和消息通道卡片目前只展示摘要信息（状态、tool 数量、能力标签），用户无法查看完整配置、tool 列表或排查连接错误。需要为两类卡片添加详情弹窗，点击卡片即可查看完整信息并执行管理操作。

## What Changes

- 点击 MCP 服务器卡片打开 **MCP 详情弹窗**，展示：启动配置（command/args/transport/env）、完整 tool 列表（名称+描述）、错误详情、管理操作（重载/删除）
- 点击 Channel 卡片打开 **Channel 详情弹窗**，展示：元数据（别名、连接模式）、能力详情、脱敏配置信息（app_id/domain，敏感字段掩码）、channel 提供的 tools 列表、管理操作（连接/断开/重连）
- 后端新增 `mcp.detail` 和 `channels.detail` WebSocket 方法，返回完整信息
- 前端新增 `McpDetailModal` 和 `ChannelDetailModal` 组件

## Capabilities

### New Capabilities
- `connection-detail-view`: 连接页的 MCP 服务器和 Channel 详情弹窗，包括后端数据获取接口和前端 Modal 组件

### Modified Capabilities

（无现有 spec 需要修改）

## Impact

- **后端**: `xiaolin-gateway/src/ws/mcp.rs` 新增 `handle_mcp_detail`；`xiaolin-gateway/src/ws/channels.rs` 新增 `handle_channels_detail`；`xiaolin-protocol/src/op.rs` 新增 `McpDetail` / `ChannelsDetail` 操作
- **前端**: `transport.ts` / `api.ts` 新增对应的 WS 函数；`ConnectionsPage.tsx` 新增两个详情 Modal 组件
- **依赖**: 无新增依赖
