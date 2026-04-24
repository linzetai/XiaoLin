use fastclaw_core::config::DangerousOpsPolicy;
use std::sync::RwLock;

/// Runtime state for the dangerous-ops policy.
/// Updated via hot-reload when config changes.
struct DangerousOpsState {
    policy: DangerousOpsPolicy,
    patterns: Vec<(regex::Regex, String)>,
}

static STATE: RwLock<Option<DangerousOpsState>> = RwLock::new(None);

/// Initialize or hot-reload the dangerous-ops policy and compiled patterns.
pub fn set_dangerous_ops_config(policy: DangerousOpsPolicy, raw_patterns: &[String]) {
    let compiled: Vec<(regex::Regex, String)> = raw_patterns
        .iter()
        .filter_map(|p| {
            regex::Regex::new(p)
                .map(|re| (re, p.clone()))
                .map_err(|e| tracing::warn!(pattern = %p, error = %e, "ignoring invalid dangerous_pattern regex"))
                .ok()
        })
        .collect();

    if let Ok(mut guard) = STATE.write() {
        *guard = Some(DangerousOpsState {
            policy,
            patterns: compiled,
        });
    }
}

pub fn get_dangerous_ops_policy() -> DangerousOpsPolicy {
    STATE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.policy))
        .unwrap_or_default()
}

/// Check a shell command against the dangerous-ops policy.
///
/// Returns:
/// - `Ok(())` if the command is safe or the policy is `allow`.
/// - `Err(CheckResult::Denied(msg))` if policy is `deny` and a pattern matched.
/// - `Err(CheckResult::NeedsConfirmation(msg))` if policy is `confirm` and a pattern matched.
pub fn check_dangerous_command(command: &str) -> Result<(), CheckResult> {
    let guard = match STATE.read() {
        Ok(g) => g,
        Err(_) => return Ok(()),
    };
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return Ok(()),
    };

    match state.policy {
        DangerousOpsPolicy::Allow => Ok(()),
        DangerousOpsPolicy::Deny | DangerousOpsPolicy::Confirm => {
            let trimmed = command.trim();
            for (re, pattern) in &state.patterns {
                if re.is_match(trimmed) {
                    let msg = format!(
                        "Dangerous operation detected (matched pattern '{pattern}'): {trimmed}"
                    );
                    return Err(if state.policy == DangerousOpsPolicy::Deny {
                        CheckResult::Denied(msg)
                    } else {
                        CheckResult::NeedsConfirmation(msg)
                    });
                }
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub enum CheckResult {
    Denied(String),
    NeedsConfirmation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_test_state(policy: DangerousOpsPolicy) {
        let patterns = vec![
            r"\brm\s".to_string(),
            r"\brm$".to_string(),
            r"\brmdir\b".to_string(),
            r"\bchmod\b".to_string(),
        ];
        set_dangerous_ops_config(policy, &patterns);
    }

    #[test]
    fn dangerous_ops_policy_all_modes() {
        // Allow mode: everything passes
        init_test_state(DangerousOpsPolicy::Allow);
        assert!(check_dangerous_command("rm -rf /").is_ok());
        assert!(check_dangerous_command("rmdir foo").is_ok());

        // Deny mode: dangerous commands blocked
        init_test_state(DangerousOpsPolicy::Deny);
        match check_dangerous_command("rm -rf /tmp/stuff") {
            Err(CheckResult::Denied(msg)) => assert!(msg.contains("rm")),
            other => panic!("expected Denied, got {other:?}"),
        }
        assert!(check_dangerous_command("ls -la").is_ok());
        assert!(check_dangerous_command("echo hello").is_ok());

        // Confirm mode: dangerous commands return NeedsConfirmation
        init_test_state(DangerousOpsPolicy::Confirm);
        match check_dangerous_command("rm file.txt") {
            Err(CheckResult::NeedsConfirmation(msg)) => assert!(msg.contains("rm")),
            other => panic!("expected NeedsConfirmation, got {other:?}"),
        }
        match check_dangerous_command("chmod 777 /etc/passwd") {
            Err(CheckResult::NeedsConfirmation(msg)) => assert!(msg.contains("chmod")),
            other => panic!("expected NeedsConfirmation, got {other:?}"),
        }
        assert!(check_dangerous_command("cat /etc/passwd").is_ok());

        // Safe commands pass in all modes
        for policy in [DangerousOpsPolicy::Deny, DangerousOpsPolicy::Confirm, DangerousOpsPolicy::Allow] {
            init_test_state(policy);
            assert!(check_dangerous_command("ls -la").is_ok());
            assert!(check_dangerous_command("git status").is_ok());
            assert!(check_dangerous_command("cargo build").is_ok());
            assert!(check_dangerous_command("grep -r pattern .").is_ok());
        }

        // rmdir detection
        init_test_state(DangerousOpsPolicy::Deny);
        assert!(check_dangerous_command("rmdir /tmp/old").is_err());
        assert!(check_dangerous_command("mkdir new_dir").is_ok());
    }
}
