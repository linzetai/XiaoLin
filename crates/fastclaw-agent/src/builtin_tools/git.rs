use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};

use super::filesystem::get_effective_work_dir;

pub struct GitTool;

#[async_trait]
impl Tool for GitTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Git operations with structured JSON output. Subcommands: status, diff, log, branch, \
         show, stash_list. Read-only operations only — use shell_exec for commits, push, merge, \
         rebase and other write operations."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "subcommand".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["status", "diff", "log", "branch", "show", "stash_list"],
                "description": "Git subcommand. \
                 status: working tree status (staged, unstaged, untracked). \
                 diff: show changes (optionally for specific file or --staged). \
                 log: recent commits (default 10). \
                 branch: list branches with current marker. \
                 show: show a specific commit. \
                 stash_list: list stash entries."
            }),
        );
        props.insert(
            "file".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional file path to scope diff or log to a specific file."
            }),
        );
        props.insert(
            "staged".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "For diff: show staged changes only (--cached). Default false."
            }),
        );
        props.insert(
            "max_count".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "For log: max number of commits (default 10, max 50)."
            }),
        );
        props.insert(
            "ref".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "For show: commit hash or ref to show. For diff: base ref (e.g. 'main..HEAD')."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["subcommand".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "git: invalid JSON: {e}. Example: {{\"subcommand\": \"status\"}}"
                ))
            }
        };

        let subcmd = match args.get("subcommand").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::err(
                    "git: missing required 'subcommand'. \
                     Use one of: status, diff, log, branch, show, stash_list."
                        .to_string(),
                )
            }
        };

        let cwd =
            get_effective_work_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        match subcmd {
            "status" => exec_status(&cwd).await,
            "diff" => {
                let file = args.get("file").and_then(|v| v.as_str());
                let staged = args
                    .get("staged")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let base_ref = args.get("ref").and_then(|v| v.as_str());
                exec_diff(&cwd, file, staged, base_ref).await
            }
            "log" => {
                let max_count = args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .min(50) as usize;
                let file = args.get("file").and_then(|v| v.as_str());
                exec_log(&cwd, max_count, file).await
            }
            "branch" => exec_branch(&cwd).await,
            "show" => {
                let ref_name = args.get("ref").and_then(|v| v.as_str()).unwrap_or("HEAD");
                exec_show(&cwd, ref_name).await
            }
            "stash_list" => exec_stash_list(&cwd).await,
            other => ToolResult::err(format!(
                "git: unknown subcommand '{other}'. \
                 Use one of: status, diff, log, branch, show, stash_list."
            )),
        }
    }
}

async fn run_git(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| format!("git: failed to execute: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn exec_status(cwd: &std::path::Path) -> ToolResult {
    let porcelain = match run_git(cwd, &["status", "--porcelain=v1", "-b"]).await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let mut branch = String::new();
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();

    for line in porcelain.lines() {
        if let Some(b) = line.strip_prefix("## ") {
            branch = b.split("...").next().unwrap_or(b).to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let x = line.as_bytes()[0] as char;
        let y = line.as_bytes()[1] as char;
        let path = line[3..].to_string();

        if x == '?' && y == '?' {
            untracked.push(path);
        } else {
            if x != ' ' && x != '?' {
                staged.push(serde_json::json!({
                    "status": status_char_to_str(x),
                    "path": &path,
                }));
            }
            if y != ' ' && y != '?' {
                unstaged.push(serde_json::json!({
                    "status": status_char_to_str(y),
                    "path": &path,
                }));
            }
        }
    }

    let clean = staged.is_empty() && unstaged.is_empty() && untracked.is_empty();

    ToolResult::ok(
        serde_json::json!({
            "branch": branch,
            "clean": clean,
            "staged": staged,
            "unstaged": unstaged,
            "untracked": untracked,
        })
        .to_string(),
    )
}

fn status_char_to_str(c: char) -> &'static str {
    match c {
        'M' => "modified",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'U' => "unmerged",
        'T' => "type_changed",
        _ => "unknown",
    }
}

async fn exec_diff(
    cwd: &std::path::Path,
    file: Option<&str>,
    staged: bool,
    base_ref: Option<&str>,
) -> ToolResult {
    let mut cmd_args = vec!["diff", "--stat"];
    if staged {
        cmd_args.push("--cached");
    }
    if let Some(r) = base_ref {
        cmd_args.push(r);
    }
    if let Some(f) = file {
        cmd_args.push("--");
        cmd_args.push(f);
    }

    let stat = match run_git(cwd, &cmd_args).await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let mut detail_args = vec!["diff"];
    if staged {
        detail_args.push("--cached");
    }
    if let Some(r) = base_ref {
        detail_args.push(r);
    }
    if let Some(f) = file {
        detail_args.push("--");
        detail_args.push(f);
    }

    let diff_text = match run_git(cwd, &detail_args).await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    const MAX_DIFF_LEN: usize = 16_000;
    let truncated = diff_text.len() > MAX_DIFF_LEN;
    let shown = if truncated {
        &diff_text[..MAX_DIFF_LEN]
    } else {
        &diff_text
    };

    ToolResult::ok(
        serde_json::json!({
            "stat": stat.trim(),
            "diff": shown.trim(),
            "truncated": truncated,
        })
        .to_string(),
    )
}

async fn exec_log(cwd: &std::path::Path, max_count: usize, file: Option<&str>) -> ToolResult {
    let count_str = max_count.to_string();
    let mut cmd_args = vec![
        "log",
        "--format=%H%x00%h%x00%an%x00%ae%x00%aI%x00%s",
        "-n",
        &count_str,
    ];
    if let Some(f) = file {
        cmd_args.push("--");
        cmd_args.push(f);
    }

    let output = match run_git(cwd, &cmd_args).await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let commits: Vec<serde_json::Value> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(6, '\0').collect();
            if parts.len() >= 6 {
                Some(serde_json::json!({
                    "hash": parts[0],
                    "short_hash": parts[1],
                    "author": parts[2],
                    "email": parts[3],
                    "date": parts[4],
                    "message": parts[5],
                }))
            } else {
                None
            }
        })
        .collect();

    ToolResult::ok(
        serde_json::json!({
            "count": commits.len(),
            "commits": commits,
        })
        .to_string(),
    )
}

async fn exec_branch(cwd: &std::path::Path) -> ToolResult {
    let output = match run_git(
        cwd,
        &[
            "branch",
            "-a",
            "--format=%(HEAD)%(refname:short)%00%(objectname:short)%00%(upstream:short)",
        ],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let mut current = String::new();
    let mut branches = Vec::new();

    for line in output.lines() {
        let is_current = line.starts_with('*');
        let rest = if is_current { &line[1..] } else { line };
        let parts: Vec<&str> = rest.splitn(3, '\0').collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts[0].trim().to_string();
        let hash = parts.get(1).unwrap_or(&"").trim().to_string();
        let upstream = parts.get(2).unwrap_or(&"").trim().to_string();

        if is_current {
            current = name.clone();
        }

        let mut entry = serde_json::json!({
            "name": name,
            "hash": hash,
            "current": is_current,
        });
        if !upstream.is_empty() {
            entry["upstream"] = serde_json::Value::String(upstream);
        }
        branches.push(entry);
    }

    ToolResult::ok(
        serde_json::json!({
            "current": current,
            "branches": branches,
        })
        .to_string(),
    )
}

async fn exec_show(cwd: &std::path::Path, ref_name: &str) -> ToolResult {
    let output = match run_git(
        cwd,
        &[
            "show",
            "--format=%H%n%h%n%an%n%ae%n%aI%n%s%n%b",
            "--stat",
            ref_name,
        ],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let lines: Vec<&str> = output.lines().collect();
    if lines.len() < 6 {
        return ToolResult::err(format!("git show: unexpected output for '{ref_name}'"));
    }

    let body_end = lines.iter().position(|l| l.is_empty()).unwrap_or(6);
    let body = if body_end > 6 {
        lines[6..body_end].join("\n")
    } else {
        String::new()
    };
    let stat = lines[body_end..].join("\n");

    ToolResult::ok(
        serde_json::json!({
            "hash": lines[0],
            "short_hash": lines[1],
            "author": lines[2],
            "email": lines[3],
            "date": lines[4],
            "message": lines[5],
            "body": body.trim(),
            "stat": stat.trim(),
        })
        .to_string(),
    )
}

async fn exec_stash_list(cwd: &std::path::Path) -> ToolResult {
    let output = match run_git(cwd, &["stash", "list", "--format=%gD%x00%gs%x00%aI"]).await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(e),
    };

    let entries: Vec<serde_json::Value> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\0').collect();
            if parts.len() >= 2 {
                Some(serde_json::json!({
                    "ref": parts[0],
                    "message": parts[1],
                    "date": parts.get(2).unwrap_or(&""),
                }))
            } else {
                None
            }
        })
        .collect();

    ToolResult::ok(
        serde_json::json!({
            "count": entries.len(),
            "entries": entries,
        })
        .to_string(),
    )
}
