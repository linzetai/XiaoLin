## ADDED Requirements

### Requirement: Markdown frontmatter defines agent type

系统 SHALL 支持通过 Markdown 文件的 YAML frontmatter 定义自定义 agent 类型。

#### Scenario: Valid agent definition
- **GIVEN** 文件 `.xiaolin/agents/code-reviewer.md` 内容为:
  ```
  ---
  name: code-reviewer
  description: Reviews code for bugs and style
  tools: [read_file, grep, list_files]
  maxTurns: 10
  background: true
  concurrencySafe: true
  ---
  You are a code review specialist...
  ```
- **WHEN** 系统加载 agent definitions
- **THEN** 注册 SubAgentDef { id: "code-reviewer", name: "code-reviewer", description: "Reviews code...", tools: allowed=[read_file, grep, list_files], max_turns: 10, background: true, concurrency_safe: true, system_prompt: "You are a code review specialist..." }

#### Scenario: Minimal definition
- **GIVEN** frontmatter 只有 `name` 和 `description`
- **WHEN** 加载
- **THEN** 其他字段使用默认值: tools=全部, maxTurns=20, background=false, concurrencySafe=false, permissionMode=autoApprove

### Requirement: Markdown body appended to system prompt

Markdown body（frontmatter 之后的内容）SHALL 作为 system prompt 的一部分。

#### Scenario: Frontmatter has systemPrompt + body
- **GIVEN** frontmatter 中有 `systemPrompt: "Base prompt."` 且 body 有额外内容
- **WHEN** 构建子 agent system prompt
- **THEN** 最终 prompt = frontmatter.systemPrompt + "\n\n" + markdown_body

#### Scenario: No systemPrompt in frontmatter, body exists
- **GIVEN** frontmatter 中无 systemPrompt 字段，但 body 有内容
- **WHEN** 构建子 agent system prompt
- **THEN** 最终 prompt = markdown_body

#### Scenario: Neither systemPrompt nor body
- **GIVEN** frontmatter 中无 systemPrompt，body 为空
- **WHEN** 构建子 agent system prompt
- **THEN** 使用父级 agent 的 system prompt（继承）

### Requirement: Agent definition load paths and priority

系统 SHALL 从多个路径加载 agent definitions，按优先级合并。

#### Scenario: Load order
- **WHEN** 系统启动或 reload
- **THEN** 加载顺序为: Builtin → `~/.xiaolin/agents/*.md` → `{project_root}/.xiaolin/agents/*.md`
- **AND** 同 id 的后者覆盖前者

#### Scenario: Project agent overrides builtin
- **GIVEN** builtin 有 id="explore" 的定义，项目目录有 `.xiaolin/agents/explore.md`
- **WHEN** 加载完成
- **THEN** 项目的 explore 定义覆盖 builtin

#### Scenario: Invalid markdown skipped with warning
- **GIVEN** `.xiaolin/agents/broken.md` 的 frontmatter YAML 格式错误
- **WHEN** 加载
- **THEN** 该文件被跳过，记录 warning 日志，不影响其他 agent 加载

### Requirement: Frontmatter schema validation

Frontmatter 中的字段 SHALL 经过类型验证。

#### Scenario: Required fields
- **GIVEN** frontmatter 缺少 `name` 字段
- **WHEN** 解析
- **THEN** 该文件被跳过，记录 error: "agent definition missing required field: name"

#### Scenario: Schema fields
- **THEN** 支持的字段:
  - `name` (string, required): agent 类型 ID
  - `description` (string, optional): 描述
  - `tools` (string[] or "all", optional): 允许的工具列表
  - `disallowedTools` (string[], optional): 禁止的工具列表
  - `model` (string or "inherit", optional): 模型覆盖
  - `maxTurns` (positive int, optional, default 20): 最大轮次
  - `background` (bool, optional, default false): 是否 async
  - `concurrencySafe` (bool, optional, default false): 是否可并行
  - `permissionMode` (enum, optional, default "autoApprove"): 权限模式
  - `systemPrompt` (string, optional): system prompt 前缀
  - `mode` (enum: "normal" | "coordinator", optional, default "normal"): 运行模式

### Requirement: Hot-reload on file change

Agent definition 文件变更时 SHALL 支持热重载。

#### Scenario: File modified
- **GIVEN** `.xiaolin/agents/reviewer.md` 被修改
- **WHEN** 文件系统 watcher 检测到变更
- **THEN** 重新加载该文件，更新 SubAgentManager 中对应的 SubAgentDef
- **AND** 已运行的子 agent 不受影响（使用旧定义直到完成）
