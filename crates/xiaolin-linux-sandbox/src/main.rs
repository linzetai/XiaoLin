#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("xiaolin-linux-sandbox is only supported on Linux");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    if let Err(e) = xiaolin_linux_sandbox::run_main() {
        eprintln!("xiaolin-linux-sandbox: {e:#}");
        std::process::exit(1);
    }
}
