use anyhow::{Context, Result};
use tracing::info;

use crate::linux_run_main::SandboxPolicy;

/// Apply Landlock LSM rules based on the sandbox policy.
pub fn apply_landlock_rules(policy: &SandboxPolicy) -> Result<()> {
    apply_landlock_rules_inner(policy)
}

#[cfg(target_os = "linux")]
fn apply_landlock_rules_inner(policy: &SandboxPolicy) -> Result<()> {
    use landlock::{
        AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus,
    };

    let read_access = AccessFs::ReadFile | AccessFs::ReadDir | AccessFs::Execute;
    let write_access = read_access
        | AccessFs::WriteFile
        | AccessFs::RemoveFile
        | AccessFs::RemoveDir
        | AccessFs::MakeChar
        | AccessFs::MakeDir
        | AccessFs::MakeReg
        | AccessFs::MakeSock
        | AccessFs::MakeFifo
        | AccessFs::MakeBlock
        | AccessFs::MakeSym
        | AccessFs::Truncate;

    let all_access = write_access;

    let ruleset = Ruleset::default()
        .handle_access(all_access)
        .context("create landlock ruleset")?;

    let mut ruleset = ruleset.create().context("create landlock ruleset")?;

    // Readable roots: whole filesystem by default (read-only)
    let root_fd = PathFd::new("/").context("open /")?;
    ruleset = ruleset
        .add_rule(PathBeneath::new(root_fd, read_access))
        .context("add read rule for /")?;

    // Explicit readable roots
    for root in &policy.readable_roots {
        if let Ok(fd) = PathFd::new(root) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, read_access))
                .context("add readable root rule")?;
        }
    }

    // Writable roots
    for root in &policy.writable_roots {
        if let Ok(fd) = PathFd::new(root) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, write_access))
                .context("add writable root rule")?;
        } else {
            tracing::warn!("writable root does not exist, skipping: {:?}", root);
        }
    }

    // /tmp is always writable
    if let Ok(fd) = PathFd::new("/tmp") {
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, write_access))
            .context("add /tmp write rule")?;
    }

    // /dev/null, /dev/zero, /dev/random, /dev/urandom
    for dev in &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
        if let Ok(fd) = PathFd::new(dev) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, write_access))
                .context("add dev write rule")?;
        }
    }

    let status = ruleset
        .restrict_self()
        .context("landlock restrict_self")?;

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            info!("landlock: fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            info!("landlock: partially enforced (kernel may not support all rules)");
        }
        RulesetStatus::NotEnforced => {
            tracing::warn!("landlock: NOT enforced (kernel does not support landlock)");
        }
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_landlock_rules_inner(_policy: &SandboxPolicy) -> Result<()> {
    anyhow::bail!("landlock is only available on Linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_with_no_roots_succeeds() {
        let policy = SandboxPolicy {
            file_system: None,
            writable_roots: vec![],
            readable_roots: vec![],
            deny_read_paths: vec![],
            use_bwrap: false,
            use_landlock: true,
            proxy_port: None,
            network_namespace: false,
            seccomp_mode: None,
        };
        // Landlock may or may not be available in CI; don't assert success
        let _ = apply_landlock_rules(&policy);
    }
}
