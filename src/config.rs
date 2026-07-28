//! Ghostty-compatible config loader (`key = value`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use libghostty_vt::style::{PaletteIndex, RgbColor};

/// Default cursor shape (Ghostty `cursor-style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
    BlockHollow,
}

/// Where the tab list lives (Ghostty `gtk-tabs-location`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsLocation {
    Top,
    Left,
    Right,
    Hidden,
}

/// Interface color scheme (Ghostty `window-theme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::System,
        }
    }
}

/// Visual + sizing settings we honor from Ghostty's config file.
#[derive(Clone, Debug)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub tabs_location: TabsLocation,
    /// Show the tab sidebar even with a single tab.
    pub sidebar_always: bool,
    pub theme: Theme,
    pub background: RgbColor,
    pub foreground: RgbColor,
    pub cursor: RgbColor,
    pub cursor_text: RgbColor,
    pub selection_background: RgbColor,
    pub selection_foreground: RgbColor,
    pub palette: [RgbColor; 16],
    pub padding_x: f64,
    pub padding_y: f64,
    pub source: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "monospace".into(),
            font_size: 13.0,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            tabs_location: TabsLocation::Top,
            sidebar_always: false,
            theme: Theme::System,
            background: rgb(0x28, 0x2c, 0x34),
            foreground: rgb(0xff, 0xff, 0xff),
            cursor: rgb(0xff, 0xff, 0xff),
            cursor_text: rgb(0x28, 0x2c, 0x34),
            selection_background: rgb(0x3a, 0x3a, 0x3a),
            selection_foreground: rgb(0xff, 0xff, 0xff),
            palette: default_ansi(),
            padding_x: 2.0,
            padding_y: 2.0,
            source: PathBuf::new(),
        }
    }
}

impl Config {
    /// Load `~/.option/terminal/config.toml`. On first run the file is
    /// generated from the system Ghostty config (sidebar tabs by default).
    pub fn load() -> Result<Self> {
        let path = option_config_path();
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let mut cfg = Self::parse_toml(&text)?;
            cfg.source = path;
            return Ok(cfg);
        }

        let mut cfg = Self::load_ghostty();
        cfg.tabs_location = TabsLocation::Left;
        cfg.source = path.clone();
        if let Err(err) = cfg.write_to(&path) {
            tracing::warn!("could not write default config to {}: {err:#}", path.display());
        } else {
            tracing::info!("generated default config at {}", path.display());
        }
        Ok(cfg)
    }

    /// Load the system Ghostty config (used to seed the default config.toml).
    fn load_ghostty() -> Self {
        let path = ghostty_config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(_) => {
                tracing::warn!(
                    "Ghostty config not found at {}, using built-in defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn parse_toml(text: &str) -> Result<Self> {
        let table: toml::Table = text.parse().context("parsing config.toml")?;
        let mut cfg = Self::default();

        let str_at = |section: &str, key: &str| -> Option<String> {
            table
                .get(section)?
                .as_table()?
                .get(key)?
                .as_str()
                .map(str::to_string)
        };
        let num_at = |section: &str, key: &str| -> Option<f64> {
            let v = table.get(section)?.as_table()?.get(key)?;
            v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
        };
        let bool_at = |section: &str, key: &str| -> Option<bool> {
            table.get(section)?.as_table()?.get(key)?.as_bool()
        };

        if let Some(v) = str_at("font", "family") {
            if !v.is_empty() {
                cfg.font_family = v;
            }
        }
        if let Some(v) = num_at("font", "size") {
            cfg.font_size = v as f32;
        }

        if let Some(v) = str_at("cursor", "style") {
            cfg.cursor_style = match v.as_str() {
                "bar" => CursorStyle::Bar,
                "underline" => CursorStyle::Underline,
                "block_hollow" => CursorStyle::BlockHollow,
                _ => CursorStyle::Block,
            };
        }
        if let Some(v) = bool_at("cursor", "blink") {
            cfg.cursor_blink = v;
        }
        if let Some(c) = str_at("cursor", "color").as_deref().and_then(parse_color) {
            cfg.cursor = c;
        }
        if let Some(c) = str_at("cursor", "text").as_deref().and_then(parse_color) {
            cfg.cursor_text = c;
        }

        if let Some(v) = str_at("window", "tabs") {
            cfg.tabs_location = match v.as_str() {
                "top" => TabsLocation::Top,
                "right" => TabsLocation::Right,
                "hidden" => TabsLocation::Hidden,
                _ => TabsLocation::Left,
            };
        }
        if let Some(v) = bool_at("window", "sidebar_always") {
            cfg.sidebar_always = v;
        }
        if let Some(v) = str_at("window", "theme") {
            cfg.theme = Theme::parse(&v);
        }
        if let Some(v) = num_at("window", "padding_x") {
            cfg.padding_x = v;
        }
        if let Some(v) = num_at("window", "padding_y") {
            cfg.padding_y = v;
        }

        let color_at = |key: &str| str_at("colors", key).as_deref().and_then(parse_color);
        if let Some(c) = color_at("background") {
            cfg.background = c;
        }
        if let Some(c) = color_at("foreground") {
            cfg.foreground = c;
        }
        if let Some(c) = color_at("selection_background") {
            cfg.selection_background = c;
        }
        if let Some(c) = color_at("selection_foreground") {
            cfg.selection_foreground = c;
        }
        if let Some(arr) = table
            .get("colors")
            .and_then(|t| t.as_table())
            .and_then(|t| t.get("palette"))
            .and_then(|v| v.as_array())
        {
            for (i, v) in arr.iter().take(16).enumerate() {
                if let Some(c) = v.as_str().and_then(parse_color) {
                    cfg.palette[i] = c;
                }
            }
        }

        Ok(cfg)
    }

    /// Persist the current settings back to the file they were loaded from.
    /// Called whenever a setting is changed from the UI so preferences
    /// survive a restart.
    pub fn save(&self) -> Result<()> {
        let path = if self.source.as_os_str().is_empty() {
            option_config_path()
        } else {
            self.source.clone()
        };
        self.write_to(&path)
    }

    /// Serialize as config.toml (with comments) and write it to `path`.
    pub fn write_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let style = match self.cursor_style {
            CursorStyle::Block => "block",
            CursorStyle::Bar => "bar",
            CursorStyle::Underline => "underline",
            CursorStyle::BlockHollow => "block_hollow",
        };
        let tabs = match self.tabs_location {
            TabsLocation::Top => "top",
            TabsLocation::Left => "left",
            TabsLocation::Right => "right",
            TabsLocation::Hidden => "hidden",
        };
        let palette = self
            .palette
            .iter()
            .map(|c| format!("  \"{}\",", hex(*c)))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            r##"# optionTerm — ~/.option/terminal/config.toml
# Generated from the system Ghostty config.

[font]
family = "{family}"
size = {size}

[cursor]
style = "{style}"   # block | bar | underline | block_hollow
blink = {blink}
color = "{cursor}"
text = "{cursor_text}"

[window]
tabs = "{tabs}"     # top | left | right | hidden
sidebar_always = {sidebar_always}   # show the sidebar even with a single tab
theme = "{theme}"   # system | light | dark
padding_x = {pad_x}
padding_y = {pad_y}

[colors]
background = "{bg}"
foreground = "{fg}"
selection_background = "{sel_bg}"
selection_foreground = "{sel_fg}"
palette = [
{palette}
]
"##,
            family = self.font_family,
            size = self.font_size,
            sidebar_always = self.sidebar_always,
            theme = self.theme.as_str(),
            blink = self.cursor_blink,
            cursor = hex(self.cursor),
            cursor_text = hex(self.cursor_text),
            pad_x = self.padding_x,
            pad_y = self.padding_y,
            bg = hex(self.background),
            fg = hex(self.foreground),
            sel_bg = hex(self.selection_background),
            sel_fg = hex(self.selection_foreground),
        );
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = strip_quotes(value.trim());
            match key {
                "font-family" => {
                    if value.is_empty() {
                        cfg.font_family = "monospace".into();
                    } else {
                        cfg.font_family = value.to_string();
                    }
                }
                "font-size" => {
                    if let Ok(v) = value.parse::<f32>() {
                        cfg.font_size = v;
                    }
                }
                "cursor-style" => {
                    cfg.cursor_style = match value {
                        "bar" => CursorStyle::Bar,
                        "underline" => CursorStyle::Underline,
                        "block_hollow" => CursorStyle::BlockHollow,
                        _ => CursorStyle::Block,
                    };
                }
                "gtk-tabs-location" => {
                    cfg.tabs_location = match value {
                        "left" => TabsLocation::Left,
                        "right" => TabsLocation::Right,
                        "hidden" => TabsLocation::Hidden,
                        // `bottom` is not supported; treat it as top.
                        _ => TabsLocation::Top,
                    };
                }
                "window-theme" => {
                    cfg.theme = Theme::parse(value);
                }
                "cursor-style-blink" => {
                    if let Ok(v) = value.parse::<bool>() {
                        cfg.cursor_blink = v;
                    }
                }
                "background" => {
                    if let Some(c) = parse_color(value) {
                        cfg.background = c;
                    }
                }
                "foreground" => {
                    if let Some(c) = parse_color(value) {
                        cfg.foreground = c;
                    }
                }
                "cursor-color" => {
                    if let Some(c) = parse_color(value) {
                        cfg.cursor = c;
                    }
                }
                "cursor-text" => {
                    if let Some(c) = parse_color(value) {
                        cfg.cursor_text = c;
                    }
                }
                "selection-background" => {
                    if let Some(c) = parse_color(value) {
                        cfg.selection_background = c;
                    }
                }
                "selection-foreground" => {
                    if let Some(c) = parse_color(value) {
                        cfg.selection_foreground = c;
                    }
                }
                "window-padding-x" => {
                    if let Ok(v) = value.parse::<f64>() {
                        cfg.padding_x = v;
                    }
                }
                "window-padding-y" => {
                    if let Ok(v) = value.parse::<f64>() {
                        cfg.padding_y = v;
                    }
                }
                "palette" => {
                    if let Some((idx, color)) = parse_palette_entry(value) {
                        if idx < 16 {
                            cfg.palette[idx] = color;
                        }
                    }
                }
                _ => {}
            }
        }
        cfg
    }

    /// Push `cursor-style` / `cursor-style-blink` into the terminal so they
    /// become the DECSCUSR defaults. Without this the VT reports a
    /// non-blinking cursor and `cursor.blink = true` never takes effect.
    pub fn apply_cursor_to_terminal(
        &self,
        terminal: &mut libghostty_vt::Terminal<'_, '_>,
    ) -> Result<()> {
        use anyhow::anyhow;
        use libghostty_vt::terminal::CursorStyle as VtCursorStyle;
        let style = match self.cursor_style {
            CursorStyle::Block => VtCursorStyle::Block,
            CursorStyle::Bar => VtCursorStyle::Bar,
            CursorStyle::Underline => VtCursorStyle::Underline,
            CursorStyle::BlockHollow => VtCursorStyle::BlockHollow,
        };
        terminal
            .set_default_cursor_style(Some(style))
            .map_err(|e| anyhow!("{e:?}"))?;
        terminal
            .set_default_cursor_blink(Some(self.cursor_blink))
            .map_err(|e| anyhow!("{e:?}"))?;
        Ok(())
    }

    /// Apply colors onto a libghostty terminal.
    pub fn apply_to_terminal(&self, terminal: &mut libghostty_vt::Terminal<'_, '_>) -> Result<()> {
        use anyhow::anyhow;
        self.apply_cursor_to_terminal(terminal)?;
        terminal
            .set_default_fg_color(Some(self.foreground))
            .map_err(|e| anyhow!("{e:?}"))?;
        terminal
            .set_default_bg_color(Some(self.background))
            .map_err(|e| anyhow!("{e:?}"))?;
        terminal
            .set_default_cursor_color(Some(self.cursor))
            .map_err(|e| anyhow!("{e:?}"))?;

        let mut palette = terminal
            .default_color_palette()
            .map_err(|e| anyhow!("{e:?}"))?;
        for (i, color) in self.palette.iter().enumerate() {
            palette.set(PaletteIndex(i as u8), *color);
        }
        terminal
            .set_default_color_palette(Some(palette))
            .map_err(|e| anyhow!("{e:?}"))?;
        Ok(())
    }
}

/// `~/.option/terminal/config.toml`
pub fn option_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".option")
        .join("terminal")
        .join("config.toml")
}

fn hex(c: RgbColor) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

pub fn ghostty_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ghostty")
        .join("config")
}

fn strip_quotes(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 {
        let bytes = v.as_bytes();
        if (bytes[0] == b'"' && bytes[v.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[v.len() - 1] == b'\'')
        {
            return &v[1..v.len() - 1];
        }
    }
    v
}

fn parse_palette_entry(value: &str) -> Option<(usize, RgbColor)> {
    let (idx_s, color_s) = value.split_once('=')?;
    let idx: usize = idx_s.trim().parse().ok()?;
    let color = parse_color(color_s.trim())?;
    Some((idx, color))
}

fn parse_color(value: &str) -> Option<RgbColor> {
    let v = value.trim().trim_start_matches('#');
    if v.len() == 6 {
        let r = u8::from_str_radix(&v[0..2], 16).ok()?;
        let g = u8::from_str_radix(&v[2..4], 16).ok()?;
        let b = u8::from_str_radix(&v[4..6], 16).ok()?;
        return Some(rgb(r, g, b));
    }
    if v.len() == 3 {
        let r = u8::from_str_radix(&v[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&v[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&v[2..3], 16).ok()? * 17;
        return Some(rgb(r, g, b));
    }
    None
}

fn rgb(r: u8, g: u8, b: u8) -> RgbColor {
    RgbColor { r, g, b }
}

fn default_ansi() -> [RgbColor; 16] {
    [
        rgb(0x1e, 0x1e, 0x1e),
        rgb(0xf4, 0x47, 0x47),
        rgb(0x6a, 0x99, 0x55),
        rgb(0xdc, 0xdc, 0xaa),
        rgb(0x56, 0x9c, 0xd6),
        rgb(0xc6, 0x78, 0xdd),
        rgb(0x4e, 0xc9, 0xb0),
        rgb(0xd4, 0xd4, 0xd4),
        rgb(0x55, 0x55, 0x55),
        rgb(0xf4, 0x47, 0x47),
        rgb(0x6a, 0x99, 0x55),
        rgb(0xdc, 0xdc, 0xaa),
        rgb(0x56, 0x9c, 0xd6),
        rgb(0xc6, 0x78, 0xdd),
        rgb(0x4e, 0xc9, 0xb0),
        rgb(0xff, 0xff, 0xff),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything we persist must survive a save/load round trip, otherwise
    /// preferences silently reset on the next start.
    #[test]
    fn config_round_trips_through_toml() {
        let mut cfg = Config::default();
        cfg.font_family = "FiraCode Nerd Font".into();
        cfg.font_size = 15.0;
        cfg.cursor_style = CursorStyle::Bar;
        cfg.cursor_blink = false;
        cfg.tabs_location = TabsLocation::Right;
        cfg.sidebar_always = true;
        cfg.theme = Theme::Dark;
        cfg.padding_x = 8.0;
        cfg.padding_y = 6.0;
        cfg.palette[3] = rgb(0x12, 0x34, 0x56);

        let dir = std::env::temp_dir().join("option-term-config-test");
        let path = dir.join("config.toml");
        cfg.write_to(&path).expect("write config");
        let text = std::fs::read_to_string(&path).expect("read config");
        let back = Config::parse_toml(&text).expect("parse config");

        assert_eq!(back.font_family, cfg.font_family);
        assert_eq!(back.font_size, cfg.font_size);
        assert_eq!(back.cursor_style, cfg.cursor_style);
        assert_eq!(back.cursor_blink, cfg.cursor_blink);
        assert_eq!(back.tabs_location, cfg.tabs_location);
        assert_eq!(back.sidebar_always, cfg.sidebar_always);
        assert_eq!(back.theme, cfg.theme);
        assert_eq!(back.padding_x, cfg.padding_x);
        assert_eq!(back.padding_y, cfg.padding_y);
        assert_eq!(back.palette[3], cfg.palette[3]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression: `cursor.blink = true` did nothing because the VT default
    /// was never set, so the render snapshot always reported a static cursor.
    #[test]
    fn cursor_blink_reaches_the_terminal() {
        use libghostty_vt::{Terminal, TerminalOptions, render::RenderState};

        let make = || {
            Terminal::new(TerminalOptions { cols: 20, rows: 5, max_scrollback: 0 })
                .expect("terminal")
        };

        let mut cfg = Config::default();
        cfg.cursor_blink = true;
        let mut terminal = make();
        cfg.apply_cursor_to_terminal(&mut terminal).expect("apply");
        let mut state = RenderState::new().expect("render state");
        let snapshot = state.update(&terminal).expect("snapshot");
        assert_eq!(snapshot.cursor_blinking().unwrap(), true);

        cfg.cursor_blink = false;
        let mut terminal = make();
        cfg.apply_cursor_to_terminal(&mut terminal).expect("apply");
        let mut state = RenderState::new().expect("render state");
        let snapshot = state.update(&terminal).expect("snapshot");
        assert_eq!(snapshot.cursor_blinking().unwrap(), false);
    }
}
