## Overview

Define boundaries, registration mechanism, and dependency rules for splitting builtin tools out of `xiaolin-agent` into domain-specific crates.

## Requirements

- Tools in `xiaolin-tools-fs` must not depend on `xiaolin-agent` internals
- Tools in `xiaolin-tools-network` must not depend on `xiaolin-agent` internals
- Tools in `xiaolin-tools-browser` must be feature-gated (`browser` feature)
- Tools in `xiaolin-tools-code` must not depend on `xiaolin-agent` internals
- Each tool crate exports `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)`
- `xiaolin-agent::register_builtin_tools` calls each tool crate's register function
- Shared types used across tool crates live in `xiaolin-core`
- Tool tests migrate with their source code to the new crate
- No change to external tool behavior or API surface
