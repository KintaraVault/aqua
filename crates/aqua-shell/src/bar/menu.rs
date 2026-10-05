//! Pull-down menus (system menu, app menu, File/Edit/View/…) on glass.
use crate::{hash_of, menubar, style, Action, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum MenuKind {
    Apple,
    AppName,
    App(usize),
    Wifi,
    Battery,
    Input,
    Recent,
    /// Context menu of the Dock item at this index of `dock::visible_items`.
    Dock(usize),
    /// Context menu of the desktop (right click on the wallpaper).
    Desktop,
    /// Menu of an application's tray item, by tray key.
    Tray(String),
    /// The green title-bar button's tiling menu for a window: (window id, tiled now).
    Window(u64, bool),
}

impl MenuKind {
    /// Context menus pop up at the pointer instead of hanging from the menu bar.
    pub fn is_context(&self) -> bool {
        matches!(self, MenuKind::Dock(_) | MenuKind::Desktop | MenuKind::Window(..))
    }
}

#[derive(Default)]
pub struct MenuState {
    pub open: Option<MenuKind>,
    pub hover: Option<usize>,
    pub anchor: f32,
    /// Pointer position that opened a context menu.
    pub pos: (f32, f32),
    /// Kind drawn last frame and when it appeared (open fade / zoom).
    shown: Option<MenuKind>,
    opened: Option<std::time::Instant>,
    /// A closed menu fading out.
    fade: Option<Fade>,
}

#[derive(Clone)]
struct Fade {
    kind: MenuKind,
    /// Row that was chosen (blinks before the fade).
    chosen: Option<usize>,
    pos: (f32, f32),
    anchor: f32,
    start: std::time::Instant,
    /// Rows as they were when the menu closed: the chosen action may change the
    /// labels (e.g. "Hide Widgets" → "Show Widgets"), the ghost must not.
    frozen: (Rect, Vec<(Option<Entry>, Rect)>),
}

/// Timings (seconds): appear, selection blink (off + on), fade out.
const OPEN_MENU: f32 = 0.09;
const OPEN_CONTEXT: f32 = 0.14;
const BLINK: f32 = 0.075;
const FADE: f32 = 0.2;

fn slow() -> f32 {
    aqua_config::anim_slow()
}

impl MenuState {
    fn fade_len(f: &Fade) -> f32 {
        (if f.chosen.is_some() { 2.0 * BLINK } else { 0.0 }) + FADE
    }
}

/// Opening / closing animation still running?
pub fn animating(sh: &Shell) -> bool {
    let k = slow();
    let opening =
        sh.menu.open.is_some() && sh.menu.opened.map(|t| t.elapsed().as_secs_f32() < OPEN_CONTEXT * k).unwrap_or(true);
    let fading =
        sh.menu.fade.as_ref().map(|f| f.start.elapsed().as_secs_f32() < MenuState::fade_len(f) * k).unwrap_or(false);
    opening || fading
}

/// Start the fade-out of the menu that is open now (`chosen` = clicked row).
fn begin_fade(sh: &mut Shell, chosen: Option<usize>) {
    let Some(kind) = sh.menu.open.clone() else { return };
    if sh.cfg.reduce_motion {
        return;
    }
    let frozen = geometry(sh, &kind);
    sh.menu.fade =
        Some(Fade { kind, chosen, pos: sh.menu.pos, anchor: sh.menu.anchor, start: std::time::Instant::now(), frozen });
}

/// Close the open menu with the fade (Esc, click outside, app actions).
pub fn dismiss(sh: &mut Shell) {
    begin_fade(sh, None);
    sh.menu.open = None;
}

#[derive(Clone)]
pub struct Entry {
    pub label: String,
    pub shortcut: &'static str,
    pub action: Option<Action>,
    pub enabled: bool,
    pub extra: Extra,
}

/// Rich row decorations used by the status-item menus.
#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    None,
    /// Bold title row (not selectable); `shortcut` text shown right-aligned.
    Header,
    /// Bold title with an on/off switch on the right.
    Toggle(bool),
    /// Small grey section caption.
    Caption,
    /// Wi-Fi network: signal 0..1, secured, connected.
    Network(f32, bool, bool),
    /// Leading checkmark.
    Check(bool),
}

fn e(label: &str, shortcut: &'static str, action: Option<Action>) -> Option<Entry> {
    Some(Entry { label: label.into(), shortcut, enabled: true, action, extra: Extra::None })
}
fn d(label: &str, shortcut: &'static str) -> Option<Entry> {
    Some(Entry { label: label.into(), shortcut, enabled: false, action: None, extra: Extra::None })
}

fn x(label: &str, extra: Extra, enabled: bool, action: Option<Action>) -> Option<Entry> {
    Some(Entry { label: label.into(), shortcut: "", enabled, action, extra })
}

fn status_entries(sh: &Shell, kind: &MenuKind) -> Vec<Option<Entry>> {
    use crate::sysinfo;
    match kind {
        MenuKind::Wifi => {
            let snap = aqua_sys::snapshot();
            let on = snap.net.wifi_enabled;
            let (avail, nets) = sysinfo::wifi_networks();
            let mut v = vec![x("Wi-Fi", Extra::Toggle(on), avail, Some(Action::WifiPower(!on)))];
            if let Some(w) = &snap.net.wired {
                v.push(x(&crate::trf("Ethernet: {w}", &[("w", &w)]), Extra::Caption, false, None));
            }
            if on {
                let known: Vec<_> = nets.iter().filter(|n| n.active || n.known).collect();
                let other: Vec<_> = nets.iter().filter(|n| !n.active && !n.known).take(8).collect();
                if !known.is_empty() {
                    v.push(x("Known Networks", Extra::Caption, false, None));
                    for n in known {
                        let act =
                            if n.active { Action::WifiDisconnect } else { Action::WifiConnect(n.ssid.clone(), false) };
                        v.push(x(
                            &n.ssid,
                            Extra::Network(n.signal as f32 / 100.0, n.secure, n.active),
                            true,
                            Some(act),
                        ));
                    }
                }
                v.push(x("Other Networks", Extra::Caption, false, None));
                if other.is_empty() {
                    v.push(d(
                        if avail { "No networks found" } else { "No Wi-Fi hardware or service (NetworkManager/iwd)" },
                        "",
                    ));
                }
                for n in other {
                    v.push(x(
                        &n.ssid,
                        Extra::Network(n.signal as f32 / 100.0, n.secure, false),
                        true,
                        Some(Action::WifiConnect(n.ssid.clone(), n.secure)),
                    ));
                }
            }
            v.push(None);
            v.push(e("Wi-Fi Settings…", "", Some(Action::OpenSettings("wifi".into()))));
            v
        }
        MenuKind::Battery => {
            let p = sysinfo::power();
            let pct = p.level.map(|l| format!("{:.0}%", l * 100.0)).unwrap_or_else(|| "—".into());
            let mut v = vec![Some(Entry {
                label: "Battery".into(),
                shortcut: "",
                enabled: false,
                action: None,
                extra: Extra::Header,
            })];
            if p.level.is_some() {
                v.push(x(&crate::trf("Charge: {pct}", &[("pct", &pct)]), Extra::Caption, false, None));
                if let Some(m) = p.minutes {
                    v.push(x(
                        &format!("{}:{:02} {}", m / 60, m % 60, if p.charging { "until full" } else { "remaining" }),
                        Extra::Caption,
                        false,
                        None,
                    ));
                }
            } else {
                v.push(x("No battery installed", Extra::Caption, false, None));
            }
            v.push(x(
                &crate::trf(
                    "Power Source: {src}",
                    &[("src", &crate::tr(if p.on_ac { "Power Adapter" } else { "Battery" }))],
                ),
                Extra::Caption,
                false,
                None,
            ));
            if p.charging {
                v.push(x("Charging", Extra::Caption, false, None));
            }
            v.push(None);
            v.push(x("Low Power Mode", Extra::Check(p.low_power), true, Some(Action::LowPower(!p.low_power))));
            v.push(e("Battery Settings…", "", Some(Action::OpenSettings("battery".into()))));
            v
        }
        MenuKind::Input => {
            let mut v = vec![];
            for (i, l) in sh.layouts.iter().enumerate() {
                v.push(x(
                    sysinfo::layout_name(l),
                    Extra::Check(i == sh.layout_idx),
                    true,
                    Some(Action::SwitchLayout(i)),
                ));
            }
            v.push(None);
            v.push(e("Show Emoji & Symbols", "⌃⌘Space", Some(Action::ShowChars)));
            v.push(e("Show Keyboard Viewer", "", Some(Action::ShowKeyboardViewer)));
            v.push(e("Show Clipboard History", "⇧⌘V", Some(Action::ShowClipboard)));
            v.push(None);
            v.push(e("Open Keyboard Settings…", "", Some(Action::OpenSettings("keyboard".into()))));
            v
        }
        MenuKind::Recent => {
            let items = recent_items(10);
            let mut v = vec![x("Documents", Extra::Caption, false, None)];
            if items.is_empty() {
                v.push(d("No Recent Items", ""));
            }
            for (name, path) in items {
                v.push(e(&name, "", Some(Action::Launch(format!("xdg-open '{}'", path.replace('\'', ""))))));
            }
            v.push(None);
            v.push(e("Clear Menu", "", Some(Action::ClearRecent)));
            v
        }
        _ => vec![],
    }
}

/// Recently used files from `~/.local/share/recently-used.xbel` (GTK's list).
pub fn recent_items(n: usize) -> Vec<(String, String)> {
    let p = aqua_config::paths::home().join(".local/share/recently-used.xbel");
    let Ok(s) = std::fs::read_to_string(p) else { return vec![] };
    let mut v: Vec<(String, String, String)> = vec![];
    for chunk in s.split("<bookmark ").skip(1) {
        let attr =
            |k: &str| chunk.split(&format!("{k}=\"")).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
        let Some(href) = attr("href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        let path = percent_decode(path);
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        let modified = attr("modified").or_else(|| attr("visited")).unwrap_or_default();
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        v.push((modified, name, path));
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(n).map(|(_, a, b)| (a, b)).collect()
}

pub fn clear_recent() {
    let p = aqua_config::paths::home().join(".local/share/recently-used.xbel");
    let _ = std::fs::write(p, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\" xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n</xbel>\n");
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn xdg_dir(name: &str, fallback: &str) -> String {
    aqua_config::paths::user_dir(name, fallback).to_string_lossy().into_owned()
}

fn open_dir(path: String) -> Option<Action> {
    Some(Action::Launch(crate::dock::open_cmd(&path)))
}

/// Unique "untitled folder" path on the desktop.
fn new_folder_path() -> String {
    let desk = xdg_dir("DESKTOP", "Desktop");
    let mut p = format!("{desk}/untitled folder");
    let mut n = 2;
    while std::path::Path::new(&p).exists() {
        p = format!("{desk}/untitled folder {n}");
        n += 1;
    }
    p
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Aqua context menus show a line symbol in front of most commands (24×24 SVG markup).
fn context_icon(label: &str) -> Option<&'static str> {
    Some(match label {
        "New Folder" => "<path d=\"M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z\"/><path d=\"M12 11v5M9.5 13.5h5\"/>",
        "Open Desktop Folder" | "Open Downloads" => "<path d=\"M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z\"/>",
        "Open Terminal" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"3\"/><path d=\"M7.5 10l2.5 2-2.5 2M12 14.5h4\"/>",
        "Change Wallpaper…" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"3\"/><path d=\"M4 17l5-5 4 4 2.5-2.5L20 18\"/><circle cx=\"15.5\" cy=\"9.5\" r=\"1.5\"/>",
        "Display Settings…" => "<rect x=\"3\" y=\"4.5\" width=\"18\" height=\"12\" rx=\"2.5\"/><path d=\"M9 20h6M12 16.5V20\"/>",
        "Hide Widgets" | "Show Widgets" => "<rect x=\"4\" y=\"4\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"13\" y=\"4\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"4\" y=\"13\" width=\"16\" height=\"7\" rx=\"2\"/>",
        "Mission Control" | "Show All Windows" => "<rect x=\"3.5\" y=\"5\" width=\"7.5\" height=\"6\" rx=\"1.5\"/><rect x=\"13\" y=\"5\" width=\"7.5\" height=\"6\" rx=\"1.5\"/><rect x=\"3.5\" y=\"13\" width=\"17\" height=\"6\" rx=\"1.5\"/>",
        "Applications" | "Open Applications" => "<rect x=\"4\" y=\"4\" width=\"6.5\" height=\"6.5\" rx=\"1.6\"/><rect x=\"13.5\" y=\"4\" width=\"6.5\" height=\"6.5\" rx=\"1.6\"/><rect x=\"4\" y=\"13.5\" width=\"6.5\" height=\"6.5\" rx=\"1.6\"/><rect x=\"13.5\" y=\"13.5\" width=\"6.5\" height=\"6.5\" rx=\"1.6\"/>",
        "System Settings…" | "Desktop & Dock Settings…" => "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4 18 18M6 18l1.6-1.6M16.4 7.6 18 6\"/><circle cx=\"12\" cy=\"12\" r=\"6.3\"/>",
        "New Window" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"3\"/><path d=\"M3.5 9h17M12 12v5M9.5 14.5h5\"/>",
        "Open" => "<rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" rx=\"3.5\"/><path d=\"M9.5 14.5 15 9M10 9h5v5\"/>",
        "Remove from Dock" | "Close" => "<path d=\"M6.5 6.5l11 11M17.5 6.5l-11 11\"/>",
        "Keep in Dock" => "<path d=\"M12 4v16M4 12h16\"/>",
        "Hide" => "<path d=\"M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z\"/><circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M4.5 4.5l15 15\"/>",
        "Quit" => "<circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M9 9l6 6M15 9l-6 6\"/>",
        "Force Quit" => "<circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M12 7.5v5.5M12 16.2v.3\"/>",
        "Restore" => "<path d=\"M12 19V6M7 11l5-5 5 5\"/>",
        "Empty Trash" => "<path d=\"M4.5 7h15M9.5 7V5h5v2M6.5 7l.8 11.5a2 2 0 0 0 2 1.5h5.4a2 2 0 0 0 2-1.5L17.5 7\"/>",
        "Left" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"7\" width=\"5.5\" height=\"10\" rx=\"1\"/>",
        "Right" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"13\" y=\"7\" width=\"5.5\" height=\"10\" rx=\"1\"/>",
        "Top" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"7\" width=\"13\" height=\"4\" rx=\"1\"/>",
        "Bottom" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"13\" width=\"13\" height=\"4\" rx=\"1\"/>",
        "Top Left" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"7\" width=\"5.5\" height=\"4\" rx=\"1\"/>",
        "Top Right" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"13\" y=\"7\" width=\"5.5\" height=\"4\" rx=\"1\"/>",
        "Bottom Left" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"13\" width=\"5.5\" height=\"4\" rx=\"1\"/>",
        "Bottom Right" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"13\" y=\"13\" width=\"5.5\" height=\"4\" rx=\"1\"/>",
        "Fill" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"5.5\" y=\"7\" width=\"13\" height=\"10\" rx=\"1\"/>",
        "Center" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><rect x=\"8\" y=\"8.5\" width=\"8\" height=\"7\" rx=\"1\"/>",
        "Left & Right" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><path d=\"M12 5v14\"/>",
        "Top & Bottom" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><path d=\"M3.5 12h17\"/>",
        "Quarters" => "<rect x=\"3.5\" y=\"5\" width=\"17\" height=\"14\" rx=\"2.5\"/><path d=\"M12 5v14M3.5 12h17\"/>",
        "Enter Full Screen" => "<path d=\"M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5\"/>",
        "Return to Previous Size" => "<path d=\"M9 7 4.5 11.5 9 16\"/><path d=\"M5 11.5h9a5 5 0 0 1 0 10\"/>",
        "Options" => "<circle cx=\"6\" cy=\"12\" r=\"1\"/><circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"18\" cy=\"12\" r=\"1\"/>",
        _ => return None,
    })
}

/// The green button's menu (macOS 15/26 "Move & Resize" / "Fill & Arrange").
pub fn window_menu(id: u64, tiled: bool) -> Vec<Option<Entry>> {
    use aqua_wm::tile::{Arrange, Tile};
    let t = |tile: Tile| e(tile.label(), "", Some(Action::TileWindow(id, format!("tile-{}", tile.name()))));
    let mut v = vec![x("Move & Resize", Extra::Caption, false, None)];
    for tile in [Tile::Left, Tile::Right, Tile::Top, Tile::Bottom] {
        v.push(t(tile));
    }
    for tile in [Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight] {
        v.push(t(tile));
    }
    v.push(None);
    v.push(x("Fill & Arrange", Extra::Caption, false, None));
    v.push(t(Tile::Fill));
    v.push(t(Tile::Center));
    for a in [Arrange::LeftRight, Arrange::TopBottom, Arrange::Quarters] {
        v.push(e(a.label(), "", Some(Action::TileWindow(id, format!("arrange-{}", a.name())))));
    }
    v.push(None);
    v.push(e("Enter Full Screen", "", Some(Action::TileWindow(id, "fullscreen".into()))));
    if tiled {
        v.push(e("Return to Previous Size", "", Some(Action::TileWindow(id, "tile-restore".into()))));
    }
    v
}

/// Dock and desktop context menus.
fn context_entries(sh: &Shell, kind: &MenuKind) -> Vec<Option<Entry>> {
    use crate::dock::{self, Kind};
    match kind {
        MenuKind::Window(id, tiled) => window_menu(*id, *tiled),
        MenuKind::Desktop => {
            let desk = xdg_dir("DESKTOP", "Desktop");
            let widgets = sh.cfg.show_widgets && !sh.widgets.hidden;
            vec![
                e("New Folder", "", Some(Action::NewFolder(new_folder_path()))),
                e("Open Desktop Folder", "", open_dir(desk)),
                e("Open Terminal", "", Some(Action::OpenTerminal)),
                None,
                e("Change Wallpaper…", "", Some(Action::OpenSettings("wallpaper".into()))),
                e("Display Settings…", "", Some(Action::OpenSettings("displays".into()))),
                e(if widgets { "Hide Widgets" } else { "Show Widgets" }, "", Some(Action::ToggleWidgets)),
                None,
                e("Mission Control", "⌃↑", Some(Action::MissionControl)),
                e("Applications", "", Some(Action::ShowApps)),
                e("System Settings…", "", Some(Action::OpenSettings(String::new()))),
            ]
        }
        MenuKind::Dock(i) => {
            let items = dock::visible_items(sh);
            let Some(it) = items.get(*i) else { return vec![d("No Actions", "")] };
            match &it.kind {
                Kind::App => {
                    let wins: Vec<_> = sh.windows.iter().filter(|w| it.matches(&w.app_id)).collect();
                    let running = !wins.is_empty();
                    let mut v = vec![];
                    let mut seen: Vec<String> = vec![];
                    for w in wins.iter().take(12) {
                        let mut title = if w.title.is_empty() { it.name.clone() } else { short(&w.title, 48) };
                        // several windows with the same title (two "Home" file browsers):
                        // number the copies so each row is told apart
                        let n = seen.iter().filter(|t| **t == title).count();
                        seen.push(title.clone());
                        if n > 0 {
                            title = format!("{title} ({})", n + 1);
                        }
                        let act = if w.minimized { Action::Restore(w.id) } else { Action::FocusWindow(w.id) };
                        v.push(x(&title, Extra::Check(w.focused), true, Some(act)));
                    }
                    if running {
                        v.push(None);
                        let app_id = wins[0].app_id.clone();
                        v.push(e("New Window", "", Some(Action::NewWindow(app_id))));
                    } else if !it.exec.is_empty() {
                        v.push(e("Open", "", Some(Action::Launch(it.exec.clone()))));
                    } else {
                        v.push(d("Application not installed", ""));
                    }
                    v.push(None);
                    if it.pinned {
                        v.push(e("Remove from Dock", "", Some(Action::RemoveFromDock(it.app.clone()))));
                    } else {
                        v.push(e("Keep in Dock", "", Some(Action::KeepInDock(it.app.clone()))));
                    }
                    if !it.exec.is_empty() {
                        let at_login = sh.cfg.autostart.iter().any(|a| a == &it.exec);
                        v.push(x(
                            "Open at Login",
                            Extra::Check(at_login),
                            true,
                            Some(Action::ToggleLogin(it.exec.clone())),
                        ));
                    }
                    if running {
                        v.push(None);
                        let app_id = wins[0].app_id.clone();
                        v.push(e("Show All Windows", "", Some(Action::Activate(app_id.clone()))));
                        v.push(e("Hide", "", Some(Action::HideApp(app_id.clone()))));
                        v.push(e("Quit", "", Some(Action::QuitApp(app_id.clone()))));
                        v.push(e("Force Quit", "", Some(Action::ForceQuit(app_id))));
                    }
                    v
                }
                Kind::Minimized(id) => vec![
                    e("Restore", "", Some(Action::Restore(*id))),
                    None,
                    e("Close", "", Some(Action::CloseWindow(*id))),
                ],
                Kind::Folder => {
                    let dl = dock::downloads_dir();
                    vec![e("Open Downloads", "", Some(Action::Launch(dock::open_cmd(&dl))))]
                }
                Kind::Trash => vec![
                    e("Open", "", Some(Action::Launch(dock::TRASH_OPEN.into()))),
                    None,
                    if dock::trash_full() {
                        e("Empty Trash", "", Some(Action::EmptyTrash))
                    } else {
                        d("Empty Trash", "")
                    },
                ],
                Kind::Launchpad => vec![
                    e("Open Applications", "", Some(Action::Redraw)),
                    None,
                    e("Desktop & Dock Settings…", "", Some(Action::OpenSettings("dock".into()))),
                ],
                Kind::Separator => vec![e("Desktop & Dock Settings…", "", Some(Action::OpenSettings("dock".into())))],
            }
        }
        _ => vec![],
    }
}

/// `None` entries are separators.
pub fn entries(sh: &Shell, kind: &MenuKind) -> Vec<Option<Entry>> {
    let app = sh.active_app_name();
    let app_id = sh.focused_app().map(|w| w.app_id.clone()).unwrap_or_default();
    let has_win = !app_id.is_empty();
    if matches!(kind, MenuKind::Wifi | MenuKind::Battery | MenuKind::Input | MenuKind::Recent) {
        return status_entries(sh, kind);
    }
    if let MenuKind::Tray(key) = kind {
        return crate::tray::menu_entries(key);
    }
    if kind.is_context() {
        return context_entries(sh, kind);
    }
    let k = |label: &str, sc: &'static str, chord: &str| {
        if has_win {
            e(label, sc, Some(Action::SendKeys(chord.into())))
        } else {
            d(label, sc)
        }
    };
    match kind {
        MenuKind::Apple => vec![
            e("About This Computer", "", Some(Action::ShowAbout(String::new()))),
            None,
            e("System Settings…", "", Some(Action::OpenSettings(String::new()))),
            e(
                "App Store…",
                "",
                Some(Action::Launch(
                    "aqua-store || pamac-manager || gnome-software || plasma-discover || bauh || xdg-open https://flathub.org".into(),
                )),
            ),
            None,
            e("Recent Items", "▸", Some(Action::ShowRecent)),
            None,
            if has_win {
                e(&crate::trf("Force Quit {app}", &[("app", &app)]), "⌥⌘⎋", Some(Action::ForceQuit(app_id.clone())))
            } else {
                d("Force Quit…", "⌥⌘⎋")
            },
            None,
            e("Sleep", "", Some(Action::Sleep)),
            e("Restart…", "", Some(Action::Restart)),
            e("Shut Down…", "", Some(Action::ShutDown)),
            None,
            e("Lock Screen", "⌃⌘Q", Some(Action::Lock)),
            e(
                &crate::trf("Log Out {user}…", &[("user", &std::env::var("USER").unwrap_or_default())]),
                "⇧⌘Q",
                Some(Action::LogOut),
            ),
        ],
        MenuKind::AppName => vec![
            e(
                &crate::trf("About {app}", &[("app", &app)]),
                "",
                Some(Action::ShowAbout(if has_win { app_id.clone() } else { "finder".into() })),
            ),
            None,
            if has_win {
                e("Settings…", "⌘,", Some(Action::AppSettings))
            } else {
                e("Settings…", "⌘,", Some(Action::OpenSettings(String::new())))
            },
            None,
            if has_win {
                e(&crate::trf("Hide {app}", &[("app", &app)]), "⌘H", Some(Action::MinimizeFocused))
            } else {
                d(&crate::trf("Hide {app}", &[("app", &app)]), "⌘H")
            },
            e("Hide Others", "⌥⌘H", Some(Action::HideOthers)),
            e("Show All", "", Some(Action::BringAllToFront)),
            None,
            if app_id.is_empty() {
                d(&crate::trf("Quit {app}", &[("app", &app)]), "⌘Q")
            } else {
                e(&crate::trf("Quit {app}", &[("app", &app)]), "⌘Q", Some(Action::QuitApp(app_id)))
            },
        ],
        MenuKind::App(i) => {
            if let Some(v) = crate::tray::app_menu_entries(*i) {
                return v;
            }
            let menus = menubar::app_menus(&app);
            let menu = menus.get(*i).copied().unwrap_or("");
            if has_win && app == "Finder" {
                let f = match menu {
                    "File" => vec![
                        k("New Finder Window", "⌘N", "ctrl+n"),
                        k("New Folder", "⇧⌘N", "ctrl+shift+n"),
                        k("New Folder with Selection", "⌃⌘N", "ctrl+super+n"),
                        k("New Smart Folder", "⌥⌘N", "ctrl+alt+n"),
                        k("New Tab", "⌘T", "ctrl+t"),
                        k("Open", "⌘O", "ctrl+o"),
                        k("Close Tab", "⌘W", "ctrl+w"),
                        k("Close Window", "⌥⌘W", "ctrl+alt+w"),
                        None,
                        k("Get Info", "⌘I", "ctrl+i"),
                        k("Get Summary Info", "⌃⌘I", "ctrl+super+i"),
                        k("Rename", "↩", "return"),
                        None,
                        k("Duplicate", "⌘D", "ctrl+d"),
                        k("Make Alias", "⌃⌘A", "ctrl+super+a"),
                        k("Quick Look", "⌘Y", "ctrl+y"),
                        k("Show Original", "⌘R", "ctrl+r"),
                        k("Add to Sidebar", "⌃⌘T", "ctrl+super+t"),
                        None,
                        k("Move to Trash", "⌘⌫", "ctrl+backspace"),
                        k("Eject", "⌘E", "ctrl+e"),
                        None,
                        k("Find", "⌘F", "ctrl+f"),
                    ],
                    "Edit" => vec![
                        k("Undo", "⌘Z", "ctrl+z"),
                        k("Redo", "⇧⌘Z", "ctrl+shift+z"),
                        None,
                        k("Cut", "⌘X", "ctrl+x"),
                        k("Copy", "⌘C", "ctrl+c"),
                        k("Paste", "⌘V", "ctrl+v"),
                        k("Move Item Here", "⌥⌘V", "ctrl+alt+v"),
                        k("Copy as Pathname", "⌥⌘C", "ctrl+alt+c"),
                        k("Select All", "⌘A", "ctrl+a"),
                        k("Deselect All", "⌥⌘A", "ctrl+alt+a"),
                        None,
                        e("Clipboard History", "⇧⌘V", Some(Action::ShowClipboard)),
                        e("Emoji & Symbols", "⌃⌘Space", Some(Action::ShowChars)),
                    ],
                    "View" => vec![
                        k("as Icons", "⌘1", "ctrl+1"),
                        k("as List", "⌘2", "ctrl+2"),
                        k("as Columns", "⌘3", "ctrl+3"),
                        k("as Gallery", "⌘4", "ctrl+4"),
                        None,
                        k("Use Groups", "⌃⌘0", "ctrl+super+0"),
                        k("Clean Up", "", "ctrl+super+alt+shift+u"),
                        None,
                        k("Show Tab Bar", "⇧⌘T", "ctrl+shift+t"),
                        k("Show Path Bar", "⌥⌘P", "ctrl+alt+p"),
                        k("Show Status Bar", "⌘/", "ctrl+/"),
                        k("Show Sidebar", "⌃⌘S", "ctrl+super+s"),
                        k("Show Preview", "⇧⌘P", "ctrl+shift+p"),
                        None,
                        k("Show Hidden Files", "⇧⌘.", "ctrl+shift+."),
                        k("Show View Options", "⌘J", "ctrl+j"),
                        k("Customize Toolbar…", "", "ctrl+super+alt+shift+b"),
                        None,
                        e("Enter Full Screen", "⌃⌘F", Some(Action::FullscreenFocused)),
                    ],
                    "Go" => vec![
                        k("Back", "⌘[", "ctrl+["),
                        k("Forward", "⌘]", "ctrl+]"),
                        k("Enclosing Folder", "⌘↑", "ctrl+#103"),
                        None,
                        k("Recents", "⇧⌘F", "ctrl+shift+f"),
                        k("Documents", "⇧⌘O", "ctrl+shift+o"),
                        k("Desktop", "⇧⌘D", "ctrl+shift+d"),
                        k("Downloads", "⌥⌘L", "ctrl+alt+l"),
                        k("Home", "⇧⌘H", "ctrl+shift+h"),
                        k("Computer", "⇧⌘C", "ctrl+shift+c"),
                        k("Applications", "⇧⌘A", "ctrl+shift+a"),
                        None,
                        k("Go to Folder…", "⇧⌘G", "ctrl+shift+g"),
                        k("Connect to Server…", "⌘K", "ctrl+k"),
                    ],
                    "Window" => vec![
                        e("Minimize", "⌘M", Some(Action::MinimizeFocused)),
                        e("Zoom", "", Some(Action::ZoomFocused)),
                        None,
                        k("Show Previous Tab", "⌃⇧⇥", "ctrl+shift+tab"),
                        k("Show Next Tab", "⌃⇥", "ctrl+tab"),
                        k("Move Tab to New Window", "", "ctrl+super+alt+shift+n"),
                        k("Merge All Windows", "", "ctrl+super+alt+shift+m"),
                        None,
                        e("Bring All to Front", "", Some(Action::BringAllToFront)),
                    ],
                    _ => vec![],
                };
                if !f.is_empty() {
                    return f;
                }
            }
            match menu {
                "File" => vec![
                    if has_win {
                        e("New Window", "⌘N", Some(Action::NewWindow(app_id.clone())))
                    } else {
                        e(
                            "New Finder Window",
                            "⌘N",
                            open_dir(aqua_config::paths::home().to_string_lossy().into_owned()),
                        )
                    },
                    k("Open…", "⌘O", "ctrl+o"),
                    k("Save", "⌘S", "ctrl+s"),
                    k("Print…", "⌘P", "ctrl+p"),
                    None,
                    if has_win {
                        e("Close Window", "⌘W", Some(Action::CloseFocused))
                    } else {
                        d("Close Window", "⌘W")
                    },
                ],
                "Edit" => vec![
                    k("Undo", "⌘Z", "ctrl+z"),
                    k("Redo", "⇧⌘Z", "ctrl+shift+z"),
                    None,
                    k("Cut", "⌘X", "ctrl+x"),
                    k("Copy", "⌘C", "ctrl+c"),
                    k("Paste", "⌘V", "ctrl+v"),
                    k("Select All", "⌘A", "ctrl+a"),
                    None,
                    k("Find…", "⌘F", "ctrl+f"),
                    None,
                    e("Clipboard History", "⇧⌘V", Some(Action::ShowClipboard)),
                    e("Emoji & Symbols", "⌃⌘Space", Some(Action::ShowChars)),
                ],
                "View" => vec![
                    k("Zoom In", "⌘+", "ctrl+equal"),
                    k("Zoom Out", "⌘−", "ctrl+minus"),
                    k("Actual Size", "⌘0", "ctrl+0"),
                    None,
                    if has_win {
                        e("Enter Full Screen", "⌃⌘F", Some(Action::FullscreenFocused))
                    } else {
                        d("Enter Full Screen", "⌃⌘F")
                    },
                ],
                "Go" => vec![
                    e("Recents", "⇧⌘F", Some(Action::ShowRecent)),
                    e("Documents", "⇧⌘O", open_dir(xdg_dir("DOCUMENTS", "Documents"))),
                    e("Desktop", "⇧⌘D", open_dir(xdg_dir("DESKTOP", "Desktop"))),
                    e("Downloads", "⌥⌘L", open_dir(xdg_dir("DOWNLOAD", "Downloads"))),
                    e("Home", "⇧⌘H", open_dir(aqua_config::paths::home().to_string_lossy().into_owned())),
                    e("Computer", "⇧⌘C", open_dir("/".into())),
                    None,
                    e("Applications", "⇧⌘A", Some(Action::ShowApps)),
                    e("Utilities", "⇧⌘U", Some(Action::Launch("xdg-open /usr/share/applications".into()))),
                ],
                "Window" => vec![
                    if has_win { e("Minimize", "⌘M", Some(Action::MinimizeFocused)) } else { d("Minimize", "⌘M") },
                    if has_win { e("Zoom", "", Some(Action::ZoomFocused)) } else { d("Zoom", "") },
                    None,
                    e("Bring All to Front", "", Some(Action::BringAllToFront)),
                ],
                _ => vec![e(&format!("{app} Help"), "", Some(Action::Help(app_id.clone(), app.clone())))],
            }
        }
        _ => vec![],
    }
}

const ROW: f32 = 24.0;
const SEP: f32 = 11.0;
const PAD: f32 = 6.0;

fn geometry(sh: &Shell, kind: &MenuKind) -> (Rect, Vec<(Option<Entry>, Rect)>) {
    let ents = entries(sh, kind);
    let f = &sh.fonts;
    let status = matches!(kind, MenuKind::Wifi | MenuKind::Battery | MenuKind::Input | MenuKind::Recent);
    let mut w: f32 = if status { 280.0 } else { 200.0 };
    for en in ents.iter().flatten() {
        w = w.max(f.measure(&en.label, 13.5, Weight::Regular) + if kind.is_context() { 110.0 } else { 90.0 });
    }
    let total_h: f32 = ents
        .iter()
        .map(|en| match en {
            None => SEP,
            Some(Entry { extra: Extra::Header | Extra::Toggle(_) | Extra::Network(..), .. }) => 30.0,
            Some(Entry { extra: Extra::Caption, .. }) => 22.0,
            _ => ROW,
        })
        .sum::<f32>()
        + 2.0 * PAD;
    let (x, top) = match kind {
        MenuKind::Dock(_) => {
            let x = (sh.menu.pos.0 - w / 2.0).clamp(6.0, (sh.w - w - 6.0).max(6.0));
            let y = (crate::dock::top(sh) - total_h - 10.0).max(sh.cfg.menubar_height + 4.0);
            (x, y)
        }
        MenuKind::Desktop | MenuKind::Window(..) => {
            let (px, py) = sh.menu.pos;
            let x = if px + w + 6.0 > sh.w { (px - w).max(6.0) } else { px };
            let y = if py + total_h + 6.0 > sh.h { (py - total_h).max(sh.cfg.menubar_height + 4.0) } else { py };
            (x, y)
        }
        _ => (sh.menu.anchor.min(sh.w - w - 6.0), sh.cfg.menubar_height + 2.0),
    };
    let mut y = top + PAD;
    let mut rows = vec![];
    for en in ents {
        let h = match &en {
            None => SEP,
            Some(Entry { extra: Extra::Header | Extra::Toggle(_), .. }) => 30.0,
            Some(Entry { extra: Extra::Caption, .. }) => 22.0,
            Some(Entry { extra: Extra::Network(..), .. }) => 30.0,
            _ => ROW,
        };
        rows.push((en, Rect::new(x + PAD, y, w - 2.0 * PAD, h)));
        y += h;
    }
    (Rect::new(x, top, w, y + PAD - top), rows)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    let Some(kind) = sh.menu.open.clone() else {
        sh.menu.shown = None;
        return None;
    };
    if sh.menu.shown.is_none() {
        sh.menu.opened = Some(std::time::Instant::now());
    }
    sh.menu.shown = Some(kind.clone());
    let hover = sh.menu.hover;
    let mut l = render(sh, &kind, hover, LayerId::Menu, None);
    if !sh.cfg.reduce_motion {
        let dur = if kind.is_context() { OPEN_CONTEXT } else { OPEN_MENU } * slow();
        let t = sh.menu.opened.map(|t| (t.elapsed().as_secs_f32() / dur).min(1.0)).unwrap_or(1.0);
        let e = 1.0 - (1.0 - t).powi(3);
        l.opacity = e;
        if kind.is_context() {
            l.zoom = 0.94 + 0.06 * e;
        }
    }
    Some(l)
}

/// The fading ghost of the last closed menu.
pub fn fade_layer(sh: &mut Shell) -> Option<Layer> {
    let f = sh.menu.fade.clone()?;
    let k = slow();
    let el = f.start.elapsed().as_secs_f32() / k;
    if el >= MenuState::fade_len(&f) {
        sh.menu.fade = None;
        return None;
    }
    let (open, pos, anchor) = (sh.menu.open.take(), sh.menu.pos, sh.menu.anchor);
    sh.menu.pos = f.pos;
    sh.menu.anchor = f.anchor;
    let blink = f.chosen.is_some() && el < 2.0 * BLINK;
    let hover = if blink && el < BLINK { None } else { f.chosen };
    let mut l = render(sh, &f.kind, hover, LayerId::MenuFade, Some(f.frozen.clone()));
    sh.menu.open = open;
    sh.menu.pos = pos;
    sh.menu.anchor = anchor;
    let fade_t = ((el - if f.chosen.is_some() { 2.0 * BLINK } else { 0.0 }) / FADE).clamp(0.0, 1.0);
    l.opacity = 1.0 - fade_t * fade_t * (3.0 - 2.0 * fade_t);
    Some(l)
}

fn render(
    sh: &mut Shell,
    kind: &MenuKind,
    hover: Option<usize>,
    id: LayerId,
    frozen: Option<(Rect, Vec<(Option<Entry>, Rect)>)>,
) -> Layer {
    let kind = kind.clone();
    let (rect, rows) = frozen.unwrap_or_else(|| geometry(sh, &kind));
    let dark = sh.style.dark;
    let glass = style::glass_menu(&sh.cfg.glass, dark);
    let sig: Vec<String> =
        rows.iter().map(|(e, _)| e.as_ref().map(|e| format!("{}{:?}", e.label, e.extra)).unwrap_or_default()).collect();
    let key = hash_of(&(format!("{kind:?}"), hover, rect.x as i32, rect.y as i32, sh.active_app_name(), sig, dark));
    let (pm, serial) = sh.cached(id, key, rect.w, rect.h, |c, sh| {
        let f = sh.fonts.clone();
        for (i, (en, r)) in rows.iter().enumerate() {
            let r = r.translate(-rect.x, -rect.y);
            match en {
                None => c.fill_rect(Rect::new(r.x + 8.0, r.cy() - 0.5, r.w - 16.0, 1.0), style::separator(dark)),
                Some(en) if en.extra != Extra::None => {
                    let fg = style::text_primary(dark);
                    let fg2 = style::text_secondary(dark);
                    let hot =
                        hover == Some(i) && en.enabled && matches!(en.extra, Extra::Network(..) | Extra::Check(_));
                    if hot {
                        c.fill_rrect(r, 7.0, if dark { rgba(255, 255, 255, 0.12) } else { rgba(0, 0, 0, 0.07) });
                    }
                    match en.extra {
                        Extra::Header | Extra::Toggle(_) => {
                            c.text_in(
                                &f,
                                Rect::new(r.x + 12.0, r.y, r.w - 20.0, r.h),
                                0.0,
                                13.5,
                                Weight::Bold,
                                fg,
                                &en.label,
                            );
                            if let Extra::Toggle(on) = en.extra {
                                let sw = Rect::new(r.right() - 48.0, r.cy() - 11.0, 38.0, 22.0);
                                c.fill_rrect(
                                    sw,
                                    11.0,
                                    if on {
                                        style::accent(1.0)
                                    } else if dark {
                                        rgba(255, 255, 255, 0.2)
                                    } else {
                                        rgba(0, 0, 0, 0.12)
                                    },
                                );
                                let kx = if on { sw.right() - 11.0 } else { sw.x + 11.0 };
                                c.fill_circle(kx, sw.cy(), 9.0, rgba(255, 255, 255, 1.0));
                            }
                        }
                        Extra::Caption => {
                            c.text_in(
                                &f,
                                Rect::new(r.x + 12.0, r.y, r.w - 20.0, r.h),
                                0.0,
                                12.0,
                                Weight::Semibold,
                                fg2,
                                &en.label,
                            );
                        }
                        Extra::Network(sig, secure, active) => {
                            let b = Rect::new(r.x + 8.0, r.cy() - 12.0, 24.0, 24.0);
                            c.fill_circle(
                                b.cx(),
                                b.cy(),
                                12.0,
                                if active {
                                    style::accent(1.0)
                                } else if dark {
                                    rgba(255, 255, 255, 0.16)
                                } else {
                                    rgba(0, 0, 0, 0.08)
                                },
                            );
                            let gc = if active { rgba(255, 255, 255, 1.0) } else { fg };
                            let a = if sig > 0.66 {
                                1.0
                            } else if sig > 0.33 {
                                0.75
                            } else {
                                0.5
                            };
                            aqua_gfx::symbols::wifi(
                                c,
                                Rect::new(b.cx() - 7.0, b.cy() - 6.0, 14.0, 11.5),
                                aqua_gfx::Color::from_rgba(gc.red(), gc.green(), gc.blue(), gc.alpha() * a)
                                    .unwrap_or(gc),
                            );
                            c.text_in(
                                &f,
                                Rect::new(r.x + 42.0, r.y, r.w - 80.0, r.h),
                                0.0,
                                13.5,
                                Weight::Regular,
                                fg,
                                &en.label,
                            );
                            if secure {
                                let lx = r.right() - 22.0;
                                let ly = r.cy() - 1.0;
                                c.stroke_rrect(Rect::new(lx + 2.0, ly - 6.0, 6.0, 8.0), 3.0, fg2, 1.4);
                                c.fill_rrect(Rect::new(lx, ly - 1.0, 10.0, 7.5), 1.5, fg2);
                            }
                        }
                        Extra::Check(on) => {
                            if on {
                                aqua_gfx::symbols::checkmark(
                                    c,
                                    Rect::new(r.x + 8.0, r.cy() - 5.0, 11.0, 10.0),
                                    fg,
                                    1.8,
                                );
                            }
                            c.text_in(
                                &f,
                                Rect::new(r.x + 26.0, r.y, r.w - 30.0, r.h),
                                0.0,
                                13.5,
                                Weight::Regular,
                                fg,
                                &en.label,
                            );
                        }
                        Extra::None => {}
                    }
                }
                Some(en) => {
                    let hot = hover == Some(i) && en.enabled;
                    if hot {
                        c.fill_rrect(r, 7.0, style::accent(0.92));
                    }
                    let col = if hot {
                        rgba(255, 255, 255, 1.0)
                    } else if en.enabled {
                        style::text_primary(dark)
                    } else if dark {
                        rgba(255, 255, 255, 0.3)
                    } else {
                        rgba(60, 60, 67, 0.32)
                    };
                    let mut tx = r.x + 12.0;
                    if kind.is_context() {
                        tx = r.x + 32.0;
                        if let Some(svg) = context_icon(&en.label) {
                            let px = (15.0 * c.scale).round() as u32;
                            let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                            if let Some(ic) = aqua_icons::theme::symbol(
                                svg,
                                px,
                                [to8(col.red()), to8(col.green()), to8(col.blue()), to8(col.alpha())],
                            ) {
                                c.blit(&ic, r.x + 9.0, r.cy() - 7.5);
                            }
                        }
                    }
                    c.text_in(
                        &f,
                        Rect::new(tx, r.y, r.right() - tx - 8.0, r.h),
                        0.0,
                        13.5,
                        Weight::Regular,
                        col,
                        &en.label,
                    );
                    if !en.shortcut.is_empty() {
                        let sc = if hot { rgba(255, 255, 255, 0.85) } else { style::text_secondary(dark) };
                        shortcut(c, &f, Rect::new(r.x, r.y, r.w - 10.0, r.h), en.shortcut, sc);
                    }
                }
            }
        }
    });
    Layer { id, rect, glass: Some(glass), tiles: vec![], content: pm, serial, opacity: 1.0, zoom: 1.0 }
}

/// Draw keyboard shortcut glyph string right-aligned.
fn shortcut(c: &mut aqua_gfx::Canvas, f: &aqua_gfx::Fonts, r: Rect, s: &str, col: aqua_gfx::Color) {
    let size = 13.0;
    let gw = 13.0;
    let total: f32 = s
        .chars()
        .map(|ch| if "⌘⇧⌥⌃⎋".contains(ch) { gw } else { f.measure(&ch.to_string(), size, Weight::Regular) + 1.0 })
        .sum();
    let mut x = r.right() - total;
    for ch in s.chars() {
        let g = Rect::new(x + 1.5, r.cy() - 5.5, 10.0, 11.0);
        match ch {
            '⌘' => mod_cmd(c, g, col),
            '⇧' => mod_shift(c, g, col),
            '⌥' => mod_opt(c, g, col),
            '⌃' => mod_ctrl(c, g, col),
            '⎋' => mod_esc(c, g, col),
            _ => {
                let w = f.measure(&ch.to_string(), size, Weight::Regular) + 1.0;
                c.text(f, x, r.cy() + 4.6, size, Weight::Regular, col, &ch.to_string());
                x += w;
                continue;
            }
        }
        x += gw;
    }
}

use aqua_gfx::tiny_skia::PathBuilder;
fn stroke(c: &mut aqua_gfx::Canvas, pb: PathBuilder, col: aqua_gfx::Color) {
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &aqua_gfx::canvas::solid(col), 1.25);
    }
}
fn mod_cmd(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let (x0, y0, x1, y1) = (r.x + 3.0, r.y + 3.0, r.right() - 3.0, r.bottom() - 3.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x0, y1);
    pb.move_to(x1, y0);
    pb.line_to(x1, y1);
    pb.move_to(x0, y0);
    pb.line_to(x1, y0);
    pb.move_to(x0, y1);
    pb.line_to(x1, y1);
    for (cx, cy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        pb.push_circle(cx + if cx == x0 { -1.5 } else { 1.5 }, cy + if cy == y0 { -1.5 } else { 1.5 }, 1.6);
    }
    stroke(c, pb, col);
}
fn mod_shift(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.cx(), r.y);
    pb.line_to(r.right(), r.cy());
    pb.line_to(r.x + r.w * 0.72, r.cy());
    pb.line_to(r.x + r.w * 0.72, r.bottom());
    pb.line_to(r.x + r.w * 0.28, r.bottom());
    pb.line_to(r.x + r.w * 0.28, r.cy());
    pb.line_to(r.x, r.cy());
    pb.close();
    stroke(c, pb, col);
}
fn mod_opt(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.x, r.y + 2.0);
    pb.line_to(r.x + r.w * 0.35, r.y + 2.0);
    pb.line_to(r.x + r.w * 0.7, r.bottom() - 1.0);
    pb.line_to(r.right(), r.bottom() - 1.0);
    pb.move_to(r.x + r.w * 0.6, r.y + 2.0);
    pb.line_to(r.right(), r.y + 2.0);
    stroke(c, pb, col);
}
fn mod_ctrl(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.x + 1.0, r.cy());
    pb.line_to(r.cx(), r.y + 2.0);
    pb.line_to(r.right() - 1.0, r.cy());
    stroke(c, pb, col);
}
fn mod_esc(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let mut pb = PathBuilder::new();
    pb.push_circle(r.cx() + 0.5, r.cy() + 0.5, 4.2);
    pb.move_to(r.x, r.y);
    pb.line_to(r.x + 4.0, r.y + 4.0);
    stroke(c, pb, col);
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    let Some(kind) = sh.menu.open.clone() else { return };
    if y < sh.cfg.menubar_height && !kind.is_context() {
        for (it, r) in menubar::layout(sh) {
            if r.contains(x, y) {
                let k = match it {
                    menubar::Item::Apple => Some(MenuKind::Apple),
                    menubar::Item::AppName => Some(MenuKind::AppName),
                    menubar::Item::Menu(i) => Some(MenuKind::App(i)),
                    _ => None,
                };
                if let Some(k) = k {
                    if k != kind {
                        menubar::about_to_open(&k);
                        sh.menu.open = Some(k);
                        sh.menu.anchor = r.x;
                        sh.menu.hover = None;
                    }
                }
            }
        }
        return;
    }
    let (_, rows) = geometry(sh, &kind);
    sh.menu.hover = rows.iter().position(|(en, r)| en.is_some() && r.contains(x, y));
}

/// Returns Some when a menu was open (click consumed).
pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let kind = sh.menu.open.clone()?;
    if y < sh.cfg.menubar_height && !kind.is_context() {
        return None;
    }
    let (rect, rows) = geometry(sh, &kind);
    if !rect.contains(x, y) {
        dismiss(sh);
        return Some(vec![Action::Redraw]);
    }
    for (i, (en, r)) in rows.into_iter().enumerate() {
        if let Some(en) = en {
            if r.contains(x, y) && en.enabled {
                let mut v = vec![Action::Redraw];
                if let Extra::Toggle(_) = en.extra {
                } else {
                    let plain = en.extra == Extra::None || matches!(en.extra, Extra::Check(_) | Extra::Network(..));
                    begin_fade(sh, plain.then_some(i));
                    sh.menu.open = None;
                }
                if let Some(a) = en.action {
                    v.push(a);
                }
                return Some(v);
            }
        }
    }
    Some(vec![Action::Redraw])
}

/// Keyboard navigation of an open menu: ↑/↓ move the highlight, ↩ chooses (with the
/// blink), ⎋ closes.
pub fn key(sh: &mut Shell, key: Option<crate::Key>) -> (bool, Vec<Action>) {
    use crate::Key;
    let Some(kind) = sh.menu.open.clone() else { return (false, vec![]) };
    let (_, rows) = geometry(sh, &kind);
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (e, _))| {
            e.as_ref()
                .map(|e| {
                    e.enabled
                        && matches!(e.extra, Extra::None | Extra::Check(_) | Extra::Network(..) | Extra::Toggle(_))
                })
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();
    match key {
        Some(Key::Escape) => {
            dismiss(sh);
            (true, vec![Action::Redraw])
        }
        Some(Key::Down) | Some(Key::Up) if !selectable.is_empty() => {
            let cur = sh.menu.hover.and_then(|h| selectable.iter().position(|&i| i == h));
            let n = selectable.len();
            let next = match (cur, key) {
                (None, Some(Key::Down)) => 0,
                (None, _) => n - 1,
                (Some(c), Some(Key::Down)) => (c + 1) % n,
                (Some(c), _) => (c + n - 1) % n,
            };
            sh.menu.hover = Some(selectable[next]);
            (true, vec![Action::Redraw])
        }
        Some(Key::Enter) => {
            let Some(h) = sh.menu.hover else { return (true, vec![]) };
            let Some((_, r)) = rows.get(h) else { return (true, vec![]) };
            let (x, y) = (r.cx(), r.cy());
            (true, click(sh, x, y).unwrap_or_default())
        }
        _ => (true, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_menu_actions_are_valid_tile_commands() {
        use aqua_wm::tile::{Arrange, Tile};
        for tiled in [false, true] {
            let menu = window_menu(7, tiled);
            let acts: Vec<String> = menu
                .iter()
                .flatten()
                .filter_map(|e| match &e.action {
                    Some(Action::TileWindow(id, what)) => {
                        assert_eq!(*id, 7);
                        Some(what.clone())
                    }
                    Some(other) => panic!("unexpected action {other:?}"),
                    None => None,
                })
                .collect();
            for a in &acts {
                let ok = a == "fullscreen"
                    || a == "tile-restore"
                    || a.strip_prefix("tile-").and_then(Tile::parse).is_some()
                    || a.strip_prefix("arrange-").and_then(Arrange::parse).is_some();
                assert!(ok, "{a} is not understood by the compositor");
            }
            assert_eq!(acts.iter().filter(|a| a.starts_with("tile-") && *a != "tile-restore").count(), 10);
            assert_eq!(acts.contains(&"tile-restore".to_string()), tiled);
            // captions are not clickable
            for e in menu.iter().flatten().filter(|e| e.extra == Extra::Caption) {
                assert!(e.action.is_none() && !e.enabled);
            }
        }
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%D0%9F%D1%80"), "Пр");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%2"), "%zz%2");
        assert_eq!(percent_decode("%é1"), "%é1", "multi-byte characters after % must not panic");
    }

    #[test]
    fn shortening() {
        assert_eq!(short("hello", 10), "hello");
        assert_eq!(short("hello world", 6), "hello…");
        assert_eq!(short("привет мир", 4), "при…");
        assert_eq!(short("abc", 0), "…");
    }

    #[test]
    fn context_icons_exist_for_desktop_menu() {
        for l in ["New Folder", "Open Terminal", "Change Wallpaper…"] {
            assert!(context_icon(l).is_some(), "{l}");
        }
        assert!(context_icon("No Such Command").is_none());
    }
}
