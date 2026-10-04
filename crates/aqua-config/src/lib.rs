//! aqua-config: user configuration + design tokens (metrics, colors, glass parameters).
//!
//! Configuration is read from `$XDG_CONFIG_HOME/aqua/config.toml`; every field is optional.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub mod apple_icons;
pub mod autostart;
pub mod paths;
pub mod schema;
pub mod shortcuts;
pub mod xkb;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

/// Liquid-glass material parameters (shared by CPU preview and GL shaders).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GlassStyle {
    /// Blur radius in logical px.
    pub blur: f32,
    /// Tint color mixed over the blurred backdrop (premultiplied in shader).
    pub tint: Rgba,
    /// Saturation boost of the backdrop (1.0 = unchanged).
    pub saturation: f32,
    /// Edge refraction strength in logical px (glass lens).
    pub refraction: f32,
    /// Width of the refracting bevel in logical px.
    pub bevel: f32,
    /// Specular rim highlight intensity.
    pub rim: f32,
    /// Corner radius in logical px.
    pub radius: f32,
    /// Drop shadow alpha.
    pub shadow: f32,
    /// Legibility cap for the backdrop luminance (1.0 = off).
    #[serde(default = "one")]
    pub max_luma: f32,
}

impl Default for GlassStyle {
    fn default() -> Self {
        Self {
            blur: 22.0,
            tint: Rgba(1.0, 1.0, 1.0, 0.18),
            saturation: 1.6,
            refraction: 9.0,
            bevel: 14.0,
            rim: 0.55,
            radius: 16.0,
            shadow: 0.18,
            max_luma: 1.0,
        }
    }
}

impl GlassStyle {
    pub fn with_radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn with_tint(mut self, t: Rgba) -> Self {
        self.tint = t;
        self
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DockItem {
    pub name: String,
    /// Desktop-entry id (e.g. "firefox") or command.
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub exec: String,
    /// Built-in icon key or path.
    #[serde(default)]
    pub icon: String,
    /// Wayland app ids that should be grouped under this item.
    #[serde(default)]
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Config {
    /// Format version of the file (older files are migrated when loaded).
    pub version: u32,
    pub wallpaper: Option<PathBuf>,
    pub font_dir: Option<PathBuf>,
    pub icon_cache: Option<PathBuf>,
    /// Allow downloading icons from the network.
    pub fetch_icons: bool,
    /// Show Linux apps as their counterparts (Firefox → Safari, Files → Finder…):
    /// "all" (default), "selected" (only `apple_icon_apps`) or "off" (real icons/names).
    pub apple_icons: String,
    /// Desktop ids replaced when `apple_icons = "selected"`.
    pub apple_icon_apps: Vec<String>,
    /// Icon & widget style: "default", "auto" (dark in Dark Mode), "dark",
    /// "clear" or "tinted" (accent colour).
    pub icon_style: String,
    /// Glass edge highlight on app icons.
    pub icon_glass: bool,
    pub menubar_height: f32,
    pub dock_icon_size: f32,
    pub dock_magnification: f32,
    pub window_radius: f32,
    /// "genie" (default) or "scale".
    pub minimize_effect: String,
    pub dark: bool,
    pub language: String,
    pub terminal: String,
    pub dock: Vec<DockItem>,
    pub glass: GlassStyle,
    /// Accent colour: multicolor, blue, purple, pink, red, orange, yellow, green, graphite.
    pub accent: String,
    /// "auto" follows the time of day, otherwise the `dark` flag.
    pub appearance: String,
    pub dock_autohide: bool,
    /// Automatically hide and show the menu bar: "fullscreen" (only over full-screen
    /// windows, default), "always" or "never".
    pub menubar_autohide: String,
    pub dock_magnify: bool,
    pub show_widgets: bool,
    pub clock_24h: bool,
    pub clock_seconds: bool,
    /// Menu bar status items hidden by the user ("input", "battery", "wifi", "search").
    pub menubar_hidden: Vec<String>,
    /// Draw a translucent bar behind the menu bar.
    pub menubar_background: bool,
    /// Show the date next to the clock.
    pub clock_date: bool,
    /// Show the lock screen when the session starts.
    pub lock_on_start: bool,
    /// Start an XWayland server for X11 apps (Steam, Discord, older Electron, IDEs).
    pub xwayland: bool,
    pub idle: IdleCfg,
    pub keyboard: KeyboardCfg,
    pub pointer: PointerCfg,
    pub outputs: Vec<OutputCfg>,
    /// Extra / overriding shortcuts, e.g. `{ keys = "super+shift+v", action = "clipboard" }`.
    pub bindings: Vec<Binding>,
    /// Sound: play alert sounds (bell).
    pub alert_sound: bool,
    /// Make Firefox / Chromium-family browsers use window controls
    /// (Firefox: GTK titlebuttons styled by Aqua; Chromium: system title bar).
    pub theme_browsers: bool,
    /// Appearance → "Style other apps like macOS": Aqua GTK 3 theme, libadwaita overrides,
    /// qt6ct palette + stylesheet.
    pub style_apps: bool,
    /// Hot corners: top-left, top-right, bottom-left, bottom-right actions ("", "mission", "desktop", "launchpad", "lock", "notifications").
    pub hot_corners: [String; 4],
    /// Commands started with the session (after the compositor is ready).
    pub autostart: Vec<String>,
    /// Remapped system shortcuts: id (see `shortcuts::SYSTEM`) → chords separated by
    /// ", " (e.g. `"super+space"`); an empty string disables the shortcut.
    pub shortcuts: std::collections::BTreeMap<String, String>,
    /// Clicking the Dock icon of a running app: "focus", "minimize"
    /// (focus, or minimise when already frontmost), "cycle" (next window of the app),
    /// "expose" (Mission Control for the app's windows) or "new" (open a new window).
    pub dock_click: String,
    /// Sidebar look of Finder, System Settings and the App Store: "floating" (an inset
    /// rounded glass island) or "solid" (full-height, edge to edge).
    pub sidebar_style: String,
    /// Toolbar buttons of Finder, System Settings and the App Store (navigation arrows,
    /// view and action capsules) drawn as raised liquid glass; false = flat.
    pub glass_controls: bool,
    /// Window traffic lights as glossy glass beads (light rim, specular highlight); false =
    /// flat discs. Applies to Aqua's own apps and to server-side window decorations.
    pub glass_traffic_lights: bool,
    /// Bounce Dock icons while apps launch.
    pub dock_bounce: bool,
    /// Running apps keep their place in the Dock (in launch order);
    /// false = most recently used apps first.
    pub dock_keep_order: bool,
    /// Screenshots: "pictures" (~/Pictures/Screenshots), "desktop" or "clipboard" (copy only).
    pub screenshot_save: String,
    /// Screenshot timer in seconds (0 = none).
    pub screenshot_timer: u32,
    /// Show the floating thumbnail after a capture.
    pub screenshot_thumbnail: bool,
    /// Include the mouse pointer in screenshots.
    pub screenshot_pointer: bool,
    /// Screen recordings: "movies" (~/Movies) or "desktop".
    pub record_save: String,
    /// Record the default microphone along with the screen.
    pub record_mic: bool,
    /// Draw a ring at every mouse click while recording.
    pub record_clicks: bool,
    /// Include the mouse pointer in recordings.
    pub record_pointer: bool,
    /// Double-click on a title bar: "zoom", "minimize" or "none".
    pub titlebar_double_click: String,
    /// Animate zoom (maximise) and full-screen transitions.
    pub animate_windows: bool,
    /// Menu Bar → show the focused app's own menus (dbusmenu global menu).
    pub global_menu: bool,
    /// Stage Manager (Control Centre / Desktop & Dock).
    pub stage_manager: bool,
    /// Desktop & Dock → "Drag windows to screen edges to tile" (macOS 15+).
    pub tile_by_drag: bool,
    /// Desktop & Dock → "Tiled windows have margins".
    pub tile_margins: bool,
    /// Accessibility → Reduce motion: no window/space animations, quick fades.
    pub reduce_motion: bool,
    /// Accessibility → Pointer size (1.0 – 4.0).
    pub cursor_size: f32,
    /// Notifications → Do Not Disturb (banners suppressed).
    pub do_not_disturb: bool,
    /// Show notification previews (banner body text).
    pub notification_previews: bool,
    /// Every display has its own Spaces (switching on one display leaves the others alone).
    pub spaces_per_output: bool,
    /// Re-blur a glass backdrop that keeps changing (video behind the dock) at most this
    /// often per second; the previous blur is reused in between (0 = every frame).
    pub blur_max_fps: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct IdleCfg {
    /// Seconds without input before the screen dims (0 = never).
    pub dim_secs: u32,
    /// Seconds before the screen turns off (0 = never).
    pub screen_off_secs: u32,
    /// Seconds before the session locks (0 = never).
    pub lock_secs: u32,
    /// Seconds before the computer sleeps (0 = never).
    pub suspend_secs: u32,
    /// Lock before sleeping / when the lid closes.
    pub lock_on_sleep: bool,
}
impl Default for IdleCfg {
    fn default() -> Self {
        Self { dim_secs: 120, screen_off_secs: 300, lock_secs: 300, suspend_secs: 0, lock_on_sleep: true }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct KeyboardCfg {
    /// XKB layouts, e.g. ["us", "ru"].
    pub layouts: Vec<String>,
    /// XKB variants (same order as layouts, may be shorter).
    pub variants: Vec<String>,
    pub model: String,
    /// Extra XKB options, e.g. "caps:escape,compose:ralt".
    pub options: String,
    /// Shortcut that cycles input sources: "ctrl+space", "super+space", "alt+shift", "caps".
    pub switch: String,
    pub repeat_delay: i32,
    pub repeat_rate: i32,
    pub numlock: bool,
}
impl Default for KeyboardCfg {
    fn default() -> Self {
        let env = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_else(|_| "us".into());
        Self {
            layouts: env.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            variants: std::env::var("XKB_DEFAULT_VARIANT")
                .map(|v| v.split(',').map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            model: String::new(),
            options: std::env::var("XKB_DEFAULT_OPTIONS").unwrap_or_default(),
            switch: "ctrl+space".into(),
            repeat_delay: 300,
            repeat_rate: 30,
            numlock: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct PointerCfg {
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    /// -1.0 ..= 1.0 libinput acceleration speed.
    pub speed: f64,
    /// "adaptive" or "flat".
    pub accel_profile: String,
    pub disable_while_typing: bool,
    /// Three/four-finger swipes and pinches (Mission Control, Spaces, Launchpad).
    pub gestures: bool,
    pub scroll_factor: f64,
    /// Secondary click with two fingers / bottom-right corner ("fingers" or "corner").
    pub secondary_click: String,
    pub mouse_natural_scroll: bool,
    pub mouse_speed: f64,
}
impl Default for PointerCfg {
    fn default() -> Self {
        Self {
            natural_scroll: true,
            tap_to_click: true,
            speed: 0.0,
            accel_profile: "adaptive".into(),
            disable_while_typing: true,
            gestures: true,
            scroll_factor: 1.0,
            secondary_click: "fingers".into(),
            mouse_natural_scroll: false,
            mouse_speed: 0.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct OutputCfg {
    /// Connector name, e.g. "eDP-1", "HDMI-A-1", or "*" for all.
    pub name: String,
    pub enabled: bool,
    /// Logical scale (fractional allowed, e.g. 1.25, 1.5). 0 = automatic from DPI.
    pub scale: f64,
    /// "WIDTHxHEIGHT@HZ" or empty for the preferred mode.
    pub mode: String,
    /// Logical position in the desktop layout; None = auto (left to right).
    pub position: Option<(i32, i32)>,
    /// "normal", "90", "180", "270", "flipped", ...
    pub transform: String,
    /// The display that shows the menu bar and Dock.
    pub primary: bool,
    /// Variable refresh rate.
    pub vrr: bool,
}
impl Default for OutputCfg {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            scale: 0.0,
            mode: String::new(),
            position: None,
            transform: "normal".into(),
            primary: false,
            vrr: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct Binding {
    pub keys: String,
    pub action: String,
}

impl Default for Config {
    fn default() -> Self {
        let item = |n: &str, app: &str, exec: &str, icon: &str, ids: &[&str]| DockItem {
            name: n.into(),
            app: app.into(),
            exec: exec.into(),
            icon: icon.into(),
            ids: ids.iter().map(|s| s.to_string()).collect(),
        };
        Self {
            version: schema::CONFIG_VERSION,
            wallpaper: None,
            font_dir: None,
            icon_cache: None,
            fetch_icons: true,
            apple_icons: "all".into(),
            apple_icon_apps: vec![],
            icon_style: "default".into(),
            icon_glass: true,
            menubar_height: 26.0,
            dock_icon_size: 54.0,
            dock_magnification: 1.5,
            window_radius: 26.0,
            minimize_effect: "genie".into(),
            dark: false,
            language: "en".into(),
            terminal: "foot|kitty|alacritty|gnome-terminal|konsole|xterm|weston-terminal".into(),
            dock: vec![
                item(
                    "Finder",
                    "finder",
                    "aqua-finder|nautilus|thunar|dolphin|pcmanfm",
                    "builtin:finder",
                    &[
                        "org.aqua.finder",
                        "aqua-finder",
                        "org.gnome.nautilus",
                        "nautilus",
                        "thunar",
                        "org.kde.dolphin",
                        "pcmanfm",
                    ],
                ),
                item("Apps", "launchpad", "", "builtin:launchpad", &[]),
                item(
                    "Safari",
                    "safari",
                    "firefox|chromium-browser|chromium|google-chrome|epiphany",
                    "builtin:safari",
                    &["firefox", "chromium", "chromium-browser", "google-chrome", "org.gnome.epiphany"],
                ),
                item("Messages", "messages", "", "builtin:messages", &["org.telegram.desktop", "signal"]),
                item(
                    "Mail",
                    "mail",
                    "thunderbird|evolution|geary",
                    "builtin:mail",
                    &["thunderbird", "org.gnome.evolution", "org.gnome.geary"],
                ),
                item("Maps", "maps", "gnome-maps", "builtin:maps", &["org.gnome.maps"]),
                item(
                    "Photos",
                    "photos",
                    "loupe|eog|shotwell",
                    "builtin:photos",
                    &["org.gnome.loupe", "org.gnome.eog", "eog"],
                ),
                item("FaceTime", "facetime", "", "builtin:facetime", &[]),
                item("Calendar", "calendar", "gnome-calendar", "builtin:calendar", &["org.gnome.calendar"]),
                item(
                    "Notes",
                    "notes",
                    "gnome-text-editor|gedit|mousepad|kate",
                    "builtin:notes",
                    &["org.gnome.texteditor", "org.gnome.gedit", "mousepad"],
                ),
                item("Music", "music", "rhythmbox|lollypop", "builtin:music", &["org.gnome.rhythmbox3", "rhythmbox"]),
                item(
                    "Terminal",
                    "terminal",
                    "foot|kitty|alacritty|gnome-terminal|konsole|xterm|weston-terminal",
                    "builtin:terminal",
                    &[
                        "foot",
                        "kitty",
                        "alacritty",
                        "org.gnome.terminal",
                        "org.kde.konsole",
                        "xterm",
                        "wayland-terminal",
                        "org.freedesktop.weston.wayland-terminal",
                        "weston-terminal",
                    ],
                ),
                item(
                    "App Store",
                    "appstore",
                    "aqua-store|gnome-software",
                    "builtin:appstore",
                    &["org.aqua.store", "aqua-store", "org.gnome.software"],
                ),
                item(
                    "System Settings",
                    "settings",
                    "aqua-settings|gnome-control-center",
                    "builtin:settings",
                    &["org.aqua.settings", "aqua-settings", "gnome-control-center", "org.gnome.settings"],
                ),
            ],
            glass: GlassStyle::default(),
            accent: "multicolor".into(),
            appearance: "light".into(),
            dock_autohide: false,
            menubar_autohide: "fullscreen".into(),
            dock_magnify: true,
            show_widgets: true,
            clock_24h: true,
            clock_seconds: false,
            menubar_hidden: vec![],
            menubar_background: false,
            clock_date: true,
            lock_on_start: false,
            xwayland: true,
            idle: IdleCfg::default(),
            keyboard: KeyboardCfg::default(),
            pointer: PointerCfg::default(),
            outputs: vec![],
            bindings: vec![],
            alert_sound: true,
            theme_browsers: true,
            style_apps: true,
            hot_corners: Default::default(),
            autostart: vec![],
            shortcuts: Default::default(),
            dock_click: "focus".into(),
            sidebar_style: "floating".into(),
            glass_controls: true,
            glass_traffic_lights: true,
            dock_bounce: true,
            dock_keep_order: true,
            screenshot_save: "pictures".into(),
            screenshot_timer: 0,
            screenshot_thumbnail: true,
            screenshot_pointer: false,
            record_save: "movies".into(),
            record_mic: false,
            record_clicks: false,
            record_pointer: true,
            titlebar_double_click: "zoom".into(),
            animate_windows: true,
            global_menu: true,
            stage_manager: false,
            tile_by_drag: true,
            tile_margins: true,
            reduce_motion: false,
            cursor_size: 1.0,
            do_not_disturb: false,
            notification_previews: true,
            spaces_per_output: true,
            blur_max_fps: 30,
        }
    }
}

/// Where packages install the JSON Schema (`aqua config-schema`), referenced by saved files.
pub const SCHEMA_URL: &str = "file:///usr/share/aqua/config.schema.json";

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("aqua/config.toml")
    }

    /// Effective config file path (`$AQUA_CONFIG` overrides the XDG location).
    pub fn file() -> PathBuf {
        std::env::var_os("AQUA_CONFIG").map(PathBuf::from).unwrap_or_else(Self::path)
    }
    /// Modification time of the config file (for live reload).
    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::file()).and_then(|m| m.modified()).ok()
    }
    /// Write the configuration atomically (used by System Settings).
    /// Full-height sidebars instead of floating islands.
    pub fn solid_sidebar(&self) -> bool {
        self.sidebar_style == "solid"
    }
    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::file();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("#:schema {SCHEMA_URL}\n# Aqua configuration — edited by System Settings\n{s}"))?;
        std::fs::rename(tmp, p)
    }
    /// Output settings for a connector (exact name, then "*").
    pub fn output(&self, name: &str) -> Option<&OutputCfg> {
        self.outputs.iter().find(|o| o.name == name).or_else(|| self.outputs.iter().find(|o| o.name == "*"))
    }
    /// Accent colour as RGB (multicolor = blue).
    pub fn accent_rgb(&self) -> (f32, f32, f32) {
        accent_rgb(&self.accent)
    }
    /// Resolved Dark Mode state for `appearance` ("auto": dark from 19:00 to 7:00 local time).
    pub fn resolve_dark(&self) -> bool {
        match self.appearance.as_str() {
            "dark" => true,
            "light" => false,
            "auto" => auto_dark_now(),
            _ => self.dark,
        }
    }
    /// Parse a configuration file's contents (missing keys take their defaults). Strict: any
    /// invalid value is an error; see [`Config::from_toml_lenient`] for recovery.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let mut table: toml::Table = toml::from_str(s)?;
        schema::migrate(&mut table);
        let mut c: Self = table.try_into()?;
        c.dark = c.resolve_dark();
        Ok(c)
    }

    /// Parse, migrate and validate: invalid values fall back to their defaults one key at a
    /// time and are reported, unknown keys are reported, out-of-range values are clamped.
    /// Only a syntax error discards the whole file.
    pub fn from_toml_lenient(s: &str) -> Result<(Self, Vec<schema::Issue>), toml::de::Error> {
        let (mut c, issues) = schema::check(s)?;
        c.dark = c.resolve_dark();
        Ok((c, issues))
    }

    /// Load the configuration, reporting every problem found.
    pub fn load_checked() -> (Self, Vec<schema::Issue>) {
        let p = Self::file();
        let Ok(s) = std::fs::read_to_string(&p) else { return (Self::default(), vec![]) };
        match Self::from_toml_lenient(&s) {
            Ok(r) => r,
            Err(e) => {
                let backup = p.with_extension("toml.bad");
                let _ = std::fs::copy(&p, &backup);
                let msg = format!("syntax error, defaults used (copy kept in {}): {e}", backup.display());
                (Self::default(), vec![schema::Issue { path: String::new(), message: msg }])
            }
        }
    }

    pub fn load() -> Self {
        let (c, issues) = Self::load_checked();
        for i in &issues {
            eprintln!("aqua: config {}: {i}", Self::file().display());
        }
        c
    }

    /// Resolve the directory containing the SF Pro fonts.
    pub fn font_dir(&self) -> PathBuf {
        if let Some(d) = &self.font_dir {
            return d.clone();
        }
        if let Some(d) = std::env::var_os("AQUA_ASSETS") {
            return PathBuf::from(d).join("fonts");
        }
        let candidates = [
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
            PathBuf::from("/usr/share/aqua/fonts"),
            PathBuf::from("/usr/local/share/aqua/fonts"),
            dirs::data_dir().unwrap_or_default().join("aqua/fonts"),
        ];
        candidates.iter().find(|p| p.exists()).cloned().unwrap_or_else(|| candidates[0].clone())
    }

    pub fn icon_cache(&self) -> PathBuf {
        self.icon_cache
            .clone()
            .unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("aqua/icons"))
    }
}

/// Fixed design metrics (logical px) of the glass look.
pub mod metrics {
    pub const MENUBAR_FONT: f32 = 13.5;
    pub const MENU_ITEM_PAD: f32 = 11.0;
    pub const DOCK_PADDING: f32 = 8.0;
    pub const DOCK_GAP: f32 = 6.0;
    pub const DOCK_BOTTOM_MARGIN: f32 = 6.0;
    pub const DOCK_RADIUS: f32 = 26.0;
    pub const TITLEBAR_HEIGHT: f32 = 38.0;
    pub const TRAFFIC_LIGHT: f32 = 13.0;
    pub const TRAFFIC_GAP: f32 = 8.5;
    /// App-icon grid: icon body occupies 824/1024 of the canvas.
    pub const ICON_BODY: f32 = 824.0 / 1024.0;
}

fn one() -> f32 {
    1.0
}

pub fn accent_rgb(name: &str) -> (f32, f32, f32) {
    match name {
        "purple" => (0.58, 0.24, 0.59),
        "pink" => (0.91, 0.27, 0.56),
        "red" => (0.93, 0.24, 0.27),
        "orange" => (0.97, 0.51, 0.11),
        "yellow" => (0.99, 0.73, 0.10),
        "green" => (0.38, 0.73, 0.27),
        "graphite" => (0.55, 0.55, 0.55),
        _ => (0.0, 0.48, 1.0),
    }
}

/// Time-of-day Dark Mode used by `appearance = "auto"`.
pub fn auto_dark_now() -> bool {
    let off = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i64
    };
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let h = (now + off).rem_euclid(86400) / 3600;
    !(7..19).contains(&h)
}

/// Animation time multiplier from `AQUA_ANIM_SLOW` (debugging; read once).
pub fn anim_slow() -> f32 {
    static SLOW: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SLOW.get_or_init(|| {
        std::env::var("AQUA_ANIM_SLOW").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0).max(0.05)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let c = Config::from_toml("").unwrap();
        let d = Config::default();
        assert_eq!(toml::to_string(&c).unwrap(), toml::to_string(&d).unwrap());
    }

    #[test]
    fn roundtrip_keeps_values() {
        let mut c = Config {
            accent: "green".into(),
            appearance: "dark".into(),
            clock_24h: false,
            menubar_hidden: vec!["tray".into()],
            ..Default::default()
        };
        c.shortcuts.insert("spotlight".into(), "super+k".into());
        let s = toml::to_string_pretty(&c).unwrap();
        let back = Config::from_toml(&s).unwrap();
        assert_eq!(back.accent, "green");
        assert!(back.dark, "appearance=dark resolves to dark");
        assert!(!back.clock_24h);
        assert_eq!(back.menubar_hidden, vec!["tray"]);
        assert_eq!(back.shortcuts.get("spotlight").map(String::as_str), Some("super+k"));
        assert_eq!(back.dock.len(), c.dock.len());
    }

    #[test]
    fn partial_file_overrides_only_given_keys() {
        let c = Config::from_toml("accent = \"red\"\n[keyboard]\nlayouts = [\"us\", \"ru\"]\n").unwrap();
        assert_eq!(c.accent, "red");
        assert_eq!(c.keyboard.layouts, vec!["us", "ru"]);
        assert_eq!(c.dock_click, Config::default().dock_click);
    }

    #[test]
    fn invalid_file_is_an_error() {
        assert!(Config::from_toml("accent = [").is_err());
        assert!(Config::from_toml("clock_24h = \"yes\"").is_err());
    }

    #[test]
    fn migrate_points_finder_to_aqua_finder() {
        let mut c = Config::default();
        let f = c.dock.iter_mut().find(|d| d.icon == "builtin:finder").expect("finder in dock");
        f.exec = "nautilus|thunar|dolphin|pcmanfm".into();
        f.ids.retain(|i| i != "aqua-finder" && i != "org.aqua.finder");
        let c = Config::from_toml(&toml::to_string(&c).unwrap()).unwrap();
        let f = c.dock.iter().find(|d| d.icon == "builtin:finder").unwrap();
        assert!(f.exec.starts_with("aqua-finder|"));
        assert_eq!(&f.ids[..2], &["org.aqua.finder".to_string(), "aqua-finder".to_string()]);
    }

    #[test]
    fn output_lookup_falls_back_to_wildcard() {
        let mut c = Config {
            outputs: vec![
                OutputCfg { name: "*".into(), scale: 2.0, ..Default::default() },
                OutputCfg { name: "HDMI-A-1".into(), scale: 1.0, ..Default::default() },
            ],
            ..Default::default()
        };
        assert_eq!(c.output("HDMI-A-1").map(|o| o.scale), Some(1.0));
        assert_eq!(c.output("eDP-1").map(|o| o.scale), Some(2.0));
        c.outputs.remove(0);
        assert!(c.output("eDP-1").is_none());
    }

    #[test]
    fn appearance_resolution() {
        let mut c = Config { appearance: "dark".into(), ..Default::default() };
        assert!(c.resolve_dark());
        c.appearance = "light".into();
        assert!(!c.resolve_dark());
        c.appearance = "custom".into();
        c.dark = true;
        assert!(c.resolve_dark());
    }

    #[test]
    fn accents() {
        assert_eq!(accent_rgb("multicolor"), (0.0, 0.48, 1.0));
        assert_eq!(accent_rgb("nonsense"), accent_rgb("blue"));
        assert_ne!(accent_rgb("red"), accent_rgb("blue"));
    }
}
