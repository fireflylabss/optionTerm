//! Session persistence: remember open tabs, split trees and window size.
//!
//! The shape of the workspace is stored in `session.toml` (tabs, nested split
//! layout with divider ratios, cwd per leaf, custom titles, window geometry).
//! Scrollback content is not restored.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Orientation of a split pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitOrientation {
    Horizontal,
    Vertical,
}

impl SplitOrientation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "vertical" => Self::Vertical,
            _ => Self::Horizontal,
        }
    }
}

/// Nested split tree for one tab.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneLayout {
    Leaf {
        cwd: Option<String>,
    },
    Split {
        orientation: SplitOrientation,
        /// Divider position as a fraction of the paned size (0..=1).
        ratio: f64,
        start: Box<PaneLayout>,
        end: Box<PaneLayout>,
    },
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self::Leaf { cwd: None }
    }
}

impl PaneLayout {
    /// Flat list of leaf cwds in left-to-right / top-to-bottom creation order.
    pub fn leaf_cwds(&self) -> Vec<Option<String>> {
        match self {
            Self::Leaf { cwd } => vec![cwd.clone()],
            Self::Split { start, end, .. } => {
                let mut out = start.leaf_cwds();
                out.extend(end.leaf_cwds());
                out
            }
        }
    }

    /// Build a left-biased horizontal chain (legacy restore path).
    pub fn from_flat_panes(panes: Vec<Option<String>>) -> Self {
        let mut iter = panes.into_iter();
        let Some(first) = iter.next() else {
            return Self::Leaf { cwd: None };
        };
        let mut node = Self::Leaf { cwd: first };
        for cwd in iter {
            node = Self::Split {
                orientation: SplitOrientation::Horizontal,
                ratio: 0.5,
                start: Box::new(node),
                end: Box::new(Self::Leaf { cwd }),
            };
        }
        node
    }
}

/// What kind of page a restored tab holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum TabKind {
    /// One or more terminal panes in a `PaneLayout`.
    #[default]
    Terminal,
    /// A single embedded browser page, optionally restoring its URL.
    Browser { url: Option<String> },
}

/// One restored tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabState {
    /// Custom title, if the user renamed the tab.
    pub title: Option<String>,
    /// Nested split tree (preferred for `TabKind::Terminal`).
    pub layout: PaneLayout,
    /// Page kind (terminal vs browser). Defaults to `Terminal`.
    pub kind: TabKind,
}

impl TabState {
    /// Legacy accessor: leaf cwds in tree order (empty for browsers).
    pub fn panes(&self) -> Vec<Option<String>> {
        if matches!(self.kind, TabKind::Browser { .. }) {
            Vec::new()
        } else {
            self.layout.leaf_cwds()
        }
    }
}

/// The whole restorable workspace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub tabs: Vec<TabState>,
    pub active: usize,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub maximized: bool,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// `~/.option/terminal/session.toml`
    pub fn path() -> PathBuf {
        option_sdk::App::TERMINAL.session_toml()
    }

    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        match Self::parse(&text) {
            Ok(session) => Some(session).filter(|s| !s.is_empty()),
            Err(err) => {
                tracing::warn!(
                    "session could not be restored; original file will be preserved: {err:#}"
                );
                None
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let text = self.to_toml();
        Self::parse(&text).context("validating session before saving")?;
        match std::fs::read_to_string(path) {
            Ok(existing) => {
                Self::parse(&existing).context(
                    "preserving invalid session; recover or move it before saving a replacement",
                )?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("reading existing session before replacing it"),
        }
        crate::storage::atomic_write(path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Remove a stored session (used when restore is turned off).
    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
        // Drop leftover VT dumps from ≤0.1.x installs.
        Self::clear_legacy_scrollback();
    }

    fn scrollback_dir() -> PathBuf {
        Self::path()
            .parent()
            .map(|p| p.join("scrollback"))
            .unwrap_or_else(|| PathBuf::from("scrollback"))
    }

    /// Remove leftover scrollback dumps from older releases.
    pub fn clear_legacy_scrollback() {
        let _ = std::fs::remove_dir_all(Self::scrollback_dir());
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::from("# optionTerm session — regenerated on exit.\n");
        out.push_str(&format!("active = {}\n", self.active));
        if let Some(w) = self.width {
            out.push_str(&format!("width = {w}\n"));
        }
        if let Some(h) = self.height {
            out.push_str(&format!("height = {h}\n"));
        }
        if self.maximized {
            out.push_str("maximized = true\n");
        }
        for tab in &self.tabs {
            out.push_str("\n[[tab]]\n");
            if let Some(title) = &tab.title {
                out.push_str(&format!("title = {}\n", quote(title)));
            }
            match &tab.kind {
                TabKind::Terminal => {
                    // Keep a flat `panes` list for older readers / debugging.
                    let panes: Vec<String> = tab
                        .panes()
                        .iter()
                        .map(|p| quote(p.as_deref().unwrap_or("")))
                        .collect();
                    out.push_str(&format!("panes = [{}]\n", panes.join(", ")));
                    // `[tab.layout]` attaches to the preceding `[[tab]]` array element.
                    write_layout(&mut out, "tab.layout", &tab.layout);
                }
                TabKind::Browser { url } => {
                    out.push_str("kind = \"browser\"\n");
                    if let Some(url) = url {
                        out.push_str(&format!("url = {}\n", quote(url)));
                    }
                }
            }
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self> {
        anyhow::ensure!(text.len() <= 4 * 1024 * 1024, "session exceeds size limit");
        let table: toml::Table = text.parse().context("parsing session.toml")?;
        let mut remaining_panes = 1024usize;
        let active = table
            .get("active")
            .and_then(|v| v.as_integer())
            .unwrap_or(0)
            .max(0) as usize;
        let width = table
            .get("width")
            .and_then(|v| v.as_integer())
            .and_then(|w| i32::try_from(w).ok())
            .filter(|&w| w > 0);
        let height = table
            .get("height")
            .and_then(|v| v.as_integer())
            .and_then(|h| i32::try_from(h).ok())
            .filter(|&h| h > 0);
        let maximized = table
            .get("maximized")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut tabs = Vec::new();
        if let Some(items) = table.get("tab").and_then(|v| v.as_array()) {
            anyhow::ensure!(items.len() <= 128, "session exceeds tab limit");
            for item in items {
                let Some(t) = item.as_table() else { continue };
                let title = t
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty());
                let kind = match t.get("kind").and_then(|v| v.as_str()) {
                    Some("browser") => {
                        let url = t
                            .get("url")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        TabKind::Browser { url }
                    }
                    _ => TabKind::Terminal,
                };
                let layout = if matches!(kind, TabKind::Browser { .. }) {
                    PaneLayout::default()
                } else if let Some(layout_tbl) = t.get("layout").and_then(|v| v.as_table()) {
                    validate_layout(Some(layout_tbl), 0, &mut remaining_panes)?;
                    parse_layout(layout_tbl)
                } else {
                    let panes = t
                        .get("panes")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                        .filter(|p: &Vec<_>| !p.is_empty())
                        .unwrap_or_else(|| vec![None]);
                    anyhow::ensure!(
                        panes.len() <= 64 && panes.len() <= remaining_panes,
                        "session exceeds pane limit"
                    );
                    remaining_panes -= panes.len();
                    PaneLayout::from_flat_panes(panes)
                };
                tabs.push(TabState {
                    title,
                    layout,
                    kind,
                });
            }
        }
        let active = active.min(tabs.len().saturating_sub(1));
        Ok(Self {
            tabs,
            active,
            width,
            height,
            maximized,
        })
    }
}

fn write_layout(out: &mut String, key: &str, layout: &PaneLayout) {
    match layout {
        PaneLayout::Leaf { cwd } => {
            out.push_str(&format!("[{key}]\n"));
            out.push_str("kind = \"leaf\"\n");
            if let Some(cwd) = cwd {
                out.push_str(&format!("cwd = {}\n", quote(cwd)));
            }
        }
        PaneLayout::Split {
            orientation,
            ratio,
            start,
            end,
        } => {
            out.push_str(&format!("[{key}]\n"));
            out.push_str("kind = \"split\"\n");
            out.push_str(&format!("orientation = \"{}\"\n", orientation.as_str()));
            out.push_str(&format!("ratio = {ratio:.4}\n"));
            write_layout(out, &format!("{key}.start"), start);
            write_layout(out, &format!("{key}.end"), end);
        }
    }
}

fn validate_layout(table: Option<&toml::Table>, depth: usize, remaining: &mut usize) -> Result<()> {
    anyhow::ensure!(depth <= 64, "session exceeds split depth limit");
    if let Some(table) = table
        && table.get("kind").and_then(|v| v.as_str()) == Some("split")
    {
        validate_layout(
            table.get("start").and_then(|v| v.as_table()),
            depth + 1,
            remaining,
        )?;
        validate_layout(
            table.get("end").and_then(|v| v.as_table()),
            depth + 1,
            remaining,
        )?;
    } else {
        anyhow::ensure!(*remaining > 0, "session exceeds pane limit");
        *remaining -= 1;
    }
    Ok(())
}

fn parse_layout(table: &toml::Table) -> PaneLayout {
    let kind = table.get("kind").and_then(|v| v.as_str()).unwrap_or("leaf");
    if kind == "split" {
        let orientation = table
            .get("orientation")
            .and_then(|v| v.as_str())
            .map(SplitOrientation::parse)
            .unwrap_or(SplitOrientation::Horizontal);
        let ratio = table
            .get("ratio")
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .filter(|ratio| ratio.is_finite())
            .unwrap_or(0.5)
            .clamp(0.05, 0.95);
        let start = table
            .get("start")
            .and_then(|v| v.as_table())
            .map(parse_layout)
            .unwrap_or_default();
        let end = table
            .get("end")
            .and_then(|v| v.as_table())
            .map(parse_layout)
            .unwrap_or_default();
        PaneLayout::Split {
            orientation,
            ratio,
            start: Box::new(start),
            end: Box::new(end),
        }
    } else {
        let cwd = table
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        PaneLayout::Leaf { cwd }
    }
}

fn quote(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_session_is_not_overwritten() {
        let dir = crate::test_support::TestDir::new("invalid-session");
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "broken = [").unwrap();
        assert!(sample().save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken = [");
    }

    #[test]
    fn nonfinite_ratios_and_overflowing_dimensions_are_safe() {
        let session = Session::parse("width = 4294967297\nheight = 9223372036854775807\n[[tab]]\n[tab.layout]\nkind = 'split'\nratio = nan\n").unwrap();
        assert_eq!(session.width, None);
        assert_eq!(session.height, None);
        assert!(matches!(
            session.tabs[0].layout,
            PaneLayout::Split { ratio: 0.5, .. }
        ));
    }

    #[test]
    fn rejects_excessive_tabs_and_legacy_panes() {
        assert!(Session::parse(&"[[tab]]\n".repeat(129)).is_err());
        let panes = vec!["''"; 65].join(",");
        assert!(Session::parse(&format!("[[tab]]\npanes = [{panes}]\n")).is_err());
    }

    #[test]
    fn control_characters_round_trip() {
        let mut session = sample();
        session.tabs[0].title = Some("a\r\t\u{0001}\\\"b".into());
        assert_eq!(Session::parse(&session.to_toml()).unwrap(), session);
    }

    fn sample() -> Session {
        Session {
            tabs: vec![
                TabState {
                    title: Some("build".into()),
                    layout: PaneLayout::Split {
                        orientation: SplitOrientation::Horizontal,
                        ratio: 0.4,
                        start: Box::new(PaneLayout::Leaf {
                            cwd: Some("/home/u/proj".into()),
                        }),
                        end: Box::new(PaneLayout::Leaf {
                            cwd: Some("/tmp".into()),
                        }),
                    },
                    ..Default::default()
                },
                TabState {
                    title: None,
                    layout: PaneLayout::Leaf { cwd: None },
                    ..Default::default()
                },
            ],
            active: 1,
            width: Some(1200),
            height: Some(800),
            maximized: false,
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let session = sample();
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }

    #[test]
    fn escapes_awkward_paths() {
        let session = Session {
            tabs: vec![TabState {
                title: Some("say \"hi\"".into()),
                layout: PaneLayout::Leaf {
                    cwd: Some(r#"/tmp/we"ird\path"#.into()),
                },
                ..Default::default()
            }],
            active: 0,
            width: None,
            height: None,
            maximized: true,
        };
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }

    #[test]
    fn defaults_to_a_single_pane_when_missing() {
        let parsed = Session::parse("active = 0\n\n[[tab]]\n").expect("parse");
        assert_eq!(parsed.tabs.len(), 1);
        assert_eq!(parsed.tabs[0].panes(), vec![None]);
    }

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
    fn legacy_flat_panes_become_horizontal_chain() {
        let parsed = Session::parse("active = 0\n\n[[tab]]\npanes = [\"/a\", \"/b\", \"/c\"]\n")
            .expect("parse");
        assert_eq!(
            parsed.tabs[0].panes(),
            vec![Some("/a".into()), Some("/b".into()), Some("/c".into()),]
        );
        match &parsed.tabs[0].layout {
            PaneLayout::Split {
                orientation: SplitOrientation::Horizontal,
                ..
            } => {}
            other => panic!("expected horizontal chain, got {other:?}"),
        }
    }

    #[test]
    fn browser_tab_round_trips() {
        let session = Session {
            tabs: vec![
                TabState {
                    title: Some("Docs".into()),
                    layout: PaneLayout::default(),
                    kind: TabKind::Browser {
                        url: Some("https://docs.rust-lang.org".into()),
                    },
                },
                TabState {
                    title: None,
                    layout: PaneLayout::default(),
                    kind: TabKind::Browser { url: None },
                },
            ],
            active: 0,
            width: None,
            height: None,
            maximized: false,
        };
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }

    #[test]
    fn browser_tab_ignores_panes_and_layout() {
        let parsed = Session::parse(
            "active = 0\n\n[[tab]]\nkind = \"browser\"\nurl = \"https://a.b\"\n\n[[tab]]\npanes = [\"/w\"]\n",
        )
        .expect("parse");
        assert_eq!(parsed.tabs.len(), 2);
        assert_eq!(
            parsed.tabs[0].kind,
            TabKind::Browser {
                url: Some("https://a.b".into())
            }
        );
        assert_eq!(parsed.tabs[0].panes(), Vec::<Option<String>>::new());
        assert_eq!(parsed.tabs[1].kind, TabKind::Terminal);
    }

    #[test]
    fn nested_vertical_split_round_trips() {
        let session = Session {
            tabs: vec![TabState {
                title: None,
                layout: PaneLayout::Split {
                    orientation: SplitOrientation::Vertical,
                    ratio: 0.33,
                    start: Box::new(PaneLayout::Leaf {
                        cwd: Some("/top".into()),
                    }),
                    end: Box::new(PaneLayout::Split {
                        orientation: SplitOrientation::Horizontal,
                        ratio: 0.6,
                        start: Box::new(PaneLayout::Leaf {
                            cwd: Some("/bl".into()),
                        }),
                        end: Box::new(PaneLayout::Leaf {
                            cwd: Some("/br".into()),
                        }),
                    }),
                },
                ..Default::default()
            }],
            active: 0,
            width: Some(900),
            height: Some(600),
            maximized: false,
        };
        let back = Session::parse(&session.to_toml()).expect("parse");
        assert_eq!(back, session);
    }
}
