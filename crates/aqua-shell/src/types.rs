//! Shell data types shared with the compositor: layers, window info, actions, keys.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerId {
    Widgets(u8),
    MenuBar,
    Dock,
    DockTooltip,
    Launchpad,
    Menu,
    /// A menu fading out after it closed (selection blink, dismissal).
    MenuFade,
    ControlCenter,
    Spotlight,
    /// The morphing glass shape behind Spotlight.
    SpotlightGlass,
    /// Spotlight's round filter buttons (Applications, Files, Actions, Clipboard).
    SpotlightButton(u8),
    /// Icon dragged out of the Dock (follows the pointer).
    DockDrag,
    Switcher,
    Banner,
    NotificationCenter,
    Mission,
    MissionNames,
    MissionBar,
    DockBadges,
    Lock,
    Clipboard,
    Alert,
    Hud,
    CharViewer,
    KeyboardViewer,
    /// Screenshot interface pieces (selection, toolbar, thumbnail).
    Shot(u8),
}

/// One drawable shell surface.
#[derive(Clone)]
pub struct Layer {
    pub id: LayerId,
    /// Logical rectangle on the output.
    pub rect: Rect,
    /// Glass material drawn behind `content` (None = content only).
    pub glass: Option<GlassStyle>,
    /// Extra glass "sub-panels" inside the layer (logical rects relative to output), e.g.
    /// control-centre tiles.
    pub tiles: Vec<(Rect, GlassStyle)>,
    /// Premultiplied RGBA at physical resolution (rect * scale).
    pub content: Arc<Pixmap>,
    /// Changes whenever `content` changes (for texture upload caching).
    pub serial: u64,
    pub opacity: f32,
    /// Uniform scale around the layer centre (open/close animations).
    pub zoom: f32,
}

/// Info about a toplevel the compositor manages (fed into the shell each frame).
#[derive(Clone, Debug, Default, Hash, PartialEq)]
pub struct WindowInfo {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Launch(String),
    /// Activate (raise/unminimize) windows of an app id.
    Activate(String),
    /// Dock icon of a running app clicked: (app id, launch command).
    DockClick(String, String),
    /// Un-minimise one specific window (Dock thumbnail click).
    Restore(u64),
    CloseFocused,
    MinimizeFocused,
    ZoomFocused,
    QuitApp(String),
    HideOthers,
    LogOut,
    Screenshot,
    /// System appearance changed (true = Dark Mode).
    SetDark(bool),
    Redraw,
    /// Lock the screen now.
    Lock,
    /// Suspend (locks first when configured).
    Sleep,
    Restart,
    ShutDown,
    /// Confirmed power actions (the alert's default button / countdown): skip the
    /// confirmation that `LogOut`/`Restart`/`ShutDown` open.
    LogOutNow,
    RestartNow,
    ShutDownNow,
    /// Capture (part of) the screen: save as PNG and copy to the clipboard.
    ScreenshotTake(screenshot::Target),
    /// Screenshot toolbar options changed (persist them).
    ScreenshotOptions,
    /// Open the screenshot interface: "ui" (⌘⇧5 toolbar), "area", "window",
    /// "record" (toolbar in screen-recording mode).
    ScreenshotUi(String),
    /// Start recording (part of) the screen to a video file.
    RecordStart(screenshot::Target),
    /// Stop the running screen recording.
    RecordStop,
    /// Put text on the clipboard (Spotlight calculator result).
    CopyText(String),
    /// Submit the lock-screen password.
    Unlock(String),
    /// Make a clipboard-history entry current; `true` = also paste it.
    ClipboardUse(u64, bool),
    ClipboardClear,
    /// Open a System Settings pane ("appearance", "wifi", "keyboard", …).
    OpenSettings(String),
    /// Send a key chord to the focused client, e.g. "ctrl+c", "ctrl+shift+z".
    SendKeys(String),
    /// Type text into the focused client (text-input / virtual keyboard).
    TypeText(String),
    /// Press one evdev key code in the focused client.
    TypeKey(u32),
    /// Activate keyboard layout index.
    SwitchLayout(usize),
    /// Show every window of every app (Window ▸ Bring All to Front).
    BringAllToFront,
    /// Open a new window of the focused application.
    NewWindow(String),
    /// Toggle the app's fullscreen state.
    FullscreenFocused,
    /// Ask the focused client to open its preferences (⌘,).
    AppSettings,
    /// Audible alert.
    Beep,
    /// Do Not Disturb toggled in Control Centre.
    SetFocusMode(bool),
    ForceQuit(String),
    ForceQuitConfirmed(String),
    Help(String, String),
    ShowAbout(String),
    ShowChars,
    ShowKeyboardViewer,
    ShowClipboard,
    /// Open the Applications panel.
    ShowApps,
    ShowRecent,
    ClearRecent,
    WifiPower(bool),
    WifiConnect(String, bool),
    WifiDisconnect,
    LowPower(bool),
    /// Raise + focus one specific window.
    FocusWindow(u64),
    /// Close one specific window.
    CloseWindow(u64),
    /// Minimise every window of an app (Dock ▸ Hide).
    HideApp(String),
    /// Open the configured terminal.
    OpenTerminal,
    MissionControl,
    KeepInDock(String),
    RemoveFromDock(String),
    /// Add/remove a command from `autostart` ("Open at Login").
    ToggleLogin(String),
    EmptyTrash,
    EmptyTrashConfirmed,
    ToggleWidgets,
    NewFolder(String),
    /// A row of an application's tray menu was chosen: (tray key, dbusmenu id).
    TrayMenu(String, i32),
    /// A row of the focused app's global menu (dbusmenu id).
    AppMenu(i32),
    /// Turn Stage Manager on/off.
    SetStageManager(bool),
    /// Tiling menu of the green button: (window id, `tile-left` / `arrange-quarters` /
    /// `tile-restore` / `fullscreen`).
    TileWindow(u64, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Tab,
    /// ⌘ + a key (Latin letter / digit of the physical key) inside a modal panel,
    /// e.g. ⌘1…⌘4 for the Spotlight filters.
    Cmd(char),
}
