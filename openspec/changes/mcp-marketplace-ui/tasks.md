## 1. 数据层 — 注册表 + Transport API

- [x] 1.1 创建 `src/data/mcp-registry.json`：15 个热门 MCP Server 条目
- [x] 1.2 扩展 `transport.addMcpServer` 签名为对象参数形式
- [x] 1.3 在 `plugin-store.ts` 新增 `addPlugin(params)` 和 `removePlugin(id)` actions

## 2. Explore 面板

- [x] 2.1 创建 `McpExplorePanel.tsx` 组件骨架
- [x] 2.2 实现搜索过滤逻辑：实时匹配 name/description
- [x] 2.3 实现分类筛选 pill 按钮
- [x] 2.4 实现卡片列表渲染
- [x] 2.5 实现 Install 按钮
- [x] 2.6 实现已安装检测

## 3. AddServerModal

- [x] 3.1 创建 `AddServerModal.tsx` 组件骨架
- [x] 3.2 实现 transport 类型选择器
- [x] 3.3 实现 Server ID 输入 + 行内验证
- [x] 3.4 实现 Stdio 表单字段
- [x] 3.5 实现 HTTP 表单字段
- [x] 3.6 实现环境变量键值对编辑器
- [x] 3.7 实现 "Add & Connect" 提交

## 4. McpDetailModal

- [x] 4.1 创建 `McpDetailModal.tsx` 组件骨架
- [x] 4.2 实现连接状态展示区
- [x] 4.3 实现配置预览区
- [x] 4.4 实现工具列表区
- [x] 4.5 实现 Remove 操作
- [x] 4.6 实现 Restart 操作按钮

## 5. PluginsView 集成

- [x] 5.1 MCP Tab 内部增加 Installed / Explore 子切换
- [x] 5.2 Header 区域增加 "+ Add" 按钮
- [x] 5.3 PluginRow 增加 hover 时删除图标
- [x] 5.4 PluginRow 点击行为改为打开 McpDetailModal
- [x] 5.5 空状态增加 "Browse MCP Servers" CTA 按钮

## 6. i18n + 验证

- [x] 6.1 更新 `zh/plugins.json` 和 `en/plugins.json`
- [ ] 6.2 `npx tsc --noEmit` 编译通过
- [ ] 6.3 E2E 测试：浏览 Explore → 安装 → 验证出现在 Installed 列表
- [ ] 6.4 E2E 测试：打开 AddServerModal → 填写表单 → 添加成功
- [ ] 6.5 E2E 测试：打开 McpDetailModal → 查看工具 → Remove
