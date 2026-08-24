//! Agent integrations: detect installed coding agents (Codex, Claude, OpenCode,
//! Cursor, Devin, Grok) and launch each one in a dedicated terminal tab.
//!
//! optionTerm does not read agent session stores; it just knows the binary name
//! per agent, whether it is installed, its real logo asset, and how to start a
//! fresh session. The agent takes over the tab and manages its own conversation.
//!
//! The logos shipped with Zed use `fill="currentColor"`, which stays dark (or
//! vanishes) on a dark theme. We derive a pure-white copy (`<name>-white.svg`)
//! into the app cache so a dark theme can show a legible white logo.

use std::{path::Path, process::Command};

use option_sdk::App;

/// The coding agents optionTerm knows about, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Codex,
    Claude,
    OpenCode,
    Cursor,
    Devin,
    Grok,
}

impl AgentKind {
    pub const ALL: [AgentKind; 6] = [
        AgentKind::Codex,
        AgentKind::Claude,
        AgentKind::OpenCode,
        AgentKind::Cursor,
        AgentKind::Devin,
        AgentKind::Grok,
    ];

    /// The binary name optionTerm runs for this agent.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::Claude => "claude",
            AgentKind::OpenCode => "opencode",
            AgentKind::Cursor => "cursor",
            AgentKind::Devin => "devin",
            AgentKind::Grok => "grok",
        }
    }

    /// Human label, used for menu entries and tab titles.
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Codex => "Codex",
            AgentKind::Claude => "Claude Code",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Cursor => "Cursor",
            AgentKind::Devin => "Devin",
            AgentKind::Grok => "Grok",
        }
    }

    /// Icon-theme name for the agent (the logo filename without `.svg`), so an
    /// `adw::TabPage` icon resolves through `GtkIconTheme`.
    pub fn icon_name(self) -> &'static str {
        match self {
            AgentKind::Codex => "codex-acp",
            AgentKind::Claude => "claude-acp",
            AgentKind::OpenCode => "opencode",
            AgentKind::Cursor => "cursor",
            AgentKind::Devin => "devin",
            AgentKind::Grok => "grok-build",
        }
    }

    /// The derived pure-white logo (dark-theme variant) for this agent.
    pub fn white_icon_name(self) -> &'static str {
        match self {
            AgentKind::Codex => "codex-acp-white",
            AgentKind::Claude => "claude-acp-white",
            AgentKind::OpenCode => "opencode-white",
            AgentKind::Cursor => "cursor-white",
            AgentKind::Devin => "devin-white",
            AgentKind::Grok => "grok-build-white",
        }
    }

    /// Pick the icon name suited to the current color scheme: the white variant
    /// for dark themes, the original for light.
    pub fn theme_icon_name(self, dark: bool) -> &'static str {
        if dark {
            self.white_icon_name()
        } else {
            self.icon_name()
        }
    }
}

/// Register the agent logos directory with the GTK icon theme, so `ThemedIcon`
/// names (from [`AgentKind::icon_name`]) resolve to the real SVG logos. Returns
/// true if the directory exists and was added.
///
/// Also derives a pure-white copy of every logo into the app cache, registered
/// under the `-white` names, so dark themes can show a legible white glyph.
pub fn register_icons() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let dir = home.join(".local/share/zed/external_agents/registry/icons");
    if !dir.is_dir() {
        return false;
    }
    // Spin up the white variants before touching the icon theme, so both name
    // sets are resolvable together. Failure to derive them is not fatal: the
    // original logos still work on light themes.
    let cache = white_icons_dir();
    if let Some(cache) = cache.as_deref() {
        for kind in AgentKind::ALL {
            let _ = derive_white_logo(kind, &dir, cache);
        }
    }
    if let Some(display) = gtk4::gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        theme.add_search_path(&dir);
        if let Some(cache) = cache.as_deref() {
            theme.add_search_path(cache);
        }
        let ok = theme.has_icon("codex-acp");
        tracing::info!(
            "agent icons registered (dir={}, white={:?}, codex-acp has_icon={ok})",
            dir.display(),
            cache
        );
    } else {
        tracing::warn!("agent icons not registered: no default display");
    }
    true
}

/// The cache directory holding our derived white logos, per optionSDK. Created
/// on demand so a first launch can still populate it.
fn white_icons_dir() -> Option<std::path::PathBuf> {
    let icons = App::TERMINAL.cache_dir().join("icons");
    std::fs::create_dir_all(&icons).ok()?;
    Some(icons)
}

/// Read `<name>.svg` from `src`, replace every `currentColor` with pure white,
/// and write `<name>-white.svg` into `dst`. Returns the destination path on
/// success.
fn derive_white_logo(kind: AgentKind, src: &Path, dst: &Path) -> Option<std::path::PathBuf> {
    let base = kind.icon_name();
    let out_name = format!("{base}-white.svg");
    let out_path = dst.join(&out_name);
    // Skip when already derived and newer than the source, so a cached copy is
    // reused across launches.
    let src_path = src.join(format!("{base}.svg"));
    if out_path.exists() {
        let fresh = match (src_path.metadata(), out_path.metadata()) {
            (Ok(s), Ok(o)) => {
                o.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    >= s.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            }
            _ => false,
        };
        if fresh {
            return Some(out_path);
        }
    }
    let svg = std::fs::read_to_string(&src_path).ok()?;
    let white = svg.replace("currentColor", "#ffffff");
    std::fs::write(&out_path, white).ok()?;
    Some(out_path)
}

/// Whether the agent's binary is on `$PATH`.
pub fn is_installed(kind: AgentKind) -> bool {
    Command::new(kind.as_str())
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// The argv that starts a brand-new agent session.
pub fn new_command(kind: AgentKind) -> Vec<String> {
    vec![kind.as_str().to_string()]
}
