use anyhow::{Context, Result};
use std::ffi::CString;
use std::path::Path;

/// Launch a child process with optional sandbox environment.
pub fn launch_child(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> Result<i32> {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::{ForkResult, fork};

    let result = unsafe { fork() };
    match result.context("fork failed")? {
        ForkResult::Parent { child } => {
            let status = waitpid(child, None).context("waitpid failed")?;
            match status {
                WaitStatus::Exited(_, code) => Ok(code),
                WaitStatus::Signaled(_, sig, _) => Ok(128 + sig as i32),
                _ => Ok(1),
            }
        }
        ForkResult::Child => {
            if let Some(dir) = cwd {
                std::env::set_current_dir(dir).ok();
            }
            for (key, val) in env {
                unsafe { std::env::set_var(key, val) };
            }

            let c_program =
                CString::new(program.as_bytes()).expect("invalid program");
            let c_args: Vec<CString> = std::iter::once(c_program.clone())
                .chain(args.iter().map(|a| CString::new(a.as_bytes()).expect("invalid arg")))
                .collect();

            match nix::unistd::execvp(&c_program, &c_args) {
                Ok(inf) => match inf {},
                Err(e) => {
                    eprintln!("execvp failed: {e}");
                    std::process::exit(127);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_echo() {
        let result = super::launch_child(
            "/bin/echo",
            &["hello".to_string()],
            None,
            &[],
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn launch_false_returns_nonzero() {
        let result = super::launch_child("/bin/false", &[], None, &[]);
        assert!(result.is_ok());
        assert_ne!(result.unwrap(), 0);
    }
}
