# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.0.x   | ✅ Current |

## Reporting a Vulnerability

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, use one of the following channels:

1. **GitHub Private Reporting** — Use the [Security Advisories](../../security/advisories) tab to create a private advisory.
2. **Email** — Send details to the maintainers listed in `Cargo.toml`.

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Impact assessment (what an attacker could achieve)
- Affected component(s): gateway, agent runtime, WASM plugins, MCP, channels, etc.

### Response Timeline

- **Acknowledgement**: within 48 hours
- **Initial assessment**: within 5 business days
- **Fix or mitigation**: varies by severity (critical: ASAP, high: 2 weeks, medium/low: next release)

## Security Architecture

FastClaw implements defense-in-depth across multiple layers:

- **Authentication**: Constant-time API key validation, per-request auth middleware
- **Rate limiting**: IP-based with configurable windows and trusted proxy support
- **Prompt injection guard**: Heuristic + pattern-based detection before LLM calls
- **Message bus security**: HMAC-SHA256 signing with replay protection and hop-depth limits
- **WASM sandboxing**: Fuel-limited execution with epoch-based shutdown
- **SSRF prevention**: Private IP blocking + DNS resolution validation
- **Path traversal guards**: Normalized path validation for file operations
- **Webhook verification**: Signature validation for Slack, WhatsApp, Feishu inbound webhooks
- **Budget enforcement**: Atomic reserve/release token budget per model
- **Code sandbox**: Shell execution disabled by default, output size limits
- **Config ACL**: Readable/writable key allow-lists with secret value masking

See `crates/fastclaw-security/` for implementation details.
