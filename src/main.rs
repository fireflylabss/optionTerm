//! optionTerm — sidebar-first GTK4 + libadwaita terminal.

mod agents;
mod app;
mod browser;
mod config_watch;
mod keys_gtk;
mod terminal;
#[cfg(test)]
mod test_support;
mod tree;
mod ui;
mod verification;

#[cfg(test)]
use option_term_core::storage;
use option_term_core::{codex, config, crash, default_terminal, keys, launch, pty, session};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args_os()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let cwd = std::env::current_dir().or_else(|_| {
        dirs::home_dir().ok_or_else(|| std::io::Error::other("no working directory"))
    })?;
    match launch::parse_command_line(&args, &cwd)? {
        launch::CommandLine::Help => {
            print!("{}", launch::HELP);
            return Ok(());
        }
        launch::CommandLine::Version => {
            println!("optionterm {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        launch::CommandLine::SelfTest => return verification::run(),
        launch::CommandLine::Launch(_) => {}
    }
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
