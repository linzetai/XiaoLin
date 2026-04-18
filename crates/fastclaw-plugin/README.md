# fastclaw-plugin

WASM 插件宿主，基于 wasmtime 运行沙箱化插件。

## 功能

- **wasmtime 宿主** — Core Module ABI，编译与实例化 `.wasm` 模块
- **燃料限制** — Fuel 机制限制执行时间，epoch 优雅退出
- **签名校验** — HMAC-SHA256 清单与二进制签名验证
- **热重载** — 文件系统监听插件目录变更，自动重新注册
- **插件注册表** — 统一管理已加载插件及其 capabilities
- **工具桥接** — 将插件 capability 注册为 Agent 可调用工具

## 关键导出

```rust
pub use host::WasmHost;
pub use registry::PluginRegistry;
pub use bridge::PluginTool;
pub use manifest::DiscoveredPlugin;
```
