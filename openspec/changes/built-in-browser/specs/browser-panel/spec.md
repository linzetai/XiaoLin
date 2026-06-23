## ADDED Requirements

### Requirement: Browser Tab 注册到 WorkspacePanel
系统 SHALL 在 WorkspacePanel 中注册一个 Browser Tab，使用浏览器图标，order 值使其排在 Files 和 Terminal 之后。Browser Tab 在至少有一个页面打开时自动激活。

#### Scenario: Tab 注册
- **WHEN** 应用启动
- **THEN** WorkspacePanel 的 tab 列表中包含 Browser tab

#### Scenario: 无页面时隐藏
- **WHEN** 没有任何 browser 页面打开
- **THEN** Browser tab 不显示在 tab 列表中

#### Scenario: 首次打开页面时自动激活
- **WHEN** 第一个 browser 页面被打开
- **THEN** WorkspacePanel 自动切换到 Browser tab 并展开面板

### Requirement: 地址栏
系统 SHALL 在 Browser Tab 顶部显示地址栏，包含后退、前进、刷新按钮和 URL 输入框。URL 输入框显示当前活跃页面的 URL，支持编辑和导航。

#### Scenario: 显示当前 URL
- **WHEN** 活跃页面的 URL 发生变化
- **THEN** 地址栏显示新的 URL

#### Scenario: 编辑并导航
- **WHEN** 用户点击地址栏，输入新 URL，按 Enter
- **THEN** 当前活跃页面导航到新 URL

#### Scenario: 安全指示器
- **WHEN** 页面使用 HTTPS
- **THEN** 地址栏显示锁定图标

#### Scenario: 非 URL 输入（Tier 1）
- **WHEN** 用户在地址栏输入非 URL 格式的文本并按 Enter
- **THEN** 不执行导航，保持当前页面不变

#### Scenario: 非 URL 输入（Tier 2 - Omnibox）
- **WHEN** 用户在地址栏输入非 URL 格式的文本并按 Enter
- **THEN** 使用用户配置的搜索引擎进行搜索（默认 Google）

### Requirement: 多页面标签栏
系统 SHALL 在地址栏下方显示页面标签栏，每个打开的页面显示为一个标签，包含页面标题和关闭按钮。支持点击切换、中键关闭。

#### Scenario: 显示页面标签
- **WHEN** 有多个页面打开
- **THEN** 每个页面显示为一个标签，包含标题和关闭按钮

#### Scenario: 切换页面
- **WHEN** 用户点击某个非活跃页面的标签
- **THEN** 切换到该页面，地址栏更新

#### Scenario: 关闭页面
- **WHEN** 用户点击标签的关闭按钮
- **THEN** 该页面关闭，WebView 销毁

#### Scenario: 新建页面
- **WHEN** 用户点击标签栏末尾的 "+" 按钮
- **THEN** 创建一个新的空白页面，聚焦地址栏

#### Scenario: 页面数量上限
- **WHEN** 已打开 8 个页面，用户尝试新建
- **THEN** 提示用户关闭一些页面后再试

### Requirement: 加载状态指示
系统 SHALL 在页面加载过程中显示加载指示器。

#### Scenario: 页面加载中
- **WHEN** 页面正在加载
- **THEN** 对应标签页显示旋转加载动画，地址栏刷新按钮变为停止按钮

#### Scenario: 页面加载完成
- **WHEN** 页面加载完成
- **THEN** 加载动画消失，刷新按钮恢复

### Requirement: Agent 控制状态指示
系统 SHALL 在 Agent 控制浏览器时显示明确的状态标记。

#### Scenario: Agent 控制中
- **WHEN** Agent 正在通过 browser 工具操作某个页面
- **THEN** 该页面标签显示 🤖 图标前缀，地址栏显示 "Agent 操作中" 状态条

#### Scenario: Agent 操作完成
- **WHEN** Agent 的 browser 工具调用完成
- **THEN** Agent 控制状态标记消失

### Requirement: 快捷键支持
系统 SHALL 支持常用浏览器快捷键。

#### Scenario: Ctrl+T 新建页面
- **WHEN** Browser Tab 激活时用户按 Ctrl+T
- **THEN** 创建新的空白页面

#### Scenario: Ctrl+W 关闭页面
- **WHEN** Browser Tab 激活时用户按 Ctrl+W
- **THEN** 关闭当前活跃页面

#### Scenario: Ctrl+L 聚焦地址栏
- **WHEN** Browser Tab 激活时用户按 Ctrl+L
- **THEN** 地址栏获得焦点且 URL 全选

### Requirement: 全宽布局模式
系统 SHALL 支持将 Browser 切换为 ContentBlock 主内容区域，Chat 收缩为左侧可折叠面板。

#### Scenario: 切换到全宽模式
- **WHEN** 用户点击全宽切换按钮（或按 Ctrl+Shift+F）
- **THEN** Browser 成为主内容区域（flex:1），Chat 收缩为左侧面板（280-500px 可拖拽）
- **AND** WorkspacePanel 的 Browser Tab 自动隐藏（因 Browser 已在全宽区域）
- **AND** WebView 不重建，仅通过 ResizeObserver 更新位置/尺寸

#### Scenario: 切换动画
- **WHEN** 模式切换触发
- **THEN** 先截取 WebView 快照显示为 `<img>`
- **AND** WebView 移到屏幕外
- **AND** 占位容器执行 CSS transition (~400ms)
- **AND** 动画完成后恢复 WebView 到新位置，移除快照 `<img>`

#### Scenario: Chat 面板拖拽
- **WHEN** 全宽模式下用户拖拽 Chat 面板右边缘
- **THEN** Chat 面板宽度在 280px-500px 之间调整
- **AND** Browser 区域同步缩放

#### Scenario: Chat 面板折叠
- **WHEN** 全宽模式下用户点击 Chat 面板的折叠按钮
- **THEN** Chat 面板收缩为 48px 窄条（Chat 图标 + 未读 badge）
- **AND** Browser 获得更多宽度

#### Scenario: Chat 面板展开
- **WHEN** 全宽模式下用户点击 48px 窄条
- **THEN** Chat 面板恢复到之前的宽度

#### Scenario: 从全宽模式返回
- **WHEN** 用户在全宽模式下点击退出全宽按钮（或按 Ctrl+Shift+F）
- **THEN** 恢复为 Panel 模式，Chat 恢复为主内容区域
- **AND** 所有 browser tabs 和状态保持不变

#### Scenario: 全宽模式下 Agent 新消息
- **WHEN** 全宽模式下 Chat 面板折叠时 Agent 发送消息
- **THEN** 48px 窄条的未读 badge 数字更新 + pulse 动画
- **AND** 不自动展开 Chat 面板（避免打扰用户浏览）

#### Scenario: 全宽模式下 WorkspacePanel
- **WHEN** 全宽模式下用户打开 WorkspacePanel
- **THEN** WorkspacePanel 正常显示（Files/Terminal/Review 等 Tab 可用）
- **AND** Browser Tab 不在 WorkspacePanel 中显示
- **AND** Panel 宽度从 Browser 的 flex:1 空间中扣除
