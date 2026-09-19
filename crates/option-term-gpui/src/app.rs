//! Application wiring: key bindings and the root window/entity.

use gpui::{
    App, Bounds, Context, Entity, KeyBinding, Render, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowKind, WindowOptions, div, prelude::*, px, size,
};
use option_term_core::config::Config;

use crate::actions::{
    Copy, Paste, Quit, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    ZoomIn, ZoomOut, ZoomReset,
};
use crate::pane::Pane;

/// Key context under which terminal key bindings are registered.
pub const PANE_CONTEXT: &str = "Pane";

/// Register the fixed key bindings for phase 2a. Phase 2c replaces these with
/// `keys.toml` overrides.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(vec![
        KeyBinding::new("ctrl-shift-c", Copy, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-shift-v", Paste, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-shift-a", SelectAll, Some(PANE_CONTEXT)),
        KeyBinding::new("shift-pageup", ScrollPageUp, Some(PANE_CONTEXT)),
        KeyBinding::new("shift-pagedown", ScrollPageDown, Some(PANE_CONTEXT)),
        KeyBinding::new("shift-home", ScrollToTop, Some(PANE_CONTEXT)),
        KeyBinding::new("shift-end", ScrollToBottom, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-=", ZoomIn, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-+", ZoomIn, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl--", ZoomOut, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-0", ZoomReset, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-shift-q", Quit, Some(PANE_CONTEXT)),
    ]);
}

/// Application entry inside `gpui_platform::application().run`.
pub fn start(cx: &mut App, config: Config) {
    bind_keys(cx);
    open_main_window(cx, &config);
}

/// Root view: fills the window with the single terminal pane.
pub struct Root {
    pane: Entity<Pane>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone())
    }
}

pub fn open_main_window(cx: &mut App, config: &Config) {
    let bounds = Bounds::centered(None, size(px(1000.), px(640.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Server),
        app_id: Some("optionterm".to_string()),
        kind: WindowKind::Normal,
        focus: true,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    let config = config.clone();
    if let Err(err) = cx.open_window(options, |window, cx| {
        let config = config.clone();
        let pane = cx.new(|cx| match Pane::new(&config, window, cx) {
            Ok(pane) => pane,
            Err(err) => {
                tracing::error!("failed to spawn terminal pane: {err:#}");
                Pane::without_thread(&config, cx)
            }
        });
        cx.new(|_| Root { pane })
    }) {
        tracing::error!("failed to open window: {err}");
    }
}
