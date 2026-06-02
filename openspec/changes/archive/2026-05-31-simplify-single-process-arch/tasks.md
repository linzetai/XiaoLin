# Tasks: 简化为单进程架构

## Phase 1: 移除 CLI/TUI crate

- [x] 从 `Cargo.toml` workspace members 中移除 `xiaolin-cli`
- [x] 删除 `crates/xiaolin-cli/` 目录
- [x] 清理其他 crate 对 `xiaolin-cli` 的依赖引用（如果有）
- [x] 确认 workspace 编译通过

## Phase 2: 移除 daemon 模式和 EmbedMode

- [x] `xiaolin-core/src/config.rs`: 移除 `EmbedMode` 枚举及其 `should_embed()` 方法
- [x] `xiaolin-core/src/config.rs`: 从 `GatewayConfig` 中移除 `embed` 字段
- [x] `xiaolin-core/src/config.rs`: 移除 `GatewayState` struct 及其 read/write/remove 方法
- [x] `xiaolin-core/src/config.rs`: 移除 gateway.json 相关路径常量
- [x] 确认 workspace 编译通过（与 Phase 3 合并验证）

## Phase 3: 简化 App 的 embedded.rs

- [x] 移除 `find_xiaolin_cli()` 函数
- [x] 移除 `which_in_path()` 函数
- [x] 移除 `start_daemon()` 方法
- [x] 移除 `wait_for_gateway()` 函数（daemon 专用）
- [x] 移除 `connect_existing()` 中的 `GatewayState` 依赖
- [x] 简化 `GatewayProcess::start()`: 直接启动 embedded，移除 daemon fallback 和 gateway.json 检测
- [x] 保留 `probe_gateway()` 用于端口冲突检测（但简化逻辑）
- [x] 确认 app 编译通过

## Phase 4: 简化构建脚本

- [x] `scripts/build-macos.sh`: 移除注入 `xiaolin` CLI 二进制的步骤
- [x] `tauri.conf.json`: 移除 sidecar/externalBin 相关配置（如果有）
- [x] 确认打包流程正常（脚本已简化，实际打包由用户验证）

## Phase 5: 清理 gateway crate 中的 daemon 启动逻辑

- [x] 检查 `xiaolin-gateway/src/lib.rs` 中是否有 daemon-specific 的入口（如 daemonize、PID file 写入等），移除之
- [x] 保留 `run_with_listener()` 作为 library 入口
- [x] 确认 gateway 仍可作为 lib 被 app 调用

## Phase 6: 验证

- [x] `cargo check` 全 workspace 通过
- [x] `cargo test` 关键模块通过（agent, gateway, sandbox, protocol）
- [x] 本地 `pnpm tauri dev` 能正常启动（已验证：gateway 启动、WS 连接、API 正常）
- [ ] 打包后 .app 能正常运行（需用户手动验证）
