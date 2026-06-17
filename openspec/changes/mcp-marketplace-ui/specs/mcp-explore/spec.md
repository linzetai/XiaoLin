## ADDED Requirements

### Requirement: Explore panel renders registry entries
MCP Tab SHALL 提供 Explore 子视图，从内置 `mcp-registry.json` 加载注册表条目并以卡片列表形式展示。

#### Scenario: Explore view loads registry
- **WHEN** 用户切换到 MCP Tab 的 Explore 子视图
- **THEN** 从 `mcp-registry.json` 加载全部条目
- **AND** 每张卡片显示 icon、name、description、category badge

#### Scenario: Registry entry card layout
- **WHEN** 注册表包含一条 id 为 "filesystem" 的条目
- **THEN** 卡片显示 name "File System"、description、category badge "development"
- **AND** 卡片右侧显示 "Install" 按钮（若未安装）或 "Installed" 禁用标记（若已安装）

### Requirement: Search filter
Explore 面板 SHALL 提供搜索输入框，实时过滤注册表条目（匹配 name 或 description）。

#### Scenario: Search by name
- **WHEN** 用户在搜索框输入 "git"
- **THEN** 列表仅显示 name 或 description 中包含 "git"（大小写不敏感）的条目

#### Scenario: Empty search shows all
- **WHEN** 搜索框为空
- **THEN** 显示所有注册表条目（受分类筛选约束）

### Requirement: Category filter
Explore 面板 SHALL 提供分类筛选 pill 按钮（All / Development / Productivity / Data / Communication）。

#### Scenario: Filter by category
- **WHEN** 用户选择 "Development" 分类
- **THEN** 列表仅显示 category 为 "development" 的条目

#### Scenario: All category shows everything
- **WHEN** 用户选择 "All" 分类
- **THEN** 显示所有注册表条目（受搜索过滤约束）

#### Scenario: Search and category combined
- **WHEN** 用户选择 "Data" 分类且搜索框输入 "sql"
- **THEN** 列表仅显示 category 为 "data" 且 name/description 包含 "sql" 的条目

### Requirement: One-click install from registry
Explore 面板中每张卡片 SHALL 提供 "Install" 按钮，点击后调用 `addPlugin` 将注册表条目中的配置写入用户配置并连接。

#### Scenario: Install stdio server
- **WHEN** 用户点击 filesystem server 卡片的 "Install" 按钮
- **THEN** 调用 `mcp.add` API，传入注册表中的 id、command、args、transport
- **AND** 按钮变为 loading 状态
- **AND** 成功后卡片显示 "Installed" 标记，Installed 列表同步更新

#### Scenario: Install HTTP server
- **WHEN** 用户点击某 streamable_http server 的 "Install" 按钮
- **THEN** 调用 `mcp.add` API，传入 id、transport "streamable_http"、url
- **AND** 成功后该 server 出现在 Installed 列表

#### Scenario: Install failure
- **WHEN** 安装请求失败（如 command 不存在）
- **THEN** 卡片显示错误提示
- **AND** "Install" 按钮恢复为可点击状态

### Requirement: Install hint display
当注册表条目包含 `installHint` 字段时，卡片 SHALL 显示安装前置条件提示文案。

#### Scenario: Show install hint
- **WHEN** 注册表条目 installHint 为 "需先安装: npm i -g @modelcontextprotocol/server-filesystem"
- **THEN** 卡片底部显示该提示文案（灰色小字）

### Requirement: Already installed detection
Explore 面板 SHALL 将注册表条目与当前已安装 plugin 列表比对，已安装条目显示 "Installed" 而非 "Install"。

#### Scenario: Installed server shows badge
- **WHEN** 注册表中 id 为 "github" 的条目，且已安装列表中存在 id "github"
- **THEN** 该卡片显示 "Installed" 禁用标记，不显示 "Install" 按钮
