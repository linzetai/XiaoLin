## ADDED Requirements

### Requirement: CodeMirror 6 代码查看器

Files tab 中的代码文件 SHALL 使用 CodeMirror 6 渲染，默认 readonly 模式。支持语法高亮、行号、代码折叠、文本搜索、虚拟滚动。

#### Scenario: 打开代码文件
- **WHEN** 用户在 Files tab 中打开一个代码文件（如 `.rs`、`.ts`、`.py`）
- **THEN** 使用 CodeMirror 6 EditorView 渲染文件内容
- **AND** 自动根据文件扩展名加载对应语言包进行语法高亮
- **AND** 左侧显示行号 gutter
- **AND** 编辑器处于 readonly 模式，用户无法修改内容

#### Scenario: 语言包按需加载
- **WHEN** 打开一个尚未加载语言包的文件类型
- **THEN** 通过动态 `import()` 加载对应的 `@codemirror/lang-*` 包
- **AND** 加载期间显示纯文本（无高亮）
- **AND** 加载完成后自动应用语法高亮（无需用户操作）

#### Scenario: 未知语言文件
- **WHEN** 打开扩展名未在语言映射表中的文件
- **THEN** 以纯文本模式渲染（无语法高亮）
- **AND** 行号和搜索功能仍可用

#### Scenario: 二进制文件
- **WHEN** 打开一个二进制文件（`.wasm`、`.zip`、`.exe`、`.sqlite` 等非文本文件未匹配到 ImageViewer 的）
- **THEN** 显示"无法预览二进制文件"提示
- **AND** 显示文件大小和类型信息
- **AND** 提供"在外部应用打开"按钮

#### Scenario: 超大文本文件
- **WHEN** 打开一个文本文件且大小超过 5MB
- **THEN** 显示"文件过大（X MB），无法在内置查看器中打开"提示
- **AND** 提供"在外部编辑器打开"按钮

#### Scenario: 文件内容自动刷新
- **WHEN** Agent 修改了一个已在查看器中打开的文件（通过 `file_artifact` WS event 检测）
- **THEN** 查看器上方显示"文件已被修改"横幅通知
- **AND** 横幅包含"重新加载"按钮
- **AND** 点击"重新加载"重新读取文件并更新查看器内容（保留滚动位置）

### Requirement: 代码折叠

CodeMirror 查看器 SHALL 支持代码折叠（基于语言语法的缩进/块结构）。

#### Scenario: 折叠代码块
- **WHEN** 用户点击行号 gutter 中的折叠图标（在函数/类/块定义行）
- **THEN** 该代码块折叠为单行，显示 `...` 占位符
- **AND** 再次点击恢复展开

#### Scenario: 全部折叠/展开
- **WHEN** 用户通过工具栏触发"全部折叠"操作
- **THEN** 所有可折叠的顶层代码块折叠
- **AND** "全部展开"操作恢复所有折叠块

### Requirement: 文本搜索

CodeMirror 查看器 SHALL 内建文本搜索功能。

#### Scenario: 触发搜索
- **WHEN** 用户按下 `Ctrl+F`（或 `Cmd+F`）
- **THEN** 在查看器顶部显示搜索输入框
- **AND** 输入关键词时实时高亮所有匹配项
- **AND** 显示匹配数量（如 "3/12"）

#### Scenario: 搜索导航
- **WHEN** 搜索有匹配结果
- **THEN** 按 Enter 或点击"下一个"跳转到下一个匹配项
- **AND** 按 Shift+Enter 或点击"上一个"跳转到上一个匹配项

### Requirement: 跳转到指定行

CodeMirror 查看器 SHALL 支持跳转到指定行号。

#### Scenario: 从 chat 链接跳转
- **WHEN** `xiaolin:open-file` 事件携带 `line` 参数
- **THEN** 查看器打开文件后自动滚动到指定行
- **AND** 指定行使用高亮背景标记（持续 3 秒后淡出）

### Requirement: 主题跟随

CodeMirror 查看器 SHALL 跟随 XiaoLin 的 light/dark 主题切换。

#### Scenario: 主题切换
- **WHEN** 用户在设置中切换 light/dark 主题
- **THEN** CodeMirror 查看器的背景色、文字色、语法高亮色自动更新
- **AND** 与 XiaoLin 整体 UI 的色调保持一致

### Requirement: lazy import

CodeMirror 6 所有代码 SHALL 通过 `React.lazy()` + `Suspense` 加载，不影响应用启动速度。

#### Scenario: 首次打开 Files tab
- **WHEN** 用户首次切换到 Files tab
- **THEN** 显示 loading skeleton
- **AND** 异步加载 CM6 核心包
- **AND** 加载完成后渲染查看器（无闪烁）

#### Scenario: 后续打开
- **WHEN** CM6 已加载过
- **THEN** 切换到 Files tab 时立即渲染，无 loading 状态

### Requirement: 工具栏

代码查看器上方 SHALL 显示工具栏，包含文件名、操作按钮。

#### Scenario: 工具栏显示
- **WHEN** 查看器中有打开的文件
- **THEN** 工具栏显示：文件路径（mono 字体）、复制内容按钮、在外部编辑器打开按钮、自动换行切换
- **AND** 复制按钮点击后短暂显示 ✓ 反馈

#### Scenario: 在外部编辑器打开
- **WHEN** 用户点击"在外部编辑器打开"按钮
- **THEN** 通过 Tauri `shell:allow-open` 用系统默认应用打开该文件
