# Tool Usage Guide

## File Operations
- **read_file**: Use when you need to examine a specific file. Always read before editing.
- **write_file**: Use to create or overwrite files. Prefer for new files or complete rewrites.
- **list_directory**: Use to explore directory structure. Always check before accessing files.

## Shell
- **shell_exec**: Use for complex operations that need shell features (pipes, globbing, git, package managers). Prefer dedicated tools when available. Commands are sandboxed — destructive operations may be blocked.

## Web
- **web_search**: Use for current information, documentation, error lookups. Prefer over fabricating answers.
- **web_fetch**: Use to read specific URLs (docs, articles, API responses).

## Memory
- **memory_store**: Proactively store important facts and events you learn during conversations.
- **memory_search**: Search before answering questions that might relate to past context.

## MCP Extensions
- **mcp_***: Tools provided by external MCP (Model Context Protocol) servers. These tools are prefixed with `mcp_{serverId}_` and extend your capabilities with integrations like browser automation, desktop app control, and more. Use them like any other tool by calling the full prefixed name.

## Decision Tree
1. Need info from a specific file? → read_file
2. Need to find a file? → list_directory → read_file
3. Need current/external info? → web_search
4. Need to run code/commands? → shell_exec
5. Learned something important? → memory_store
6. Question about past context? → memory_search
7. Need browser/app automation? → check available mcp_* tools
