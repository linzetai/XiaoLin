## ADDED Requirements

### Requirement: Shell Integration script injection
TerminalManager SHALL 在 PTY 启动时自动注入 Shell Integration 脚本，为 bash 和 zsh 提供 OSC 133 命令边界标记。

#### Scenario: Bash Shell Integration injection
- **WHEN** PTY spawn 使用 bash shell
- **THEN** 通过 `--init-file` 参数加载 Shell Integration 脚本
- **AND** 脚本设置 `PROMPT_COMMAND` 输出 `\e]133;A\a`（prompt start）
- **AND** 使用 `trap ... DEBUG` 在命令执行前输出 `\e]133;C\a`（command start）
- **AND** 在 `PROMPT_COMMAND` 中输出 `\e]133;D;$?\a`（command end + exit code）

#### Scenario: Zsh Shell Integration injection
- **WHEN** PTY spawn 使用 zsh shell
- **THEN** 通过 `ZDOTDIR` 机制加载 Shell Integration 脚本
- **AND** 使用 `precmd` hook 输出 OSC 133;A 和 OSC 133;D
- **AND** 使用 `preexec` hook 输出 OSC 133;C

#### Scenario: Unknown shell fallback
- **WHEN** PTY spawn 使用非 bash/zsh 的 shell（如 fish、dash）
- **THEN** 不注入 Shell Integration 脚本
- **AND** Agent 的 PTY 命令完成检测回退到超时模式

### Requirement: OSC 133 parsing
PTY 桥接层 SHALL 解析 PTY 输出流中的 OSC 133 序列，提取命令边界信息。

#### Scenario: Detect command end
- **WHEN** PTY 输出包含 `\e]133;D;0\a`
- **THEN** 解析器识别命令结束，exit_code = 0
- **AND** 通知 Agent 桥接层命令已完成

#### Scenario: Detect command end with error
- **WHEN** PTY 输出包含 `\e]133;D;1\a`
- **THEN** 解析器识别命令结束，exit_code = 1

#### Scenario: OSC sequences transparent to xterm.js
- **WHEN** PTY 输出包含 OSC 133 序列
- **THEN** 序列原样传递给 xterm.js（xterm.js 原生支持 OSC 133 装饰）
- **AND** 不被 TerminalManager 过滤或修改

### Requirement: Shell Integration script management
Shell Integration 脚本 SHALL 作为嵌入资源打包在应用中。

#### Scenario: Script bundled as resource
- **WHEN** 应用构建
- **THEN** bash 和 zsh 的 Shell Integration 脚本打包在 Tauri 资源目录中
- **AND** TerminalManager 在 spawn 时从资源目录读取脚本路径

#### Scenario: Script does not interfere with user config
- **WHEN** Shell Integration 脚本被加载
- **THEN** 脚本在用户的 `.bashrc`/`.zshrc` 之后执行（不覆盖用户配置）
- **AND** 如果用户已有 Shell Integration（如 VSCode 的），不重复注入（检测 `__XIAOLIN_SHELL_INTEGRATION` 环境变量）
