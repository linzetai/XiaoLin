use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use fs2::FileExt;
use thiserror::Error;

use crate::config::{normalize_network_rule_host, NetworkRuleProtocol};

/// Simple decision type for amend operations (allow / prompt / deny).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmendDecision {
    Allow,
    Prompt,
    Deny,
}

impl AmendDecision {
    fn as_policy_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Prompt => "prompt",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Error)]
pub enum AmendError {
    #[error("prefix rule requires at least one token")]
    EmptyPrefix,
    #[error("invalid network rule: {0}")]
    InvalidNetworkRule(String),
    #[error("policy path has no parent: {path}")]
    MissingParent { path: PathBuf },
    #[error("failed to create policy directory {dir}: {source}")]
    CreatePolicyDir {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to format prefix tokens: {source}")]
    SerializePrefix { source: serde_json::Error },
    #[error("failed to serialize network rule field: {source}")]
    SerializeNetworkRule { source: serde_json::Error },
    #[error("failed to open policy file {path}: {source}")]
    OpenPolicyFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write to policy file {path}: {source}")]
    WritePolicyFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to lock policy file {path}: {source}")]
    LockPolicyFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to seek policy file {path}: {source}")]
    SeekPolicyFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read policy file {path}: {source}")]
    ReadPolicyFile {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Append an allow-prefix rule to a policy file using advisory file locking.
///
/// Uses blocking I/O — call via `tokio::task::spawn_blocking` from async contexts.
pub fn blocking_append_allow_prefix_rule(
    policy_path: &Path,
    prefix: &[String],
) -> Result<(), AmendError> {
    if prefix.is_empty() {
        return Err(AmendError::EmptyPrefix);
    }

    let tokens = prefix
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| AmendError::SerializePrefix { source })?;
    let pattern = format!("[{}]", tokens.join(", "));
    let rule = format!(r#"prefix_rule(pattern={pattern}, decision="allow")"#);
    append_rule_line(policy_path, &rule)
}

/// Append a network rule to a policy file using advisory file locking.
///
/// Uses blocking I/O — call via `tokio::task::spawn_blocking` from async contexts.
pub fn blocking_append_network_rule(
    policy_path: &Path,
    host: &str,
    protocol: &NetworkRuleProtocol,
    decision: AmendDecision,
    justification: Option<&str>,
) -> Result<(), AmendError> {
    let host = normalize_network_rule_host(host)
        .map_err(|err| AmendError::InvalidNetworkRule(err.clone()))?;

    if let Some(raw) = justification {
        if raw.trim().is_empty() {
            return Err(AmendError::InvalidNetworkRule(
                "justification cannot be empty".to_string(),
            ));
        }
    }

    let host = serde_json::to_string(&host)
        .map_err(|source| AmendError::SerializeNetworkRule { source })?;
    let protocol = serde_json::to_string(&format!("{protocol}"))
        .map_err(|source| AmendError::SerializeNetworkRule { source })?;
    let decision_str = serde_json::to_string(decision.as_policy_str())
        .map_err(|source| AmendError::SerializeNetworkRule { source })?;

    let mut args = vec![
        format!("host={host}"),
        format!("protocol={protocol}"),
        format!("decision={decision_str}"),
    ];
    if let Some(justification) = justification {
        let justification = serde_json::to_string(justification)
            .map_err(|source| AmendError::SerializeNetworkRule { source })?;
        args.push(format!("justification={justification}"));
    }
    let rule = format!("network_rule({})", args.join(", "));
    append_rule_line(policy_path, &rule)
}

fn append_rule_line(policy_path: &Path, rule: &str) -> Result<(), AmendError> {
    let dir = policy_path
        .parent()
        .ok_or_else(|| AmendError::MissingParent {
            path: policy_path.to_path_buf(),
        })?;
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(AmendError::CreatePolicyDir {
                dir: dir.to_path_buf(),
                source,
            });
        }
    }

    append_locked_line(policy_path, rule)
}

fn append_locked_line(policy_path: &Path, line: &str) -> Result<(), AmendError> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(policy_path)
        .map_err(|source| AmendError::OpenPolicyFile {
            path: policy_path.to_path_buf(),
            source,
        })?;
    file.lock_exclusive()
        .map_err(|source| AmendError::LockPolicyFile {
            path: policy_path.to_path_buf(),
            source,
        })?;

    file.seek(SeekFrom::Start(0))
        .map_err(|source| AmendError::SeekPolicyFile {
            path: policy_path.to_path_buf(),
            source,
        })?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|source| AmendError::ReadPolicyFile {
            path: policy_path.to_path_buf(),
            source,
        })?;

    if contents.lines().any(|existing| existing == line) {
        return Ok(());
    }

    if !contents.is_empty() && !contents.ends_with('\n') {
        file.write_all(b"\n")
            .map_err(|source| AmendError::WritePolicyFile {
                path: policy_path.to_path_buf(),
                source,
            })?;
    }

    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|source| AmendError::WritePolicyFile {
            path: policy_path.to_path_buf(),
            source,
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn appends_rule_and_creates_directories() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");

        blocking_append_allow_prefix_rule(
            &policy_path,
            &[String::from("echo"), String::from("Hello, world!")],
        )
        .expect("append rule");

        let contents = std::fs::read_to_string(&policy_path).expect("default.rules should exist");
        assert_eq!(
            contents,
            "prefix_rule(pattern=[\"echo\", \"Hello, world!\"], decision=\"allow\")\n"
        );
    }

    #[test]
    fn appends_rule_without_duplicate_newline() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");
        std::fs::create_dir_all(policy_path.parent().unwrap()).expect("create policy dir");
        std::fs::write(
            &policy_path,
            "prefix_rule(pattern=[\"ls\"], decision=\"allow\")\n",
        )
        .expect("write seed rule");

        blocking_append_allow_prefix_rule(
            &policy_path,
            &[String::from("echo"), String::from("Hello, world!")],
        )
        .expect("append rule");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "prefix_rule(pattern=[\"ls\"], decision=\"allow\")\n\
             prefix_rule(pattern=[\"echo\", \"Hello, world!\"], decision=\"allow\")\n"
        );
    }

    #[test]
    fn inserts_newline_when_missing_before_append() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");
        std::fs::create_dir_all(policy_path.parent().unwrap()).expect("create policy dir");
        std::fs::write(
            &policy_path,
            "prefix_rule(pattern=[\"ls\"], decision=\"allow\")",
        )
        .expect("write seed rule without newline");

        blocking_append_allow_prefix_rule(
            &policy_path,
            &[String::from("echo"), String::from("Hello, world!")],
        )
        .expect("append rule");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "prefix_rule(pattern=[\"ls\"], decision=\"allow\")\n\
             prefix_rule(pattern=[\"echo\", \"Hello, world!\"], decision=\"allow\")\n"
        );
    }

    #[test]
    fn deduplicates_identical_rules() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");

        blocking_append_allow_prefix_rule(&policy_path, &[String::from("ls")])
            .expect("first append");
        blocking_append_allow_prefix_rule(&policy_path, &[String::from("ls")])
            .expect("duplicate append");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "prefix_rule(pattern=[\"ls\"], decision=\"allow\")\n"
        );
    }

    #[test]
    fn rejects_empty_prefix() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");
        let err = blocking_append_allow_prefix_rule(&policy_path, &[])
            .expect_err("empty prefix should be rejected");
        assert_eq!(err.to_string(), "prefix rule requires at least one token");
    }

    #[test]
    fn appends_network_rule() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");

        blocking_append_network_rule(
            &policy_path,
            "Api.GitHub.com",
            &NetworkRuleProtocol::Https,
            AmendDecision::Allow,
            Some("Allow https access to api.github.com"),
        )
        .expect("append network rule");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "network_rule(host=\"api.github.com\", protocol=\"https\", \
             decision=\"allow\", justification=\"Allow https access to api.github.com\")\n"
        );
    }

    #[test]
    fn appends_prefix_and_network_rules() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");

        blocking_append_allow_prefix_rule(&policy_path, &[String::from("curl")])
            .expect("append prefix rule");
        blocking_append_network_rule(
            &policy_path,
            "api.github.com",
            &NetworkRuleProtocol::Https,
            AmendDecision::Allow,
            Some("Allow https access to api.github.com"),
        )
        .expect("append network rule");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "prefix_rule(pattern=[\"curl\"], decision=\"allow\")\n\
             network_rule(host=\"api.github.com\", protocol=\"https\", \
             decision=\"allow\", justification=\"Allow https access to api.github.com\")\n"
        );
    }

    #[test]
    fn rejects_wildcard_network_rule_host() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");
        let err = blocking_append_network_rule(
            &policy_path,
            "*.example.com",
            &NetworkRuleProtocol::Https,
            AmendDecision::Allow,
            None,
        )
        .expect_err("wildcards should be rejected");
        assert!(
            err.to_string().contains("invalid network rule:"),
            "error should mention invalid network rule: {}",
            err
        );
    }

    #[test]
    fn rejects_empty_justification() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");
        let err = blocking_append_network_rule(
            &policy_path,
            "example.com",
            &NetworkRuleProtocol::Https,
            AmendDecision::Allow,
            Some("  "),
        )
        .expect_err("empty justification should be rejected");
        assert!(err.to_string().contains("justification cannot be empty"));
    }

    #[test]
    fn network_rule_without_justification() {
        let tmp = tempdir().expect("create temp dir");
        let policy_path = tmp.path().join("rules").join("default.rules");

        blocking_append_network_rule(
            &policy_path,
            "example.com",
            &NetworkRuleProtocol::Any,
            AmendDecision::Deny,
            None,
        )
        .expect("append rule without justification");

        let contents = std::fs::read_to_string(&policy_path).expect("read policy");
        assert_eq!(
            contents,
            "network_rule(host=\"example.com\", protocol=\"*\", decision=\"deny\")\n"
        );
    }
}
