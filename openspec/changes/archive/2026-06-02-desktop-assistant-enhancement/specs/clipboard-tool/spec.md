## Overview

Cross-platform clipboard read/write as both Tauri commands and agent builtin tools.

## Requirements

- `clipboard_read` tool returns current clipboard content (text or image)
- `clipboard_write` tool writes text or image to system clipboard
- Supported platforms: macOS, Windows, Linux (Wayland + X11)
- Image format: PNG for cross-platform compatibility
- Permission: requires user consent on first use (macOS Accessibility)
- Tool available in both Agent and Plan modes (Direct exposure)
