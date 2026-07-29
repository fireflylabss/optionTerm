//! optionTerm AI — GTK4 + libadwaita terminal on libghostty-vt.

mod app;
mod config;
mod default_terminal;
mod graphics;
mod input;
mod profile;
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

    let _ = libghostty_vt::set_logger(Some(Box::new(libghostty_vt::log::TracingLogger)));

    app::run()
}
