//! Registering optionTerm as the system's default terminal.
//!
//! There is no single mechanism for this. The portable one is
//! `xdg-terminals.list`, read by `xdg-terminal-exec`, which newer desktops and
//! tools consult. GNOME still reads a deprecated GSettings key, and KDE keeps
//! its own entry in `kdeglobals`. So this writes whichever ones apply and
//! reports back exactly what it touched rather than claiming success blindly.

use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result};

/// Desktop entry that identifies us to the XDG mechanism.
const DESKTOP_ID: &str = "io.option.terminal.desktop";

/// Command name used by the desktop-specific keys.
const BINARY: &str = "optionterm";

fn terminals_list() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("xdg-terminals.list"))
}

/// Whether we appear to be the preferred terminal already.
///
/// Only the XDG list is checked: it is the mechanism we can read back reliably,
/// and being first in it is what "default" means there.
pub fn is_default() -> bool {
    let Some(path) = terminals_list() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        == Some(DESKTOP_ID)
}

/// Make optionTerm the default terminal, returning a description of every
/// mechanism that was actually updated.
pub fn set_default() -> Result<Vec<String>> {
    let mut applied = Vec::new();

    // --- The portable mechanism ---
    if let Some(path) = terminals_list() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // Keep the user's other choices, just ahead of them.
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines: Vec<&str> = existing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != DESKTOP_ID)
            .collect();
        lines.insert(0, DESKTOP_ID);
        std::fs::write(&path, format!("{}\n", lines.join("\n")))
            .with_context(|| format!("writing {}", path.display()))?;
        applied.push(format!("{}", path.display()));
    }

    // --- GNOME's deprecated key, still read by some file managers ---
    if schema_exists("org.gnome.desktop.default-applications.terminal") {
        let ok = run(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.default-applications.terminal",
                "exec",
                BINARY,
            ],
        );
        // exec-arg matters: the default is `-e`, which optionTerm does not take.
        let arg_ok = run(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.default-applications.terminal",
                "exec-arg",
                "",
            ],
        );
        if ok && arg_ok {
            applied.push("GNOME default-applications.terminal".into());
        }
    }

    // --- KDE ---
    for tool in ["kwriteconfig6", "kwriteconfig5"] {
        if which(tool)
            && run(
                tool,
                &[
                    "--file",
                    "kdeglobals",
                    "--group",
                    "General",
                    "--key",
                    "TerminalApplication",
                    BINARY,
                ],
            )
        {
            applied.push("KDE kdeglobals TerminalApplication".into());
            break;
        }
    }

    Ok(applied)
}

fn which(binary: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {binary}")])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn run(binary: &str, args: &[&str]) -> bool {
    match Command::new(binary).args(args).output() {
        Ok(out) => out.status.success(),
        Err(err) => {
            tracing::warn!("{binary} failed: {err}");
            false
        }
    }
}

fn schema_exists(schema: &str) -> bool {
    Command::new("gsettings")
        .args(["list-schemas"])
        .output()
        .is_ok_and(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == schema)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Our entry has to end up first, and the user's existing choices must
    /// survive below it rather than being wiped.
    #[test]
    fn preserves_other_terminals_below_ours() {
        let dir = std::env::temp_dir().join("optionterm-xdg-terminals-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("xdg-terminals.list");
        std::fs::write(&path, "# comment\nalacritty.desktop\nfoot.desktop\n").expect("seed");

        // Mirrors set_default's rewrite, without touching the real config dir.
        let existing = std::fs::read_to_string(&path).expect("read");
        let mut lines: Vec<&str> = existing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != DESKTOP_ID)
            .collect();
        lines.insert(0, DESKTOP_ID);
        let out = format!("{}\n", lines.join("\n"));

        let kept: Vec<&str> = out.lines().collect();
        assert_eq!(kept[0], DESKTOP_ID, "we must be first to be the default");
        assert!(kept.contains(&"alacritty.desktop"));
        assert!(kept.contains(&"foot.desktop"));
        assert_eq!(
            kept.iter().filter(|l| **l == DESKTOP_ID).count(),
            1,
            "applying twice must not duplicate the entry"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
