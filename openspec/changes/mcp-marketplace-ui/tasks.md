## 1. 数据层 — 注册表 + Transport API

- [ ] 1.1 创建 `src/data/mcp-registry.json`：~15 个热门 MCP Server 条目（filesystem, github, postgres, sqlite, brave-search, puppeteer, slack, memory, sequential-thinking, everything, fetch, google-maps, time, docker, git），每条包含 id/name/description/category/icon/transport/command/args/url/installHint/homepage
- [ ] 1.2 扩展 `transport.addMcpServer` 签名为对象参数形式 `(params: { id, command?, args?, transport?, url?, env? })`，确保 transport/url/env 透传到后端 `mcp.add`
- [ ] 1.3 在 `plugin-store.ts` 新增 `addPlugin(params)` 和 `removePlugin(id)` actions，调用 transport 层并刷新列表

## 2. Explore 面板

- [ ] 2.1 创建 `McpExplorePanel.tsx` 组件骨架：导入 registry 数据、搜索状态、分类筛选状态
- [ ] 2.2 实现搜索过滤逻辑：实时匹配 name/description（大小写不敏感）
- [ ] 2.3 实现分类筛选 pill 按钮（All / Development / Productivity / Data / Communication）
- [ ] 2.4 实现卡片列表渲染：icon 映射、name、description、category badge、installHint
- [ ] 2.5 实现 Install 按钮：调用 `addPlugin`、loading 状态、成功/失败反馈
- [ ] 2.6 实现已安装检测：与 plugin store 中的已安装列表比对，显示 "Installed" 标记

## 3. AddServerModal

- [ ] 3.1 创建 `AddServerModal.tsx` 组件骨架：模态框容器 + 关闭逻辑
- [ ] 3.2 实现 transport 类型选择器（Stdio / SSE / Streamable HTTP），切换动态表单
- [ ] 3.3 实现 Server ID 输入 + 行内验证（非空、不含 `__`、重复 ID 提示）
- [ ] 3.4 实现 Stdio 表单字段（Command 必填 + Args 可选）
- [ ] 3.5 实现 HTTP 表单字段（URL 必填）
- [ ] 3.6 实现环境变量键值对编辑器（+ Add Variable / 删除行）
- [ ] 3.7 实现 "Add & Connect" 提交：验证 → `addPlugin` → loading → 成功关闭/失败提示

## 4. McpDetailModal

- [ ] 4.1 创建 `McpDetailModal.tsx` 组件骨架：模态框容器 + 数据加载（调用 `mcp.detail`）
- [ ] 4.2 实现连接状态展示区（Status badge + connectedAt + toolCount）
- [ ] 4.3 实现配置预览区（Command/Args/URL/Transport + env 脱敏显示）
- [ ] 4.4 实现工具列表区（name + description，>5 个时提供搜索框）
- [ ] 4.5 实现 Remove 操作（确认提示 → `removePlugin` → 关闭）
- [ ] 4.6 实现 Restart 操作按钮

## 5. PluginsView 集成

- [ ] 5.1 MCP Tab 内部增加 Installed / Explore 子切换（复用 SegmentedControl 或 pill toggle）
- [ ] 5.2 Header 区域增加 "+ Add" 按钮（Installed 视图可见），点击打开 AddServerModal
- [ ] 5.3 PluginRow 增加 hover 时删除图标（触发确认 + removePlugin）
- [ ] 5.4 PluginRow 点击行为改为打开 McpDetailModal（替代或增强 inline expand）
- [ ] 5.5 空状态增加 "Browse MCP Servers" CTA 按钮，点击切换到 Explore

## 6. i18n + 验证

- [ ] 6.1 更新 `zh/plugins.json` 和 `en/plugins.json`：Explore、Add、Detail 相关文案
- [ ] 6.2 `npx tsc --noEmit` 编译通过
- [ ] 6.3 E2E 测试：浏览 Explore → 安装 → 验证出现在 Installed 列表
- [ ] 6.4 E2E 测试：打开 AddServerModal → 填写表单 → 添加成功
- [ ] 6.5 E2E 测试：打开 McpDetailModal → 查看工具 → Remove
