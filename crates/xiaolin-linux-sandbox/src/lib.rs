#[cfg(target_os = "linux")]
pub mod bwrap;
#[cfg(target_os = "linux")]
pub mod landlock_rules;
#[cfg(target_os = "linux")]
pub mod launcher;
#[cfg(target_os = "linux")]
pub mod linux_run_main;
#[cfg(target_os = "linux")]
pub mod proxy_routing;

#[cfg(target_os = "linux")]
pub use linux_run_main::run_main;

#[cfg(not(target_os = "linux"))]
pub fn run_main() -> anyhow::Result<()> {
    anyhow::bail!("xiaolin-linux-sandbox is only supported on Linux")
}
