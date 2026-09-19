//! `optionterm-next`: GPUI frontend for optionTerm. Startup only — all
//! application logic lives in `option_term_gpui::app`.

use option_term_core::config::Config;
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("option_term_gpui=info")),
        )
        .init();
    option_term_core::crash::install();

    let config = Config::load().unwrap_or_else(|err| {
        tracing::warn!("failed to load config, using defaults: {err:#}");
        Config::default()
    });

    gpui_platform::application().run(move |cx| option_term_gpui::app::start(cx, config));
}
