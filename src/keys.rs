//! Keyboard shortcuts, in their own file next to `config.toml`.
//!
//! Kept separate on purpose: bindings are edited far more often than colours
//! and padding, they are long enough to dominate a shared file, and a syntax
//! error here must not cost the user the rest of their configuration.
//!
//! Only overrides are stored. Anything absent keeps the built-in binding, so
//! the file stays readable and new defaults reach existing installs.

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::ui::COMMANDS;

/// `~/.option/terminal/keys.toml`
pub fn path() -> std::path::PathBuf {
    crate::config::config_dir().join("keys.toml")
}

/// Action name (without the `win.` prefix) mapped to its accelerator.
///
/// An empty accelerator means the action is deliberately unbound.
#[derive(Clone, Debug, Default)]
pub struct Bindings(BTreeMap<String, String>);

impl Bindings {
    /// Read the overrides, treating an unreadable or invalid file as empty so a
    /// typo cannot lock the user out of their own shortcuts.
    pub fn load() -> Self {
        let path = path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match Self::parse(&text) {
            Ok(bindings) => bindings,
            Err(err) => {
                tracing::warn!("ignoring {}: {err:#}", path.display());
                Self::default()
            }
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text.parse().context("parsing keys.toml")?;
        let mut map = BTreeMap::new();
        // `[keys]` is optional; a bare table of pairs also works.
        let entries = table
            .get("keys")
            .and_then(|v| v.as_table())
            .unwrap_or(&table);
        for (action, value) in entries {
            if let Some(accel) = value.as_str() {
                map.insert(action.clone(), accel.to_string());
            }
        }
        Ok(Self(map))
    }

    pub fn get(&self, action: &str) -> Option<&str> {
        self.0.get(action).map(String::as_str)
    }

    /// Override an action, or clear the override with `None`.
    pub fn set(&mut self, action: &str, accel: Option<&str>) {
        match accel {
            Some(accel) => {
                self.0.insert(action.to_string(), accel.to_string());
            }
            None => {
                self.0.remove(action);
            }
        }
    }

    /// The action already using `accel`, if any. Used to refuse duplicates,
    /// since two actions on one key means one of them silently never fires.
    pub fn conflict(&self, accel: &str, ignoring: &str) -> Option<String> {
        for (label, action, default) in COMMANDS {
            let name = action.trim_start_matches("win.");
            if name == ignoring {
                continue;
            }
            let effective = self.get(name).unwrap_or(default);
            if !effective.is_empty() && accel_eq(effective, accel) {
                return Some((*label).to_string());
            }
        }
        None
    }

    /// Every action paired with the accelerator actually in effect.
    pub fn effective(&self) -> Vec<(&'static str, &'static str, String)> {
        COMMANDS
            .iter()
            .map(|(label, action, default)| {
                let name = action.trim_start_matches("win.");
                let accel = self.get(name).unwrap_or(default).to_string();
                (*label, *action, accel)
            })
            .collect()
    }

    pub fn save(&self) -> Result<()> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut out = String::from(
            "# optionTerm shortcuts — ~/.option/terminal/keys.toml\n\
             #\n\
             # Only overrides live here; anything not listed keeps its built-in\n\
             # binding. Use GTK accelerator syntax, e.g. \"<Control><Shift>t\".\n\
             # An empty string unbinds the action.\n\n\
             [keys]\n",
        );
        for (action, accel) in &self.0 {
            out.push_str(&format!("{action} = \"{accel}\"\n"));
        }
        crate::storage::atomic_write(&path, out.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// Compare two accelerators by meaning rather than spelling, so `Ctrl+Shift+T`
/// and `<Control><Shift>t` are recognised as the same binding.
///
/// Done textually rather than through `gtk_accelerator_parse`, which requires an
/// initialized GTK and so could not be tested or called before the window
/// exists.
fn accel_eq(a: &str, b: &str) -> bool {
    canonical_accel(a) == canonical_accel(b)
}

/// Modifiers sorted, key lowercased, so equal bindings compare equal.
fn canonical_accel(accel: &str) -> String {
    let gtk = to_gtk_accel(accel);
    let mut mods: Vec<&str> = Vec::new();
    let mut rest = gtk.as_str();
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        mods.push(&rest[start + 1..start + end]);
        rest = &rest[start + end + 1..];
    }
    // Control and Ctrl name the same key; normalise before sorting.
    let mut mods: Vec<String> = mods
        .into_iter()
        .map(|m| match m.to_lowercase().as_str() {
            "ctrl" => "control".to_string(),
            "cmd" => "super".to_string(),
            other => other.to_string(),
        })
        .collect();
    mods.sort();
    mods.dedup();
    format!("{}{}", mods.join("+"), rest.to_lowercase())
}

/// Translate the human spelling used in `COMMANDS` into GTK's syntax.
///
/// The tables in this project are written for humans ("Ctrl+Shift+T"), while
/// `gtk_accelerator_parse` wants "<Control><Shift>t".
pub fn to_gtk_accel(accel: &str) -> String {
    if accel.contains('<') {
        return accel.to_string();
    }
    let mut out = String::new();
    let mut key = "";
    for part in accel.split('+') {
        match part.trim() {
            "Ctrl" | "Control" => out.push_str("<Control>"),
            "Shift" => out.push_str("<Shift>"),
            "Alt" => out.push_str("<Alt>"),
            "Super" | "Cmd" => out.push_str("<Super>"),
            // A trailing "+" makes an empty final part, as in "Ctrl++".
            "" => key = "plus",
            other => key = other,
        }
    }
    out.push_str(&match key {
        "PgUp" => "Page_Up".to_string(),
        "PgDn" => "Page_Down".to_string(),
        "+" | "plus" => "plus".to_string(),
        "-" => "minus".to_string(),
        "," => "comma".to_string(),
        other => other.to_lowercase(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_overrides_are_stored() {
        let bindings = Bindings::parse("[keys]\nnew-tab = \"<Control>n\"\n").expect("parse");
        assert_eq!(bindings.get("new-tab"), Some("<Control>n"));
        assert_eq!(bindings.get("close-tab"), None, "untouched stays default");

        // The effective list must merge overrides over the built-ins.
        let effective = bindings.effective();
        let new_tab = effective.iter().find(|(_, a, _)| *a == "win.new-tab");
        assert_eq!(
            new_tab.map(|(_, _, accel)| accel.as_str()),
            Some("<Control>n")
        );
        let close = effective.iter().find(|(_, a, _)| *a == "win.close-tab");
        assert_eq!(
            close.map(|(_, _, accel)| accel.as_str()),
            Some("Ctrl+Shift+W"),
            "a default must survive untouched"
        );
    }

    #[test]
    fn a_table_without_the_section_header_still_parses() {
        let bindings = Bindings::parse("find = \"<Control>f\"\n").expect("parse");
        assert_eq!(bindings.get("find"), Some("<Control>f"));
    }

    /// A broken file must cost the user their shortcuts, not their config.
    #[test]
    fn invalid_toml_is_rejected_rather_than_panicking() {
        assert!(Bindings::parse("this is not toml = = =").is_err());
    }

    #[test]
    fn human_spellings_convert_to_gtk_syntax() {
        assert_eq!(to_gtk_accel("Ctrl+Shift+T"), "<Control><Shift>t");
        assert_eq!(to_gtk_accel("Ctrl+PgUp"), "<Control>Page_Up");
        assert_eq!(to_gtk_accel("Ctrl+,"), "<Control>comma");
        assert_eq!(to_gtk_accel("F2"), "f2");
        // Already-GTK strings pass through untouched.
        assert_eq!(to_gtk_accel("<Control><Shift>t"), "<Control><Shift>t");
    }

    #[test]
    fn accelerators_compare_by_meaning() {
        assert!(accel_eq("Ctrl+Shift+T", "<Control><Shift>t"));
        assert!(accel_eq("<Shift><Control>t", "<Control><Shift>T"));
        assert!(!accel_eq("Ctrl+T", "Ctrl+Shift+T"));
        assert!(!accel_eq("Ctrl+T", "Ctrl+Y"));
    }

    #[test]
    fn conflicts_are_detected_across_spellings() {
        let bindings = Bindings::default();
        // Ctrl+Shift+T is New Tab out of the box, however it is spelled.
        assert_eq!(
            bindings.conflict("<Control><Shift>t", "find").as_deref(),
            Some("New Tab")
        );
        // ...unless it is the very action being rebound.
        assert_eq!(bindings.conflict("<Control><Shift>t", "new-tab"), None);
        // A free combination is free.
        assert_eq!(bindings.conflict("<Control><Alt><Shift>j", "find"), None);
    }
}
