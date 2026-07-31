//! optionTerm config (`~/.option/terminal/config.toml`).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// RGB color used by the palette and theme fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Default cursor shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
    BlockHollow,
}

/// Where the tab list lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsLocation {
    Top,
    Left,
    Right,
    Hidden,
}

/// Interface color scheme.
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

/// Where a freshly created tab is inserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTabPosition {
    AfterCurrent,
    BeforeCurrent,
    End,
    Start,
}

impl NewTabPosition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AfterCurrent => "after_current",
            Self::BeforeCurrent => "before_current",
            Self::End => "end",
            Self::Start => "start",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "before_current" => Self::BeforeCurrent,
            "end" => Self::End,
            "start" => Self::Start,
            _ => Self::AfterCurrent,
        }
    }
}

/// What a middle click on a tab does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiddleClickTab {
    Ignore,
    NewTab,
    CloseTab,
}

impl MiddleClickTab {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ignore => "nothing",
            Self::NewTab => "new_tab",
            Self::CloseTab => "close_tab",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "new_tab" => Self::NewTab,
            "close_tab" => Self::CloseTab,
            _ => Self::Ignore,
        }
    }
}

/// How tabs share the width of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabWidth {
    Fill,
    Natural,
}

impl TabWidth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Natural => "natural",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "natural" => Self::Natural,
            _ => Self::Fill,
        }
    }
}

/// What happens once there are more tabs than fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabOverflow {
    Squeeze,
    Scroll,
}

impl TabOverflow {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Squeeze => "squeeze",
            Self::Scroll => "scroll",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "scroll" => Self::Scroll,
            _ => Self::Squeeze,
        }
    }
}

/// Visual + sizing settings.
#[derive(Clone, Debug)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    /// Shape programming ligatures (`->`, `=>`, `!=`).
    pub font_ligatures: bool,
    /// Use the desktop's monospace font instead of `font_family`.
    pub use_system_font: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub tabs_location: TabsLocation,
    /// Show the tab sidebar even with a single tab.
    pub sidebar_always: bool,
    pub theme: Theme,
    /// Window/terminal background alpha, 0.0..=1.0.
    pub background_opacity: f64,
    /// Restore tabs/splits and their working directories on start.
    pub session_restore: bool,
    /// New tabs and splits start in the focused pane's directory.
    pub inherit_working_directory: bool,
    pub new_tab_position: NewTabPosition,
    pub tab_width: TabWidth,
    pub tab_overflow: TabOverflow,
    pub show_search_button: bool,
    pub middle_click_tab: MiddleClickTab,
    pub confirm_close_tab: bool,
    pub confirm_quit: bool,
    pub bell_sound: bool,
    pub command_finished_sound: bool,
    pub scroll_on_keystroke: bool,
    pub scroll_button: bool,
    pub scroll_bar: bool,
    pub keep_awake: bool,
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
            font_ligatures: true,
            use_system_font: false,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            // Sidebar-first is the product default on first run.
            tabs_location: TabsLocation::Left,
            sidebar_always: false,
            theme: Theme::System,
            background_opacity: 1.0,
            session_restore: true,
            inherit_working_directory: true,
            new_tab_position: NewTabPosition::AfterCurrent,
            tab_width: TabWidth::Fill,
            tab_overflow: TabOverflow::Squeeze,
            show_search_button: false,
            middle_click_tab: MiddleClickTab::Ignore,
            confirm_close_tab: false,
            confirm_quit: true,
            bell_sound: true,
            command_finished_sound: false,
            scroll_on_keystroke: true,
            scroll_button: false,
            scroll_bar: true,
            keep_awake: false,
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
    /// generated from built-in defaults (sidebar tabs).
    pub fn load() -> Result<Self> {
        let path = option_config_path();
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let mut cfg = Self::parse_toml(&text)?;
            cfg.source = path;
            return Ok(cfg);
        }

        let mut cfg = Self::default();
        cfg.source = path.clone();
        if let Err(err) = cfg.write_to(&path) {
            tracing::warn!(
                "could not write default config to {}: {err:#}",
                path.display()
            );
        } else {
            tracing::info!("generated default config at {}", path.display());
        }
        Ok(cfg)
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

        if let Some(v) = str_at("font", "family")
            && !v.is_empty()
        {
            cfg.font_family = v;
        }
        if let Some(v) = num_at("font", "size") {
            cfg.font_size = v as f32;
        }
        if let Some(v) = bool_at("font", "ligatures") {
            cfg.font_ligatures = v;
        }
        if let Some(v) = bool_at("font", "use_system") {
            cfg.use_system_font = v;
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
        if let Some(v) = num_at("window", "background_opacity") {
            cfg.background_opacity = v.clamp(0.15, 1.0);
        }
        if let Some(v) = bool_at("window", "inherit_working_directory") {
            cfg.inherit_working_directory = v;
        }
        if let Some(v) = str_at("window", "new_tab_position") {
            cfg.new_tab_position = NewTabPosition::parse(&v);
        }
        if let Some(v) = str_at("window", "tab_width") {
            cfg.tab_width = TabWidth::parse(&v);
        }
        if let Some(v) = str_at("window", "tab_overflow") {
            cfg.tab_overflow = TabOverflow::parse(&v);
        }
        if let Some(v) = bool_at("window", "show_search_button") {
            cfg.show_search_button = v;
        }
        if let Some(v) = str_at("window", "middle_click_tab") {
            cfg.middle_click_tab = MiddleClickTab::parse(&v);
        }
        if let Some(v) = bool_at("window", "confirm_close_tab") {
            cfg.confirm_close_tab = v;
        }
        if let Some(v) = bool_at("window", "confirm_quit") {
            cfg.confirm_quit = v;
        }
        if let Some(v) = bool_at("sound", "bell") {
            cfg.bell_sound = v;
        }
        if let Some(v) = bool_at("sound", "command_finished") {
            cfg.command_finished_sound = v;
        }
        if let Some(v) = bool_at("scroll", "on_keystroke") {
            cfg.scroll_on_keystroke = v;
        }
        if let Some(v) = bool_at("scroll", "show_button") {
            cfg.scroll_button = v;
        }
        if let Some(v) = bool_at("scroll", "show_bar") {
            cfg.scroll_bar = v;
        }
        if let Some(v) = bool_at("window", "keep_awake") {
            cfg.keep_awake = v;
        }
        if let Some(v) = bool_at("window", "session_restore") {
            cfg.session_restore = v;
        }
        // Ignored legacy key from ≤0.1.x (scrollback-content restore removed).
        let _ = bool_at("window", "session_restore_scrollback");
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

    pub fn save(&self) -> Result<()> {
        let path = if self.source.as_os_str().is_empty() {
            option_config_path()
        } else {
            self.source.clone()
        };
        self.write_to(&path)
    }

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

[font]
family = "{family}"
size = {size}
ligatures = {ligatures}   # shape ->, =>, != as single glyphs
use_system = {use_system}   # ignore `family` and use the desktop's monospace font

[cursor]
style = "{style}"   # block | bar | underline | block_hollow
blink = {blink}
color = "{cursor}"
text = "{cursor_text}"

[window]
tabs = "{tabs}"     # top | left | right | hidden
sidebar_always = {sidebar_always}   # show the sidebar even with a single tab
theme = "{theme}"   # system | light | dark
background_opacity = {opacity}   # 0.15 .. 1.0
session_restore = {session_restore}   # restore tabs/splits and cwd on start
inherit_working_directory = {inherit_cwd}   # new tabs/splits open in the focused pane's directory
new_tab_position = "{new_tab_pos}"   # after_current | before_current | end | start
tab_width = "{tab_width}"   # fill (share the bar) | natural (as wide as the title)
tab_overflow = "{tab_overflow}"   # squeeze (keep shrinking) | scroll (hold a width, scroll the bar)
show_search_button = {show_search}   # magnifier in the header for the command palette
middle_click_tab = "{middle_click}"   # nothing | new_tab | close_tab
confirm_close_tab = {confirm_close_tab}   # ask before closing a tab that is still running something
confirm_quit = {confirm_quit}   # ask before closing a window with more than one tab
keep_awake = {keep_awake}   # keep the session awake while a pane has a foreground job
padding_x = {pad_x}
padding_y = {pad_y}

[scroll]
on_keystroke = {scroll_keys}   # typing while scrolled up jumps back to the prompt
show_button = {scroll_btn}   # floating button to jump to the bottom
show_bar = {scroll_bar}   # scrollbar, shown only when there is scrollback

[sound]
bell = {bell}   # ring the system bell on BEL
command_finished = {cmd_done}   # sound when a command finishes while the window is unfocused

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
            ligatures = self.font_ligatures,
            use_system = self.use_system_font,
            inherit_cwd = self.inherit_working_directory,
            new_tab_pos = self.new_tab_position.as_str(),
            tab_width = self.tab_width.as_str(),
            tab_overflow = self.tab_overflow.as_str(),
            show_search = self.show_search_button,
            middle_click = self.middle_click_tab.as_str(),
            confirm_close_tab = self.confirm_close_tab,
            confirm_quit = self.confirm_quit,
            keep_awake = self.keep_awake,
            bell = self.bell_sound,
            scroll_keys = self.scroll_on_keystroke,
            scroll_btn = self.scroll_button,
            scroll_bar = self.scroll_bar,
            cmd_done = self.command_finished_sound,
            sidebar_always = self.sidebar_always,
            theme = self.theme.as_str(),
            opacity = self.background_opacity,
            session_restore = self.session_restore,
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
}

/// `~/.option/terminal`
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".option")
        .join("terminal")
}

/// `~/.option/terminal/config.toml`
pub fn option_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

fn hex(c: RgbColor) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
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

    #[test]
    fn ligatures_and_cwd_inheritance_default_on() {
        let cfg = Config::default();
        assert!(cfg.font_ligatures);
        assert!(cfg.inherit_working_directory);
    }

    #[test]
    fn defaults_match_the_documented_behaviour() {
        let cfg = Config::default();
        assert!(cfg.session_restore);
        assert!(cfg.confirm_quit);
        assert!(!cfg.confirm_close_tab);
        assert!(cfg.bell_sound);
        assert!(cfg.scroll_on_keystroke);
        assert!(!cfg.scroll_button);
        assert!(cfg.scroll_bar);
        assert!(!cfg.keep_awake);
        assert_eq!(cfg.new_tab_position, NewTabPosition::AfterCurrent);
        assert_eq!(cfg.middle_click_tab, MiddleClickTab::Ignore);
        assert_eq!(cfg.tabs_location, TabsLocation::Left);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut cfg = Config {
            font_family: "FiraCode Nerd Font".into(),
            font_size: 15.0,
            cursor_style: CursorStyle::Bar,
            cursor_blink: false,
            tabs_location: TabsLocation::Right,
            sidebar_always: true,
            theme: Theme::Dark,
            background_opacity: 0.85,
            session_restore: true,
            font_ligatures: false,
            use_system_font: true,
            inherit_working_directory: false,
            new_tab_position: NewTabPosition::Start,
            middle_click_tab: MiddleClickTab::CloseTab,
            confirm_close_tab: true,
            confirm_quit: false,
            bell_sound: false,
            command_finished_sound: true,
            scroll_on_keystroke: false,
            scroll_button: true,
            scroll_bar: false,
            keep_awake: true,
            padding_x: 8.0,
            padding_y: 6.0,
            ..Config::default()
        };
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
        assert_eq!(back.background_opacity, cfg.background_opacity);
        assert_eq!(back.session_restore, cfg.session_restore);
        assert_eq!(back.font_ligatures, cfg.font_ligatures);
        assert_eq!(back.use_system_font, cfg.use_system_font);
        assert_eq!(back.new_tab_position, cfg.new_tab_position);
        assert_eq!(back.middle_click_tab, cfg.middle_click_tab);
        assert_eq!(back.confirm_close_tab, cfg.confirm_close_tab);
        assert_eq!(back.confirm_quit, cfg.confirm_quit);
        assert_eq!(back.bell_sound, cfg.bell_sound);
        assert_eq!(back.command_finished_sound, cfg.command_finished_sound);
        assert_eq!(back.scroll_on_keystroke, cfg.scroll_on_keystroke);
        assert_eq!(back.scroll_button, cfg.scroll_button);
        assert_eq!(back.scroll_bar, cfg.scroll_bar);
        assert_eq!(back.keep_awake, cfg.keep_awake);
        assert_eq!(
            back.inherit_working_directory,
            cfg.inherit_working_directory
        );
        assert_eq!(back.padding_x, cfg.padding_x);
        assert_eq!(back.padding_y, cfg.padding_y);
        assert_eq!(back.palette[3], cfg.palette[3]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignores_legacy_scrollback_restore_key() {
        let cfg = Config::parse_toml(
            "\n[window]\nsession_restore = true\nsession_restore_scrollback = true\n",
        )
        .expect("parse");
        assert!(cfg.session_restore);
    }
}
