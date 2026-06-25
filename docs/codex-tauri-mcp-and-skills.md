# Codex Tauri MCP and Skills

This project uses Codex-native configuration and skills. Do not depend on Cursor skill directories for Codex workflows.

## Tauri MCP

The Tauri MCP server is installed as the npm package published from `hypothesi/mcp-server-tauri`:

```text
@hypothesi/tauri-mcp-server@0.11.2
```

Codex loads it from the user-level config:

```toml
[mcp_servers.tauri]
command = "npx"
args = ["-y", "@hypothesi/tauri-mcp-server"]

[mcp_servers.tauri.env]
MCP_BRIDGE_HOST = "127.0.0.1"
MCP_BRIDGE_PORT = "9555"
```

The app side must expose the bridge before the MCP tools can inspect the WebView. In local development, `pnpm tauri dev` starts the app and the connector advertises the bridge ports.

## Codex Skill Locations

Codex skills used by this repository live in:

```text
.codex/skills/
```

User-level system skills live in:

```text
~/.codex/skills/
```

## Skills Used For Tauri Debugging

- `tauri-self-test`: start the Tauri app, connect the MCP bridge, and run end-to-end checks.
- `tauri-ui-debug`: inspect WebView DOM, screenshots, JavaScript execution, and UI failures through MCP.
- `tauri-app-develop`: day-to-day Tauri v2 development workflow and dev server checks.
- `tauri-ipc`: command and event wiring between frontend and Rust backend.
- `tauri-security`: capability and permission checks when IPC or plugin access fails.
- `tauri-titlebar-debug`: custom titlebar, drag region, and window control issues.

For OpenSpec work in this repository, the relevant Codex skills are:

- `openspec-explore`
- `openspec-propose`
- `openspec-apply-change`
- `openspec-archive-change`

