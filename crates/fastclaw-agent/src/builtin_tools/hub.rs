use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};

// ---------- ClawHub Tools ----------

/// Search for skills on ClawHub marketplace.
pub struct HubSearchTool {
    hub: Arc<tokio::sync::Mutex<fastclaw_core::hub::HubClient>>,
}

impl HubSearchTool {
    pub fn new(hub: Arc<tokio::sync::Mutex<fastclaw_core::hub::HubClient>>) -> Self {
        Self { hub }
    }
}

#[async_trait]
impl Tool for HubSearchTool {
    fn name(&self) -> &str {
        "hub_search"
    }

    fn description(&self) -> &str {
        "Query the ClawHub public marketplace for skills and return metadata rows: id, name, version, description, author, tags, downloads. Read-only—nothing is installed yet. \
         Use hub_search when the user wants a packaged workflow or integration and you need real ids before hub_install, or before drafting write_skill from scratch. \
         After picking candidates, install with hub_install using the exact id field, then list_skills + read_skill to inspect SKILL.md and hooks. \
         Local-only or private skills are invisible here—use list_skills/read_skill/write_skill for workspace or repo content. \
         Broad one-word queries are noisy; add stack, vendor, or channel hints ('rust sqlx migrate', 'slack incoming webhook card'). \
         limit caps rows in context—default to ~5 unless you intentionally compare many options. \
         Anti-pattern: hub_install without a trusted id—search or ask first. \
         Example: {\"query\": \"feishu lark card notifier\", \"limit\": 6}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Keywords describing capability, vendor, stack, or file type. Examples: 'postgres migration helper', 'slack slash command', 'wps365 drive upload'. Longer, specific queries usually rank better than one-word searches."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum packages to return (default 10, integer). Examples: 5 for a tight shortlist, 15 when comparing many similar plugins. Lower values keep tool output small."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "hub_search: arguments are not valid JSON: {e}. \
                 Pass {{\"query\": \"keywords\", \"limit\": 10}}; limit is optional."
            )),
        };

        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => return ToolResult::err(
                "hub_search is missing required string field 'query'. \
                 Example: {\"query\": \"kubernetes deploy skill\"}."
                    .to_string(),
            ),
        };
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let hub = self.hub.lock().await;
        match hub.search(query, limit).await {
            Ok(result) => {
                let packages: Vec<serde_json::Value> = result
                    .packages
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "id": p.id,
                            "name": p.name,
                            "version": p.version,
                            "description": p.description,
                            "author": p.author,
                            "tags": p.tags,
                            "downloads": p.downloads,
                        })
                    })
                    .collect();
                ToolResult::ok(
                    serde_json::json!({
                        "query": query,
                        "results": packages,
                        "total": result.total,
                    })
                    .to_string(),
                )
            }
            Err(e) => ToolResult::err(format!(
                "hub_search failed for query '{query}': {e}. \
                 What went wrong: the hub client could not complete the search request (network, TLS, DNS, auth, or server-side failure). \
                 What to do next: verify the configured hub base URL and any API tokens with the operator; retry with a shorter or more specific query; if you see HTTP 429, back off; if the hub is optional for the task, fall back to write_skill or repo-local skills instead of looping identical queries."
            )),
        }
    }
}

/// Install a skill from ClawHub marketplace.
pub struct HubInstallTool {
    hub: Arc<tokio::sync::Mutex<fastclaw_core::hub::HubClient>>,
}

impl HubInstallTool {
    pub fn new(hub: Arc<tokio::sync::Mutex<fastclaw_core::hub::HubClient>>) -> Self {
        Self { hub }
    }
}

#[async_trait]
impl Tool for HubInstallTool {
    fn name(&self) -> &str {
        "hub_install"
    }

    fn description(&self) -> &str {
        "Download and install a marketplace skill package onto the gateway filesystem (ids may look like plain names or owner/repo slugs depending on hub configuration). \
         Success JSON includes status, resolved skill_id, semver, install_path, and files touched—use read_skill or read_file next to review content. \
         Only install ids you trust from hub_search or the user; blind installs can overwrite directories or pull unwanted code. \
         This is a mutating operation under the configured skills root—not a dry run. For reading upstream docs without installing, use web_fetch instead. \
         Omit version to track latest; pin (e.g. \"1.4.2\") when the user needs reproducible CI or rollback. \
         Many hosts need reload/restart before list_skills shows new entries—follow operator guidance. \
         Anti-pattern: retry-looping the same failing id without reading the error. \
         Example pinned: {\"skill_id\": \"feishu-notify\", \"version\": \"1.2.0\"}; latest: {\"skill_id\": \"acme-corp/deploy-runbook\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "skill_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Package id from hub_search (copy the JSON 'id' field exactly, including path-like segments), or another id format your hub accepts (e.g. 'owner/repo'). Examples: 'slack-slash-commands', 'wps365-skills/drive'. Typos typically yield not-found—re-run hub_search instead of guessing variants."
            }),
        );
        props.insert(
            "version".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional semver string to pin, e.g. '1.4.2'. Omit or null for latest. Use a pin when the user named a version, CI must be stable, or a past upgrade broke behavior."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["skill_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "hub_install: arguments are not valid JSON: {e}. \
                 Pass {{\"skill_id\": \"id-from-hub_search\", \"version\": \"1.2.3\"}} with double-quoted keys; version is optional."
            )),
        };

        let skill_id = match args.get("skill_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => return ToolResult::err(
                "hub_install is missing or empty required string field 'skill_id'. \
                 Example: {\"skill_id\": \"feishu-notify\"}. \
                 What to do next: copy the exact 'id' from hub_search results (or use the user's id), then retry—do not invent slugs."
                    .to_string(),
            ),
        };
        let version = args.get("version").and_then(|v| v.as_str());

        let hub = self.hub.lock().await;
        match hub.install(skill_id, version).await {
            Ok(result) => ToolResult::ok(
                serde_json::json!({
                    "status": "installed",
                    "skill_id": result.skill_id,
                    "version": result.version,
                    "path": result.install_path.display().to_string(),
                    "files": result.files,
                })
                .to_string(),
            ),
            Err(e) => ToolResult::err(format!(
                "hub_install failed for skill_id '{skill_id}'{}: {e}. \
                 What went wrong: the hub client could not download, verify, or write the package (not found, version mismatch, network/TLS, disk full, or permission denied on the skills directory). \
                 What to do next: confirm the id and optional version with hub_search; ensure the gateway can write the skills install path; retry after fixing credentials or disk; if the package is deprecated, pick another id from search results.",
                version.map(|v| format!(" (requested version '{v}')")).unwrap_or_default()
            )),
        }
    }
}
