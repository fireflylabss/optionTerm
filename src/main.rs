//! optionTerm — sidebar-first GTK4 + libadwaita terminal.

mod agents;
mod app;
mod browser;
mod codex;
mod config;
mod crash;
mod default_terminal;
mod keys;
mod launch;
mod pty;
mod session;
mod storage;
mod terminal;
mod tree;
mod ui;

fn main() -> anyhow::Result<()> {
    // Capture panics into ~/.option/terminal/crash.log before anything else,
    // so a crash during startup is still recorded.
    crash::install();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    app::run()
}
