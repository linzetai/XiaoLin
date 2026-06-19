## ADDED Requirements

### Requirement: Skill directory filesystem watcher
系统 SHALL 通过 `notify` crate 监控所有已配置的 skill 目录（project `.xiaolin/skills/`、global `~/.xiaolin/skills/`、extension skill 目录），在文件变更时自动触发 `reload_skills()`。

#### Scenario: 用户直接编辑 SKILL.md 后自动生效
- **WHEN** 用户在编辑器中修改 `.xiaolin/skills/my-skill/SKILL.md` 并保存
- **THEN** 系统 MUST 在 300ms debounce 窗口后自动重新加载所有 skill 注册表
- **THEN** 后续对话中使用的 skill 内容为最新版本

#### Scenario: 新增 skill 目录自动发现
- **WHEN** 用户在 `.xiaolin/skills/` 下创建新的 `new-skill/SKILL.md`
- **THEN** 系统 MUST 在 debounce 窗口后自动将新 skill 注册到 registry
- **THEN** 新 skill 可通过 `list_skills` 和 `read_skill` 工具访问

#### Scenario: 删除 skill 目录自动移除
- **WHEN** 用户删除 `.xiaolin/skills/my-skill/` 目录
- **THEN** 系统 MUST 在 debounce 窗口后从 registry 移除该 skill
- **THEN** 相关 embedding 缓存 SHOULD 在下次 embedding 更新时被 prune

### Requirement: Watcher 与 embedding 更新联动
当 watcher 触发 skill 重载后，系统 SHALL 同步触发 embedding 更新（`spawn_skill_embedding_update`），确保语义搜索索引与磁盘状态一致。

#### Scenario: 修改 skill 内容后语义搜索更新
- **WHEN** 用户修改 skill 内容后 watcher 触发重载
- **THEN** 系统 MUST 重新计算被修改 skill 的 content_hash
- **THEN** 若 hash 变化，MUST 重新生成该 skill 的 embedding 向量

### Requirement: Watcher 错误不影响主流程
文件系统 watcher 的创建或运行时错误 SHALL NOT 导致 gateway 启动失败或崩溃。

#### Scenario: 目录不存在时优雅降级
- **WHEN** 配置的 skill 目录不存在
- **THEN** 系统 MUST 记录 `warn!` 日志并跳过该目录的 watch
- **THEN** 其他可用目录的 watch 正常运行

#### Scenario: watcher 运行时错误
- **WHEN** 文件系统通知出现错误（如权限问题）
- **THEN** 系统 MUST 记录 `warn!` 日志
- **THEN** 不影响已加载的 skill 的使用
