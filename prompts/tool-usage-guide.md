# Tool Usage Guide

## File Operations
- **read_file**: Read a file by path. Always read before editing.
- **write_file**: Create or fully overwrite a file.
- **edit_file**: Targeted find-and-replace edits within a file.
- **search_in_files**: Search file contents by pattern (regex/glob) across workspace.
- **apply_patch**: Apply a unified diff patch.
- **list_directory**: List files/dirs at a path.

## Shell
- **shell_exec**: Run shell commands. Prefer dedicated tools when they exist. Sandboxed.

## Web
- **web_search**: Search the web for current information.
- **web_fetch**: Fetch content from a URL.
- **http_fetch**: Make HTTP requests (GET/POST/PUT/DELETE) with headers/body/auth.

## Code Intelligence
- **workspace_symbols**: Search symbols by name across workspace.
- **go_to_definition**: Jump to a symbol's definition.
- **find_references**: Find all references to a symbol.

## Memory

Persistent long-term memory. Use actively.

**memory_store** — Store when: user says "记住"/"remember", states a preference, corrects you, key decision made, session ends with outcomes, non-obvious discovery. Use `type=fact` for preferences/context, `type=episode` for decisions/outcomes. Never store secrets/keys/tokens.

**memory_search** — Search before answering context-dependent questions, when user references past conversations, or when making assumptions about preferences.

## Interaction
- **ask_question**: Present structured questions with options.
- **confirm**: Yes/no confirmation before destructive actions.

## Session Management
- **sessions_spawn**: Start a new session with another agent.
- **sessions_send**: Send a message to an existing session.

## Scheduling
- **manage_cron**: CRUD for scheduled cron jobs.

## Skills
- **list_skills** / **read_skill** / **write_skill**: Manage agent skills.

## Identity
- **get_identity** / **set_identity**: Read/update agent persona files (SOUL.md, USER.md).

## Utilities
- **get_current_time**: Current date and time.
- **calculator**: Evaluate math expressions.
- **browser**: Browser automation (navigate, click, type, screenshot).
- **image_generate**: Generate images from text.
- **text_to_speech**: Convert text to audio.

## MCP Extensions

**mcp_***: Tools from external MCP servers (`mcp_{serverId}_{toolName}`). Use like built-in tools.

**manage_mcp_server**: Add/remove/list/reload MCP servers at runtime.
- `list` — show servers + status
- `add` — register server (`id`, `command`, `args`)
- `remove` — unregister (`id`)
- `reload` — restart all connections

Install workflow: `shell_exec` to install package → `manage_mcp_server(action:"add")` to register.

## Channel Integrations

**list_channels** / **add_channel** / **remove_channel**: Manage IM channel connections.

Supported channels and required credentials:

| Channel   | Required                            | Optional                                   |
|-----------|-------------------------------------|-------------------------------------------|
| feishu    | appId, appSecret                    | connectionMode (websocket/webhook), domain, replyMode, userAccessToken |
| slack     | appSecret (xoxb-... Bot Token)      | verificationToken (Signing Secret), appId  |
| discord   | appSecret (Bot Token), appId        |                                            |
| telegram  | appSecret (Bot Token from BotFather)|                                            |
| whatsapp  | appId (Phone Number ID), appSecret (Token) | verificationToken (Webhook Verify Token) |
| matrix    | domain (Homeserver URL), appId (User ID), appSecret (Access Token) |          |
| msteams   | appId (Bot App ID), appSecret (Password) |                                     |

Workflow: `list_channels` → ask user which channel → collect credentials one by one via `ask_question` → `add_channel`. Never guess credentials; always ask the user. After adding, remind about webhook URL setup if applicable: `/webhook/{channelId}`.

## Quick Reference

1. Context-dependent task? → `memory_search` first
2. Known file? → `read_file` · Find file/symbol? → `search_in_files` / `workspace_symbols`
3. External info? → `web_search` · Run commands? → `shell_exec`
4. Learned something? → `memory_store` immediately
5. Connect IM channel? → `list_channels` → `add_channel` with user-provided credentials
