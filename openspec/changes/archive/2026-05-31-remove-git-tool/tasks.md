# Tasks: 移除 Git Tool

## Phase 1: 移除代码

- [x] `mod.rs`: 移除 `mod git;` 声明
- [x] `mod.rs`: 移除 `pub use git::GitTool;`
- [x] `mod.rs`: 移除 `registry.register(Arc::new(GitTool));`
- [x] 删除 `crates/xiaolin-agent/src/builtin_tools/git.rs`

## Phase 2: 清理关联 change

- [x] 删除 `openspec/changes/optimize-git-tool/` 目录（已废弃）

## Phase 3: 验证

- [x] `cargo check` 全 workspace 通过
- [x] `cargo test -p xiaolin-agent` 通过（1095 passed）
