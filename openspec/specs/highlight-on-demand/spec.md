## ADDED Requirements

### Requirement: highlight.js SHALL only register a predefined subset of languages
`rehype-highlight` 的配置 SHALL 通过 `languages` 选项只注册以下语言：javascript, typescript, python, rust, bash, json, css, html, xml, sql, go, java, c, cpp, yaml, toml, markdown, diff（共 18 种）。

#### Scenario: Common language code block
- **WHEN** markdown 内容包含 ` ```python\nprint("hello")\n``` `
- **THEN** 代码块正确应用 Python 语法高亮

#### Scenario: Uncommon language code block
- **WHEN** markdown 内容包含 ` ```haskell\nmain = putStrLn "hello"\n``` `
- **THEN** 代码块以等宽字体显示纯文本，无语法高亮
- **THEN** 不产生 console error 或渲染异常

#### Scenario: No language hint
- **WHEN** markdown 内容包含无语言标注的代码块 ` ```\nsome code\n``` `
- **THEN** 代码块以等宽字体显示，rehype-highlight 尝试自动检测但不保证高亮

### Requirement: Language imports SHALL use direct module paths
每种语言 SHALL 通过直接路径 `highlight.js/lib/languages/<lang>` 导入，禁止使用 barrel import `highlight.js/lib/common` 或 `highlight.js` 的默认导出。

#### Scenario: Import path format
- **WHEN** 构建系统处理 highlight 相关 import
- **THEN** 使用的 import 路径形如 `import javascript from "highlight.js/lib/languages/javascript"`
- **THEN** tree-shaking 可排除未导入的语言模块

### Requirement: rehype-highlight configuration SHALL be module-level constant
`rehypePlugins` 数组配置（含 `languages` 选项）SHALL 定义为模块级常量，避免每次组件渲染创建新对象引用。

#### Scenario: Plugin array stability
- **WHEN** `MarkdownContent` 组件连续多次渲染
- **THEN** 传入 `<Markdown rehypePlugins={...}>` 的 rehypePlugins 引用不变
- **THEN** react-markdown 内部不因 plugins 引用变化触发重新配置

### Requirement: Vite manualChunks SHALL separate highlight languages
`vite.config.ts` 的 `manualChunks` 配置 SHALL 将 highlight.js 语言文件打包为独立 chunk，与 react-markdown 核心分离。

#### Scenario: Build output chunks
- **WHEN** 执行 `pnpm build`
- **THEN** highlight.js 相关模块打包在 `highlight-langs` chunk 中
- **THEN** 该 chunk 的压缩体积不超过 40KB
