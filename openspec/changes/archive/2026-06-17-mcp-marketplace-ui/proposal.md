## Why

XiaoLin 的 MCP Tab 目前只提供已安装 server 的状态管理视图，缺少发现和安装入口。用户必须手动编辑 JSON 配置或通过 Agent 工具添加 MCP Server，门槛较高。Codex 已提供 Plugin Directory（分类浏览 + 一键安装），这是当前 XiaoLin MCP 能力评分从 ~88 提升到 95+ 的关键缺失功能。

## What Changes

- 在 MCP Tab 顶部新增 **Installed / Explore** 子切换，Explore 面板提供内置 MCP Server 目录的浏览、搜索、分类筛选和一键安装
- 新增 **mcp-registry.json** 本地注册表，收录约 15 个热门 MCP Server（filesystem、github、postgres、brave-search 等），包含分类、描述和安装配置
- 新增 **AddServerModal** 自定义添加模态框，支持 Stdio / SSE / Streamable HTTP transport 选择、动态表单和环境变量编辑
- 新增 **McpDetailModal** 详情模态框，展示配置预览、工具列表、错误日志和 Remove 操作
- 扩展 `transport.addMcpServer` API 签名，补充 transport/url/env 参数透传到后端
- 在 plugin-store 新增 `addPlugin` 和 `removePlugin` actions
- 在 PluginsView Header 增加 "+ Add" 按钮和 PluginRow 内的删除入口

## Capabilities

### New Capabilities
- `mcp-explore`: MCP Server 目录浏览面板，包含内置注册表数据、搜索过滤、分类筛选和一键安装流程
- `mcp-add-modal`: 自定义 MCP Server 添加模态框，支持多 transport 类型的动态表单和环境变量编辑
- `mcp-detail-modal`: MCP Server 详情模态框，展示连接配置、工具列表和管理操作（Remove）

### Modified Capabilities
- `plugin-store`: 新增 `addPlugin(params)` 和 `removePlugin(id)` actions，扩展 store 状态管理
- `plugin-panel`: MCP Tab 增加 Installed/Explore 子视图切换和 Header Add 按钮

## Impact

- **前端组件**: 新增 3 个组件文件（McpExplorePanel、AddServerModal、McpDetailModal），修改 PluginsView.tsx
- **数据文件**: 新增 mcp-registry.json（静态数据，无需后端变更）
- **Transport API**: `addMcpServer` 签名从 `(id, command, args)` 扩展为 `(params)` 对象形式
- **后端**: 无变更需要 — `mcp.add` handler 已支持 transport/url 参数，`mcp.remove` 已实现
- **i18n**: plugins.json 翻译文件新增 Explore/Add/Detail 相关 key
- **依赖**: 无新依赖
