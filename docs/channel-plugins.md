# XiaoLin Channel Plugin System

XiaoLin supports adding messaging channels (Feishu, Slack, Discord, etc.) via external process plugins. No recompilation is needed — just add a JSON config file.

## Overview

```
┌─────────────────────────────────────────────────────────────┐
│                   XiaoLin Gateway                           │
│  ┌───────────────────────────────────────────────────────┐  │
│  │               ChannelRegistry                          │  │  │
│  │  ProcessChannelPlugin ── JSON-RPC ──► Plugin Process  │  │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘

Config: ~/.xiaolin/plugins/channel/*.json
```

Plugins communicate via **JSON-RPC 2.0 over stdin/stdout**, allowing implementations in any language (Node.js, Python, Go, Rust, etc.).

---

## 1. Adding a Plugin

### Step 1: Create Plugin Config File

Create a JSON file in the plugins directory:

```bash
mkdir -p ~/.xiaolin/plugins/channel
```

**`~/.xiaolin/plugins/channel/feishu.json`:**
```json
{
  "id": "feishu",
  "name": "Feishu/Lark",
  "version": "0.1.0",
  "description": "Feishu messaging platform integration",
  "type": "process",
  "enabled": true,
  "process": {
    "command": "node",
    "args": ["/path/to/your-plugin/dist/index.js"],
    "env": {
      "LOG_LEVEL": "info"
    },
    "transport": "stdio",
    "timeoutSecs": 30
  },
  "tools": [
    {
      "name": "feishu_send_message",
      "description": "Send a message to a Feishu chat",
      "parameters": {
        "type": "object",
        "properties": {
          "chatId": { "type": "string", "description": "Chat ID to send to" },
          "text": { "type": "string", "description": "Message content" }
        },
        "required": ["chatId", "text"]
      }
    }
  ]
}
```

### Step 2: Add Account Credentials

Account credentials go in your main XiaoLin config (`~/.xiaolin/default.json`):

```json
{
  "channels": {
    "feishu": {
      "appId": "cli_a3b2c1d4e5f6g7h8",
      "appSecret": "your-app-secret",
      "verificationToken": "your-token",
      "domain": "https://open.feishu.cn"
    }
  },
  "channelPlugins": {
    "enabled": true
  }
}
```

The `channels.feishu` object is passed to the plugin's `initialize` call.

### Step 3: Start XiaoLin

```bash
xiaolin serve
```

XiaoLin will:
1. Scan `~/.xiaolin/plugins/channel/*.json`
2. Load all enabled plugins
3. Spawn each plugin process
4. Call `initialize` with account config
5. Register the channel

---

## 2. Plugin Config Reference

### ChannelPluginConfig

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | **yes** | Unique channel identifier (e.g., "feishu", "slack") |
| `name` | string | **yes** | Human-readable name |
| `version` | string | no | Plugin version |
| `description` | string | no | Description |
| `type` | string | **yes** | Must be `"process"` |
| `enabled` | boolean | no | Default: `true`. Set to `false` to disable. |
| `process` | object | **yes** | Process configuration (see below) |
| `tools` | array | no | Tool definitions exposed by this channel |

### ProcessChannelConfig

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `command` | string | **yes** | Executable command (e.g., "node", "python3") |
| `args` | string[] | no | Command-line arguments |
| `env` | object | no | Environment variables |
| `transport` | string | no | `"stdio"` (default) or `"http"` |
| `timeoutSecs` | number | no | Request timeout in seconds |
| `maxMemoryMb` | number | no | Memory limit (Unix only) |

### ChannelToolDef

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | **yes** | Tool name (e.g., "feishu_send_message") |
| `description` | string | **yes** | Tool description for AI |
| `parameters` | object | **yes** | JSON Schema for parameters |

---

## 3. Writing a Plugin

### Protocol: JSON-RPC 2.0 over Stdio

**Wire Format:**

```
Host → Plugin (Request):
{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {...}}

Plugin → Host (Response):
{"jsonrpc": "2.0", "id": 1, "result": {"status": "ok"}}

Plugin → Host (Error):
{"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "Failed"}}

Plugin → Host (Notification):
{"jsonrpc": "2.0", "method": "inbound_message", "params": {...}}
```

- Each message is a single line (newline-delimited)
- Plugin reads from `stdin`, writes to `stdout`
- Use `stderr` for logs (not mixed with protocol)

### Required Methods

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `initialize` | `{config, protocolVersion}` | `{status, protocolVersion}` | Initialize with account config |
| `verify_webhook` | `{headers, body}` | `{status: "ok"}` | Verify webhook request |
| `handle_webhook` | `{payload}` | `{challenge}` or `{messages}` | Process webhook event |
| `send_message` | `{targetId, targetType, text, ...}` | message data | Send a message |
| `reply_message` | `{messageId, text}` | message data | Reply to a message |
| `reply_streaming_placeholder` | `{messageId, text}` | message data | Send placeholder for streaming |
| `update_message` | `{messageId, text}` | message data | Update a message (e.g., streaming) |
| `probe` | `{}` | `boolean` | Health check |
| `start` | `{}` | `{status: "ok"}` | Start listening for events |
| `stop` | `{}` | `{status: "ok"}` | Cleanup and stop |

### Method Details

#### `initialize`

Called once when the plugin is loaded. The `config` parameter contains the account credentials from `channels.<id>` in XiaoLin config.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "config": {
      "appId": "cli_xxx",
      "appSecret": "yyy",
      "domain": "https://open.feishu.cn"
    },
    "protocolVersion": "1.0"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "result": {
    "status": "ok",
    "protocolVersion": "1.0"
  }
}
```

#### `handle_webhook`

Process an incoming webhook payload.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "handle_webhook",
  "params": {
    "payload": {
      "type": "event",
      "header": { "event_type": "im.message.receive_v1" },
      "event": { ... }
    }
  }
}
```

**Response (URL verification):**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "challenge": { "challenge": "xxx" }
  }
}
```

**Response (messages):**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "messages": [
      {
        "channelId": "feishu",
        "senderId": "ou_xxx",
        "chatId": "oc_xxx",
        "messageId": "msg_xxx",
        "text": "Hello bot",
        "msgType": "text",
        "chatType": "p2p",
        "botMentioned": false,
        "extra": {}
      }
    ]
  }
}
```

#### `send_message`

Send a message to a chat or user.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "send_message",
  "params": {
    "targetId": "oc_xxx",
    "targetType": "chat_id",
    "text": "Hello!",
    "replyTo": null,
    "imageKey": null
  }
}
```

#### `start`

After `start`, the plugin may send `inbound_message` notifications for real-time events (websocket mode):

```json
{"jsonrpc": "2.0", "method": "inbound_message", "params": {...}}
```

### InboundMessage Format

When returning messages from `handle_webhook` or sending `inbound_message` notifications:

```typescript
interface InboundMessage {
  channelId: string;      // Channel ID (e.g., "feishu")
  accountId?: string;     // Account ID for multi-account routing
  senderId: string;       // User ID
  chatId: string;         // Chat/Conversation ID
  messageId: string;      // Message ID
  text: string;           // Message text content
  msgType: string;        // Message type (e.g., "text")
  chatType: string;       // "p2p" or "group"
  botMentioned: boolean;  // Whether bot was mentioned
  extra: any;             // Platform-specific data
}
```

---

## 4. Reference Implementation (TypeScript)

### Project Structure

```
my-plugin/
├── package.json
├── tsconfig.json
├── src/
│   └── index.ts
└── dist/
    └── index.js
```

### package.json

```json
{
  "name": "@myorg/my-plugin",
  "version": "0.1.0",
  "type": "module",
  "main": "dist/index.js",
  "bin": {
    "my-plugin": "./dist/index.js"
  },
  "scripts": {
    "build": "tsc",
    "start": "node dist/index.js"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.0.0"
  }
}
```

### src/index.ts

```typescript
#!/usr/bin/env node
import * as readline from 'readline';

// Types
interface JsonRpcRequest {
  jsonrpc: string;
  id: number;
  method: string;
  params: any;
}

interface JsonRpcResponse {
  jsonrpc: string;
  id: number;
  result?: any;
  error?: { code: number; message: string };
}

// Plugin State
let config: any = null;

// Method Handlers
async function handleInitialize(params: any): Promise<any> {
  config = params.config;
  console.error('[plugin] Initialized');
  return { status: 'ok', protocolVersion: params.protocolVersion };
}

async function handleProbe(): Promise<boolean> {
  return config !== null;
}

async function handleSendMessage(params: any): Promise<any> {
  // Implement your platform's send logic here
  console.error(`[plugin] Sending to ${params.targetId}: ${params.text}`);
  return { messageId: 'msg_' + Date.now() };
}

// ... implement other handlers

// Dispatcher
async function handleRequest(req: JsonRpcRequest): Promise<void> {
  try {
    let result: any;
    switch (req.method) {
      case 'initialize':
        result = await handleInitialize(req.params);
        break;
      case 'probe':
        result = await handleProbe();
        break;
      case 'send_message':
        result = await handleSendMessage(req.params);
        break;
      case 'reply_message':
        result = await handleReplyMessage(req.params);
        break;
      // ... other methods
      default:
        throw new Error(`Unknown method: ${req.method}`);
    }
    sendResponse({ jsonrpc: '2.0', id: req.id, result });
  } catch (e) {
    const error = e instanceof Error ? e : new Error(String(e));
    sendResponse({
      jsonrpc: '2.0',
      id: req.id,
      error: { code: -32000, message: error.message }
    });
  }
}

function sendResponse(res: JsonRpcResponse): void {
  console.log(JSON.stringify(res));
}

// Main loop
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  terminal: false,
});

rl.on('line', async (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  try {
    const req = JSON.parse(trimmed) as JsonRpcRequest;
    await handleRequest(req);
  } catch (e) {
    console.error(`[plugin] Parse error: ${e}`);
  }
});

rl.on('close', () => {
  console.error('[plugin] Stdin closed, exiting');
  process.exit(0);
});
```

### tsconfig.json

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*"]
}
```

### Build & Install

```bash
npm install
npm run build

# Test manually
echo '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"config":{},"protocolVersion":"1.0"}}' | node dist/index.js
```

---

## 5. Multi-Account Support

A single channel plugin can handle multiple accounts (e.g., two Feishu bots routing to different agents). This is configured via the `accounts` sub-object in channel config.

### Overview

```
┌───────────────────────────────────────────────────────────────┐
│                    XiaoLin Gateway                            │
│                                                               │
│  channels.feishu = {                                         │
│    appId: "default_app",          ← top-level defaults       │
│    accounts: {                                                │
│      bot1: { appId: "cli_a", appSecret: "..." },  ← override │
│      bot2: { appId: "cli_b", appSecret: "..." },  ← override │
│    },                                                         │
│    defaultAccount: "bot1"                                     │
│  }                                                            │
│                                                               │
│  bindings: [                                                  │
│    { agentId: "sales-agent", match: { channel: "feishu",     │
│                                       accountId: "bot1" } }, │
│    { agentId: "support-agent", match: { channel: "feishu",   │
│                                       accountId: "bot2" } }, │
│  ]                                                            │
│                                                               │
│  InboundMessage ──► resolve_route(channel, accountId) ──► agent  │
└───────────────────────────────────────────────────────────────┘
```

**Design principle:** One plugin process handles multiple accounts. The plugin receives all account configs in `initialize`, manages multiple SDK clients internally, and tags each inbound message with `accountId`.

### Channel Config with Accounts

Account credentials go in your main XiaoLin config (`~/.xiaolin/default.json`):

```json
{
  "channels": {
    "feishu": {
      "domain": "https://open.feishu.cn",
      "accounts": {
        "bot-sales": {
          "appId": "cli_sales_xxx",
          "appSecret": "secret_sales"
        },
        "bot-support": {
          "appId": "cli_support_xxx",
          "appSecret": "secret_support"
        }
      },
      "defaultAccount": "bot-sales"
    }
  }
}
```

- **Top-level fields** serve as defaults (e.g., `domain`, `replyMode`).
- **Account fields** override top-level defaults for that account.
- **`defaultAccount`** specifies which account to use when no explicit `accountId` is given.

### Binding with AccountId

Route different accounts to different agents:

```json
{
  "bindings": [
    {
      "agentId": "sales-agent",
      "match": {
        "channel": "feishu",
        "accountId": "bot-sales"
      }
    },
    {
      "agentId": "support-agent",
      "match": {
        "channel": "feishu",
        "accountId": "bot-support"
      }
    }
  ]
}
```

Binding match tiers (most-specific wins):

| Tier | Match | Example |
|------|-------|---------|
| 4 (highest) | Channel + Peer | `{ channel: "feishu", peer: { kind: "p2p", id: "ou_123" } }` |
| 3 | Channel only | `{ channel: "feishu" }` |
| 2 | Channel + AccountId | `{ channel: "feishu", accountId: "bot-sales" }` |
| 1 | Channel + wildcard account | `{ channel: "feishu", accountId: "*" }` |
| 0 (fallback) | Default agent | (no match) |

### Plugin Protocol: Multi-Account

#### Initialize — All Accounts Sent

The host sends the full `ChannelConfig` (with `accounts`) to the plugin:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "config": {
      "appId": "default_app",
      "accounts": {
        "bot1": { "appId": "cli_a", "appSecret": "..." },
        "bot2": { "appId": "cli_b", "appSecret": "..." }
      },
      "defaultAccount": "bot1"
    },
    "protocolVersion": "1.0"
  }
}
```

The plugin initializes multiple SDK clients (one per account).

#### InboundMessage — Includes accountId

Plugin tags each message with the account that received it:

```json
{
  "jsonrpc": "2.0",
  "method": "inbound_message",
  "params": {
    "channelId": "feishu",
    "accountId": "bot1",
    "senderId": "ou_xxx",
    "chatId": "oc_xxx",
    "messageId": "msg_xxx",
    "text": "Hello",
    "chatType": "p2p"
  }
}
```

#### send_message — Optional accountId

When sending, optionally specify which account to use:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "send_message",
  "params": {
    "accountId": "bot1",
    "targetId": "oc_xxx",
    "text": "Reply"
  }
}
```

### Writing a Multi-Account Plugin

The key changes from single-account:

1. **Initialize multiple clients** — iterate over `config.accounts`, merge with top-level defaults
2. **Tag inbound messages** — determine which account received the event (e.g., from `header.app_id`) and set `accountId`
3. **Route outbound calls** — use `accountId` param to select the correct SDK client

Example (TypeScript):

```typescript
class MyPlugin {
  private clients: Map<string, Client> = new Map();  // accountId → Client
  private defaultAccount: string | null = null;

  async initialize(params: { config: any; protocolVersion: string }) {
    const config = params.config;

    if (config.accounts && Object.keys(config.accounts).length > 0) {
      for (const [accountId, accConfig] of Object.entries(config.accounts)) {
        const merged = this.mergeConfig(config, accConfig);
        this.clients.set(accountId, new Client({
          appId: merged.appId,
          appSecret: merged.appSecret,
          domain: merged.domain,
        }));
      }
      this.defaultAccount = config.defaultAccount || Object.keys(config.accounts)[0];
    } else {
      // Single-account (backward compatible)
      this.clients.set('default', new Client({
        appId: config.appId,
        appSecret: config.appSecret,
      }));
      this.defaultAccount = 'default';
    }

    return { status: 'ok', protocolVersion: params.protocolVersion };
  }

  async handleWebhook(params: { payload: any }) {
    // ... parse message ...
    const accountId = this.resolveAccountFromEvent(payload);
    return {
      messages: [{ channelId: 'feishu', accountId, ... }],
    };
  }

  async sendMessage(params: { accountId?: string; targetId: string; text: string }) {
    const account = params.accountId || this.defaultAccount;
    const client = this.clients.get(account!);
    // ... use client ...
  }
}
```

### Backward Compatibility

- If `accounts` is empty/missing, the channel works as single-account (existing behavior)
- `accountId` in InboundMessage is `undefined`/omitted for single-account channels
- Routing falls back to Channel tier match when `accountId` is not set
- Session keys only include account prefix when using `PerAccountChannelPeer` scope

---

## 7. Config Customization

### Plugins Directory

Default: `~/.xiaolin/state/plugins/channel/`

Customize in `default.json`:

```json
{
  "channelPlugins": {
    "enabled": true,
    "pluginsDir": "/custom/path/to/plugins/channel"
  }
}
```

### Disable All Plugins

```json
{
  "channelPlugins": {
    "enabled": false
  }
}
```

### Disable Specific Plugin

Set `enabled: false` in the plugin JSON:

```json
{
  "id": "feishu",
  "name": "Feishu",
  "enabled": false,
  ...
}
```

### Multiple Environments

Use different config files:

```bash
xiaolin serve --config production.json
```

Each config can have different `channels` credentials.

---

## 8. Debugging

### Check Plugin Logs

Plugins write logs to stderr, which XiaoLin captures:

```bash
xiaolin serve 2>&1 | grep my-plugin
```

### Manual Plugin Test

```bash
# Send initialize request
echo '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"config":{"appId":"test","appSecret":"test"},"protocolVersion":"1.0"}}' | node /path/to/plugin/dist/index.js
```

### Common Issues

1. **Plugin not found**: Check `command` and `args` paths are absolute or relative to CWD
2. **Permission denied**: Ensure plugin file is executable (`chmod +x`)
3. **Timeout**: Increase `timeoutSecs` in process config
4. **Protocol error**: Verify JSON-RPC format (no extra whitespace, single line per message)

---

## 9. Examples

See `extensions/feishu/` for the built-in Feishu/Lark implementation (Rust). For building custom process plugins, refer to the JSON-RPC protocol documentation above.
