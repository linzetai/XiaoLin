## 1. Phase 1: 修 Bug + 激活死代码（cross-tool-skills）

- [ ] 1.1 修复 upload_skill 路径：`commands/skill.rs` 写入从 `state_dir/config/skills/` 改为 `state_dir/skills/`（对齐 `resolve_global_skills_dir`）
- [ ] 1.1b 添加一次性迁移：将 `config/skills/` 下已有 skill 移动到 `skills/`
- [ ] 1.2 统一 skill 工具注册策略：确认使用 `UnifiedSkillTool`（统一 `skill` 工具 + action 分发），在其中增加 `search` action（复用 `SearchSkillTool` 逻辑），移除 `#[allow(dead_code)]`
- [ ] 1.3 修改 `builder.rs:418-443`：去掉 prompt_mode 条件限制，所有模式（full/compact/lazy）都注册 `UnifiedSkillTool`
- [ ] 1.4 默认 prompt_mode 改为 Compact：修改 `config.rs` 的 `default_prompt_mode()` 和 `impl Default for SkillsConfig`
- [ ] 1.5a 设计 write_skill → reload 回调：在 `UnifiedSkillTool` 构造时注入 `Arc<dyn Fn() -> Result<()> + Send + Sync>` callback，由 gateway builder 提供 `reload_skills()` 实现
- [ ] 1.5b `UnifiedSkillTool` write action 成功后调用 reload callback
- [ ] 1.6a 统一 `reload_skills()` 与初始化 parity：修改 `state/mod.rs` 的 `reload_skills()`，添加 `register_builtin_skills()` 和 `ext_registry` merge，确保 builtin + ext + legacy + cross-tool 全部保留
- [ ] 1.6b 实现 extension skills 加载：扫描 `resolve_extensions_dir()` 目录，加载 skill 到 `ext_registry`
- [ ] 1.7 清理 `SKILL_AUTHORING_PROMPT`：移除关于 semantic search、usage tracking 等未实现功能的虚假描述
- [ ] 1.8 运行 `cargo test --workspace --exclude xiaolin-app` 验证所有变更
- [ ] 1.9 运行 benchmark 验证 compact 模式不降低任务通过率

## 2. Phase 2: Context Budget 机制（skill-context-budget）

- [x] 2.0 在 `chat_pipeline.rs` 的 `inject_skills_prompt` 获取当前 model 的 `context_window` 参数并传入 budget 函数
- [x] 2.1 在 `SkillsConfig` 新增 `context_budget_percent: u8` 字段（默认 5）
- [x] 2.2 在 `SkillRegistry::format_with_budget` 中实现渐进截断逻辑：先缩短 description → 再 omit 低优先级 skill（对齐 Codex 策略）
- [x] 2.3 截断 warning 发送到 tracing（非追加到 system prompt）
- [x] 2.4 `context_budget_percent = 0` 时禁用预算限制
- [x] 2.5 添加单元测试覆盖：正常、description 截断、skill omit、禁用四种场景

## 3. Phase 2: 前端 Skill 管理面板（skill-management-ui）

- [x] 3.0 在 `xiaolin-protocol/src/op.rs` 新增 `SkillsRead`/`SkillsUpdate`/`SkillsDelete` op 类型和 params
- [x] 3.1 在 `ws/skills.rs` 实现 `skills.read`/`skills.update`/`skills.delete` handler；扩展 `skills.list` 返回 source/layer/enabled 字段
- [x] 3.2 `skills.update`/`skills.delete` 限制为 XiaoLin-owned skills（Cursor/Codex/Extension 返回 403）
- [x] 3.3 扩展前端 `SkillInfo`/`SkillDetail` 类型和 transport/api.ts（新增 source/layer/enabled 字段）
- [x] 3.4 前端：实现 enable/disable toggle，复用已有 `getSkillsDenyList`/`updateSkillsDenyList` API + deny 变更后触发 refresh
- [x] 3.5 前端：创建 `SkillDetailModal` 组件，展示全文 + frontmatter tags/tools + 编辑/删除功能
- [x] 3.6 前端：实现搜索框和源筛选器
- [x] 3.7 前端：集成到 Plugins → Skills tab，替换现有简单列表

## 4. Phase 2: Frontmatter tools 限制生效

- [x] 4.0 设计 skill tool restriction：通过 `/skill` slash command 激活时提取 `frontmatter.tools`
- [x] 4.1 在 `setup_chat` 中将 skill tools 应用到 `agent_config.behavior.tools_allow`（intersect 现有 allow list）
- [x] 4.2 添加测试：frontmatter.tools 解析（非空/空/缺失）+ BehaviorConfig tools_allow 行为验证

## 5. Phase 3: 条件激活（skill-conditional-activation）

- [x] 5.1 在 `SkillFrontmatter` 新增 `paths: Vec<String>` 字段
- [x] 5.2 初始化时分离 conditional（有 `paths:`）和 unconditional（无 `paths:`）skill
- [x] 5.3 实现 touch-triggered 激活：tool 操作文件后检查路径是否匹配 conditional skill 的 `paths:` glob（使用 `globset` crate，gitignore 风格相对路径）
- [x] 5.4 Cargo.toml 添加 `globset` 依赖到 `xiaolin-core`
- [x] 5.5 `paths: []` 或 `paths: ["**"]` 视为 unconditional（always-on）
- [x] 5.6 已被 deny list 禁用的 skill 不受 paths 匹配影响（deny 优先于 paths）
- [x] 5.7 添加测试覆盖匹配/不匹配/无 paths/deny 优先四种场景

## 6. Phase 3: Skill Marketplace（skill-marketplace）

- [ ] 6.0 决策：复用已有 `HubClient`（`xiaolin-core/hub.rs`）还是新建 GitHub raw fetch——基于可用性选择
- [ ] 6.1 定义 marketplace 目录索引格式（JSON schema：id, name, description, author, repo_url, skill_path, tags, version）
- [ ] 6.2 后端：实现 marketplace index 获取（GitHub raw URL fetch + 本地缓存 1h TTL）
- [ ] 6.3 后端：实现 skill 安装——下载**整个 skill 目录**（SKILL.md + scripts/ + 引用文件）→ 写入 `~/.xiaolin/skills/` → hot-reload
- [ ] 6.4 后端：实现 skill 卸载（删除目录 → hot-reload）
- [ ] 6.5 前端：创建 `SkillMarketplace` 组件（浏览、搜索、分类）
- [ ] 6.6 前端：实现安装/卸载/更新按钮和确认流程；网络失败显示缓存数据
- [ ] 6.7 前端：skill 预览面板（完整内容 + 安装按钮）

## 7. Phase 3: Skillify 元技能（skillify-meta-skill）

- [ ] 7.1 创建 skillify prompt 模板：从会话上下文提取可复用模式生成 SKILL.md
- [ ] 7.2 注册 `/skillify` slash command（与 `/skills` 并列），路由到 skillify workflow
- [ ] 7.3 生成 skill 后展示预览，用户确认后保存——调用 `skill` tool `action: write, target: project` → `<workspace>/.xiaolin/skills/`
- [ ] 7.4 保存后自动 hot-reload registry

## 8. Phase 3: MCP Skill 资源（skill-mcp-resource）

- [ ] 8.1 在 MCP 资源发现逻辑中识别 `skill://` URI scheme
- [ ] 8.2 Fetch skill 资源内容并解析为 SkillEntry（Extension layer），ID 命名为 `mcp__<server_id>__<resource_suffix>` 避免冲突
- [ ] 8.3 MCP 服务器重连时刷新 skill 资源
- [ ] 8.4 MCP skill 设为只读（update/delete 返回错误）
- [ ] 8.5 将 MCP skills 接入 `ext_registry` 或 per-agent registry 的 merge 链

## 9. Phase 4: 语义搜索（skill-semantic-search）

- [ ] 9.1 扩展 `xiaolin-memory::EmbeddingProvider` trait，新增 skill embedding 表（复用 hypembed，不引入 fastembed-rs）
- [ ] 9.2 skill 加载时生成 embedding 并存入 SQLite（content hash 校验避免重复计算）
- [ ] 9.3 `search` action 支持语义搜索模式（cosine similarity 排序）；embedding 不可用时 fallback 到关键词匹配
- [ ] 9.4 实现 embedding 缓存失效（skill 内容变更时重新生成）
- [ ] 9.5 添加测试覆盖语义搜索精度、fallback、缓存失效

## 10. Phase 4: 用量追踪（skill-usage-tracking）

- [ ] 10.1 创建 `skill_usage` SQLite 表 migration（id, skill_id, event_type, session_id, timestamp）——注意与 `xiaolin-evolution` 的 `skill_usages` 隔离
- [ ] 10.2 追踪 `read` 和 full body injection 事件（**不**追踪每轮 compact metadata 注入，避免数据爆炸）
- [ ] 10.3 `skills.list` WS API 返回 `usage_count` 字段（last 30 days）
- [ ] 10.4 prompt 注入时同 layer 内按 usage_count 降序排列（与 context budget 截断协同：先按 layer，再按 usage）
- [ ] 10.5 实现 90 天数据自动清理

## 11. Phase 4: Evolution 统一视图

- [ ] 11.1 前端合并展示静态 skill + evolution candidate/active skill
- [ ] 11.2 实现 promote（evolution → `~/.xiaolin/skills/` 静态 skill）
- [ ] 11.3 UI 展示 evolution skill 的来源、匹配会话数、置信度

## 12. 验证

- [ ] 12.1 Phase 1 完成后运行 `cargo test --workspace --exclude xiaolin-app`
- [ ] 12.2 Phase 1 完成后运行 benchmark 对比 compact vs full 模式通过率
- [ ] 12.3 Phase 2 完成后 E2E 验证 skill 管理面板 CRUD（Tauri MCP 真实 UI 操作）
- [ ] 12.4 全部 Phase 完成后运行完整 benchmark 对比基线
