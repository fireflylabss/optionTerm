//! optionTerm — sidebar-first GTK4 + libadwaita terminal.

mod app;
mod config;
mod default_terminal;
mod keys;
mod launch;
mod pty;
mod session;
mod terminal;
mod ui;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    app::run()
}
