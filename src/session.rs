//! Session persistence: remember open tabs and their working directories.
//!
//! The shape of the workspace is always stored in `session.toml` (tabs, how
//! many panes, cwd and custom titles). Scrollback is optional and opt-in:
//! when enabled it is written beside the session as VT dumps under
//! `scrollback/`, because it can hold secrets (tokens, passwords, …).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// One restored tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabState {
    /// Custom title, if the user renamed the tab.
    pub title: Option<String>,
    /// Working directory of each pane, in creation order. The length also
    /// tells us how many panes to recreate.
    pub panes: Vec<Option<String>>,
}

/// The whole restorable workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub tabs: Vec<TabState>,
    pub active: usize,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// `~/.option/terminal/session.toml`
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".option")
            .join("terminal")
            .join("session.toml")
    }

    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        Self::parse(&text).ok().filter(|s| !s.is_empty())
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, self.to_toml()).with_context(|| format!("writing {}", path.display()))
    }

    /// Remove a stored session (used when restore is turned off).
    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
        Self::clear_scrollback();
    }

    /// `~/.option/terminal/scrollback/`
    pub fn scrollback_dir() -> PathBuf {
        Self::path()
            .parent()
            .map(|p| p.join("scrollback"))
            .unwrap_or_else(|| PathBuf::from("scrollback"))
    }

    /// Drop every saved VT dump. Called when history restore is off, so a
    /// previous opt-in does not leave secrets on disk.
    pub fn clear_scrollback() {
        let _ = std::fs::remove_dir_all(Self::scrollback_dir());
    }

    pub fn scrollback_path(tab: usize, pane: usize) -> PathBuf {
        Self::scrollback_dir().join(format!("{tab}-{pane}.vt"))
    }

    /// Persist one pane's VT dump. Oversized dumps are skipped rather than
    /// truncated mid-sequence (a partial VT stream would corrupt the restore).
    pub fn save_scrollback(tab: usize, pane: usize, data: &[u8]) -> Result<()> {
        const MAX_BYTES: usize = 4 * 1024 * 1024;
        if data.is_empty() {
            return Ok(());
        }
        if data.len() > MAX_BYTES {
            tracing::warn!(
                "scrollback for tab {tab} pane {pane} is {} bytes; skipping",
                data.len()
            );
            return Ok(());
        }
        let dir = Self::scrollback_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = Self::scrollback_path(tab, pane);
        std::fs::write(&path, data).with_context(|| format!("writing {}", path.display()))
    }

    pub fn load_scrollback(tab: usize, pane: usize) -> Option<Vec<u8>> {
        std::fs::read(Self::scrollback_path(tab, pane))
            .ok()
            .filter(|d| !d.is_empty())
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::from("# optionTerm session — regenerated on exit.\n");
        out.push_str(&format!("active = {}\n", self.active));
        for tab in &self.tabs {
            out.push_str("\n[[tab]]\n");
            if let Some(title) = &tab.title {
                out.push_str(&format!("title = {}\n", quote(title)));
            }
            let panes: Vec<String> = tab
                .panes
                .iter()
                .map(|p| quote(p.as_deref().unwrap_or("")))
                .collect();
            out.push_str(&format!("panes = [{}]\n", panes.join(", ")));
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text.parse().context("parsing session.toml")?;
        let active = table
            .get("active")
            .and_then(|v| v.as_integer())
            .unwrap_or(0)
            .max(0) as usize;

        let mut tabs = Vec::new();
        if let Some(items) = table.get("tab").and_then(|v| v.as_array()) {
            for item in items {
                let Some(t) = item.as_table() else { continue };
                let title = t
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty());
                let panes = t
                    .get("panes")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    // A tab always has at least one pane.
                    .filter(|p: &Vec<_>| !p.is_empty())
                    .unwrap_or_else(|| vec![None]);
                tabs.push(TabState { title, panes });
            }
        }
        let active = active.min(tabs.len().saturating_sub(1));
        Ok(Self { tabs, active })
    }
}

/// Minimal TOML basic-string quoting: paths can contain quotes and backslashes.
fn quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            tabs: vec![
                TabState {
                    title: Some("build".into()),
                    panes: vec![Some("/home/u/proj".into()), Some("/tmp".into())],
                },
                TabState {
                    title: None,
                    panes: vec![None],
                },
            ],
            active: 1,
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let session = sample();
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }

    /// Paths with quotes or backslashes must not produce invalid TOML.
    #[test]
    fn escapes_awkward_paths() {
        let session = Session {
            tabs: vec![TabState {
                title: Some("say \"hi\"".into()),
                panes: vec![Some(r#"/tmp/we"ird\path"#.into())],
            }],
            active: 0,
        };
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }

    #[test]
    fn defaults_to_a_single_pane_when_missing() {
        let parsed = Session::parse("active = 0\n\n[[tab]]\n").expect("parse");
        assert_eq!(parsed.tabs.len(), 1);
        assert_eq!(parsed.tabs[0].panes, vec![None]);
    }

    /// A stale `active` index must not point past the end of the tab list.
    #[test]
    fn clamps_active_index() {
        let parsed = Session::parse("active = 9\n\n[[tab]]\npanes = [\"\"]\n").expect("parse");
        assert_eq!(parsed.active, 0);
    }

    #[test]
    fn empty_session_is_ignored() {
        assert!(Session::parse("active = 0\n").unwrap().is_empty());
    }

    #[test]
    fn scrollback_paths_are_stable() {
        let path = Session::scrollback_path(2, 1);
        assert!(path.ends_with("scrollback/2-1.vt"));
    }
}
