## 1. P0 — 正确性修复

- [x] 1.1 修复注入用量计数：修改 `format_with_budget_ordered` 返回 `(String, Option<SkillTruncationInfo>, Vec<String>)` 包含实际注入的 skill IDs
- [x] 1.2 修改 `chat_pipeline.rs` 的 `inject_skills_prompt`，仅对返回的 IDs 调用 `record_injections`
- [x] 1.3 替换 `skill_embedding.rs` 中的 `DefaultHasher` 为 `blake3`（添加 `blake3` 依赖到 `xiaolin-core/Cargo.toml`）
- [x] 1.4 修复 extension 嵌套扫描：修改 `load_extension_skills` 额外扫描 `extensions/*/skills/` 子目录

## 2. P1 — Token 纪律

- [x] 2.1 将 `config.rs` 的 `default_context_budget_percent()` 从 5 改为 2
- [x] 2.2 更新 `impl Default for SkillsConfig` 对应字段
- [x] 2.3 运行 benchmark 验证 2% budget 下 agent 任务通过率无退化 — 107 skills/128K: 85% 保留率，7/7 断言通过

## 3. P2 — 搜索增强

- [x] 3.1 `SkillFrontmatter` 新增 `when_to_use: Option<String>` 字段（`skill.rs`）
- [x] 3.2 `compute_relevance` 新增 `when_to_use` 匹配权重 2.0（`builtin_tools/skill.rs`）
- [x] 3.3 Compact 格式输出中追加 `when: ...` 行（`skill.rs` format 函数）
- [x] 3.4 添加 `when_to_use` 相关单元测试

## 4. P3 — 安全审查

- [x] 4.1 定义 `UNSAFE_STRATEGY_PATTERNS` 常量（`ws/skills.rs`）
- [x] 4.2 `handle_evolution_promote` 中校验 strategy 内容，不安全时在响应中返回 `warning`
- [x] 4.3 YAML 解析失败时 `warn!` 记录文件路径和错误（`skill.rs` parse_frontmatter）

## 5. 验证

- [x] 5.1 `cargo clippy -- -D warnings` 零警告
- [x] 5.2 `cargo test --workspace --exclude xiaolin-app` 全通过（228 tests）
- [x] 5.3 运行 benchmark 对比基线 — 5% vs 2% delta 15pp，Compact ≥ Full 保留率，Lazy 100%
- [x] 5.4 启动 dev 实例回归测试（Tauri MCP）— Skills(107), Evolved Skills(3728), 搜索 UI 均正常
