# fastclaw-gateway

基于 Axum 的 HTTP/WebSocket 网关，是 FastClaw 的 API 层和请求入口。

## 功能

- **REST API** — 聊天、Agent CRUD、会话、记忆、DAG、Cron、插件、工具等完整端点
- **WebSocket** — 多路复用聊天/Agent/会话，支持流式事件（含 `ask_question` 人机交互）
- **Webhook** — 渠道入站（`POST /webhook/:channel_id`）
- **配置热重载** — 文件监听 + SIGHUP 信号驱动，校验失败原子回滚
- **嵌入式运行** — `serve_with_state` 支持 Tauri 桌面应用进程内启动
- **chat_pipeline** — 聊天处理管线，串联路由、Agent 运行时、流式响应
- **记忆巩固** — `MemoryConsolidationHook` 在每轮对话后自动 LLM 摘要，提取事实并存储到长期记忆

## 关键导出

```rust
pub use state::AppState;
pub fn build_app(state: AppState) -> Router;
pub async fn run(config: FastClawConfig) -> Result<()>;
pub async fn serve_with_state(state: AppState, listener: TcpListener) -> Result<()>;
```

## Feature Flags

- `test-helpers` — 测试辅助工具
