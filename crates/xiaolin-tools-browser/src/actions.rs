use std::path::{Component, Path};
use std::time::Duration;

pub const DEFAULT_ELEMENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Validate UID format to prevent CSS selector injection (rule #40).
/// Valid UIDs match `^e\d+$` (e.g. "e0", "e42", "e1023").
pub fn validate_uid(uid: &str) -> Result<(), String> {
    if uid.is_empty() {
        return Err("empty uid".into());
    }
    if !uid.starts_with('e') {
        return Err(format!("invalid uid format: {uid}"));
    }
    if !uid[1..].chars().all(|c| c.is_ascii_digit()) || uid.len() < 2 {
        return Err(format!("invalid uid format: {uid}"));
    }
    Ok(())
}

pub fn parse_timeout(args: &serde_json::Value) -> Duration {
    args.get("timeout")
        .and_then(|v| v.as_u64())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ELEMENT_TIMEOUT)
}

pub fn require_selector<'a>(args: &'a serde_json::Value, action: &str) -> Result<&'a str, String> {
    args.get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!(
                "browser {action}: missing string field 'selector'. \
                 Example: {{\"action\": \"{action}\", \"selector\": \"button.submit\"}}."
            )
        })
}

pub fn validate_url_scheme(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return Err(format!(
            "browser: URL scheme not allowed for '{trimmed}'. Only http:// and https:// are permitted."
        ));
    }
    xiaolin_security::ssrf::ssrf_check_url(trimmed).map_err(|e| {
        format!("browser: URL blocked for '{trimmed}': {e}")
    })
}

pub fn workspace_root_for_paths() -> std::path::PathBuf {
    std::env::current_dir()
        .map(|cwd| xiaolin_core::workspace::detect_workspace_root(&cwd))
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

pub fn canonicalize_or_self(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
}

pub fn validate_output_path(path: &str) -> Result<std::path::PathBuf, String> {
    let input = Path::new(path);
    if input.as_os_str().is_empty() {
        return Err("browser: output path must not be empty".to_string());
    }
    if input
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "browser: output path '{path}' must not contain '..'"
        ));
    }

    let workspace = workspace_root_for_paths();
    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        workspace.join(input)
    };

    let parent = absolute
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(workspace.as_path());

    if !parent.exists() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "browser: cannot create output directory '{}': {e}",
                parent.display()
            )
        })?;
    }

    let canon_parent = parent.canonicalize().map_err(|e| {
        format!(
            "browser: cannot resolve output path parent for '{path}': {e}"
        )
    })?;

    let allowed_roots = [
        canonicalize_or_self(&workspace),
        canonicalize_or_self(&std::env::temp_dir()),
    ];
    let under_allowed = allowed_roots
        .iter()
        .any(|root| canon_parent.starts_with(root));

    if !under_allowed {
        return Err(format!(
            "browser: output path '{path}' is outside the workspace or temp directory"
        ));
    }

    let file_name = absolute.file_name().ok_or_else(|| {
        format!("browser: output path '{path}' must include a file name")
    })?;

    let candidate = canon_parent.join(file_name);
    let resolved = if candidate.exists() || candidate.symlink_metadata().is_ok() {
        candidate.canonicalize().map_err(|e| {
            format!("browser: cannot resolve output path '{path}': {e}")
        })?
    } else {
        candidate
    };

    let under_allowed = allowed_roots
        .iter()
        .any(|root| resolved.starts_with(root));
    if !under_allowed {
        return Err(format!(
            "browser: output path '{path}' resolves outside the workspace or temp directory"
        ));
    }

    Ok(resolved)
}

pub fn validate_args(action: &str, args: &serde_json::Value) -> Result<(), String> {
    match action {
        "navigate" => {}
        "evaluate" => {
            if args.get("script").and_then(|v| v.as_str()).is_none()
                && args.get("function").and_then(|v| v.as_str()).is_none()
            {
                return Err("browser evaluate: missing 'script' or 'function'. \
                     Example: {\"action\": \"evaluate\", \"script\": \"document.title\"}."
                    .to_string());
            }
        }
        "click" | "hover" | "drag" => {
            if args.get("uid").and_then(|v| v.as_str()).is_none()
                && args.get("selector").and_then(|v| v.as_str()).is_none()
            {
                return Err(format!(
                    "browser {action}: missing 'uid' or 'selector'. \
                     Use take_snapshot to get element UIDs, or provide a CSS selector."
                ));
            }
        }
        "fill" => {
            if args.get("uid").and_then(|v| v.as_str()).is_none()
                && args.get("selector").and_then(|v| v.as_str()).is_none()
            {
                return Err("browser fill: missing 'uid' or 'selector'.".to_string());
            }
            if args.get("value").and_then(|v| v.as_str()).is_none() {
                return Err("browser fill: missing string field 'value'.".to_string());
            }
        }
        "fill_form" => {
            if args.get("elements").and_then(|v| v.as_array()).is_none() {
                return Err("browser fill_form: missing 'elements' array.".to_string());
            }
        }
        "type_text" => {
            if args.get("text").and_then(|v| v.as_str()).is_none() {
                return Err("browser type_text: missing string field 'text'.".to_string());
            }
        }
        "press_key" => {
            if args.get("key").and_then(|v| v.as_str()).is_none() {
                return Err(
                    "browser press_key: missing string field 'key'. Example: \"Enter\", \"Control+A\"."
                        .to_string(),
                );
            }
        }
        "wait_for" => {
            if args.get("text").is_none() && args.get("selector").is_none() {
                return Err(
                    "browser wait_for: provide 'text' (array of strings) or 'selector' (CSS)."
                        .to_string(),
                );
            }
        }
        "select_page" => {
            if args.get("pageId").and_then(|v| v.as_u64()).is_none() {
                return Err("browser select_page: missing 'pageId'.".to_string());
            }
        }
        "new_page" => {
            if args.get("url").and_then(|v| v.as_str()).is_none() {
                return Err("browser new_page: missing 'url'.".to_string());
            }
        }
        "close_page" => {
            if args.get("pageId").and_then(|v| v.as_u64()).is_none() {
                return Err("browser close_page: missing 'pageId'.".to_string());
            }
        }
        "handle_dialog" => {
            let a = args
                .get("dialog_action")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if a != "accept" && a != "dismiss" {
                return Err(
                    "browser handle_dialog: 'dialog_action' must be \"accept\" or \"dismiss\"."
                        .to_string(),
                );
            }
        }
        "resize_page" => {
            if args.get("width").is_none() || args.get("height").is_none() {
                return Err("browser resize_page: missing 'width' and/or 'height'.".to_string());
            }
        }
        "cookies" => {
            let op = args
                .get("operation")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            if (op == "set" || op == "delete")
                && args.get("cookie_name").and_then(|v| v.as_str()).is_none()
            {
                return Err(format!("browser cookies {op}: missing 'cookie_name'."));
            }
        }
        "pdf" => {
            if args.get("output_path").and_then(|v| v.as_str()).is_none() {
                return Err("browser pdf: missing 'output_path'.".to_string());
            }
        }
        "take_snapshot"
        | "screenshot"
        | "scroll"
        | "interact"
        | "get_content"
        | "list_pages"
        | "list_network_requests"
        | "list_console_messages"
        | "emulate" => {}
        "upload_file" => {
            if args.get("uid").and_then(|v| v.as_str()).is_none()
                && args.get("selector").and_then(|v| v.as_str()).is_none()
            {
                return Err("browser upload_file: missing 'uid' or 'selector'.".to_string());
            }
            if args.get("filePath").and_then(|v| v.as_str()).is_none() {
                return Err("browser upload_file: missing 'filePath'.".to_string());
            }
        }
        "get_console_message" => {
            if args.get("msgid").and_then(|v| v.as_u64()).is_none() {
                return Err("browser get_console_message: missing 'msgid'.".to_string());
            }
        }
        "get_network_request" => {
            if args.get("reqid").and_then(|v| v.as_u64()).is_none() {
                return Err("browser get_network_request: missing 'reqid'.".to_string());
            }
        }
        "type" => {
            if args.get("selector").and_then(|v| v.as_str()).is_none() {
                return Err("browser type: missing 'selector'.".to_string());
            }
            if args.get("text").and_then(|v| v.as_str()).is_none() {
                return Err("browser type: missing 'text'.".to_string());
            }
        }
        "select" => {
            if args.get("selector").and_then(|v| v.as_str()).is_none() {
                return Err("browser select: missing 'selector'.".to_string());
            }
        }
        "go_back" | "go_forward" | "reload" => {}
        "set_hosts" => {
            if args.get("mappings").and_then(|v| v.as_array()).is_none()
                && args.get("hosts").and_then(|v| v.as_array()).is_none()
            {
                return Err(
                    "browser set_hosts: missing 'mappings' array of {pattern, target_ip}."
                        .to_string(),
                );
            }
        }
        "set_proxy" => {}
        "get_network_config" | "clear_hosts" => {}
        other => {
            return Err(format!(
                "browser: unknown action '{other}'. \
                 Valid: navigate, take_snapshot, screenshot, evaluate, click, fill, fill_form, \
                 type_text, press_key, hover, select, wait_for, scroll, drag, handle_dialog, \
                 list_pages, select_page, new_page, close_page, cookies, pdf, interact, \
                 get_content, list_network_requests, list_console_messages, \
                 get_console_message, get_network_request, upload_file, emulate, resize_page, \
                 set_hosts, set_proxy, get_network_config, clear_hosts. \
                 Legacy: type, go_back, go_forward, reload."
            ));
        }
    }
    Ok(())
}
