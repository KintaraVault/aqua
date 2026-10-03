//! Pointer and keyboard routing into bar, dock and panels.
use super::*;

impl Shell {
    /// Is the logical point over interactive shell UI (so the compositor should not
    /// forward the event to clients)?
    pub fn wants_pointer(&self, x: f32, y: f32) -> bool {
        self.locked
            || self.dock.press.is_some()
            || alert::active(self)
            || self.clipboard.open
            || charviewer::over(self, x, y)
            || screenshot::wants_pointer(self, x, y)
            || self.menu.open.is_some()
            || self.launchpad.visible()
            || self.control.visible()
            || self.spotlight.open
            || notifications::wants_pointer(self, x, y)
            || (self.bar_shown() && y < self.cfg.menubar_height)
            || (self.dock_shown() && dock::hit(self, x, y))
    }

    /// Edge reveal of the auto-hidden menu bar / Dock. A full-screen window owns the
    /// whole screen: there the bar and the Dock stay hidden even at the screen edges.
    pub(super) fn fullscreen_reveal(&mut self, x: f32, y: f32) {
        if self.fullscreen {
            self.fs_reveal_bar = false;
            self.fs_reveal_dock = false;
            return;
        }
        if !self.bar_hides() {
            self.fs_reveal_bar = false;
        } else if y <= 1.0 {
            self.fs_reveal_bar = true;
        } else if y > self.cfg.menubar_height + 24.0 && self.menu.open.is_none() {
            self.fs_reveal_bar = false;
        }
        if !self.dock_hides() {
            self.fs_reveal_dock = false;
            return;
        }
        if y >= self.h - 2.0 {
            self.fs_reveal_dock = true;
        } else if self.fs_reveal_dock && !dock::hit(self, x, y) && y < self.h - self.cfg.dock_icon_size * 2.2 {
            self.fs_reveal_dock = false;
        }
    }

    pub fn pointer_motion(&mut self, x: f32, y: f32) {
        self.pointer = (x, y);
        self.fullscreen_reveal(x, y);
        if self.locked {
            lockscreen::hover(self, x, y);
            return;
        }
        screenshot::motion(self, x, y);
        if self.shot.active() {
            return;
        }
        alert::hover(self, x, y);
        clipboard::hover(self, x, y);
        charviewer::hover(self, x, y);
        dock::hover(self, x, y);
        menu::hover(self, x, y);
        launchpad::hover(self, x, y);
        control::hover(self, x, y);
        spotlight::hover(self, x, y);
    }

    pub fn pointer_button(&mut self, x: f32, y: f32, pressed: bool) -> Vec<Action> {
        let a = self.pointer_button_raw(x, y, pressed);
        self.intercept(a)
    }

    pub(super) fn pointer_button_raw(&mut self, x: f32, y: f32, pressed: bool) -> Vec<Action> {
        self.pointer = (x, y);
        if !self.locked {
            if let Some(a) = screenshot::button(self, x, y, pressed) {
                return a;
            }
        }
        if !pressed {
            if self.dock.press.is_some() {
                return dock::release(self, x, y).unwrap_or_default();
            }
            return vec![];
        }
        self.dock.press = None;
        self.launch_origin = None;
        if self.locked {
            return lockscreen::click(self, x, y);
        }
        if let Some(a) = alert::click(self, x, y) {
            return a;
        }
        if let Some(a) = clipboard::click(self, x, y) {
            return a;
        }
        if let Some(a) = charviewer::click(self, x, y) {
            return a;
        }
        if let Some(a) = spotlight::click(self, x, y) {
            return a;
        }
        if let Some(acts) = menu::click(self, x, y) {
            return acts;
        }
        if y < self.cfg.menubar_height {
            return menubar::click(self, x, y);
        }
        if let Some(a) = notifications::click(self, x, y) {
            return a;
        }
        if let Some(a) = control::click(self, x, y) {
            return a;
        }
        if let Some(a) = launchpad::click(self, x, y) {
            return a;
        }
        if let Some(a) = dock::press(self, x, y) {
            return a;
        }
        vec![]
    }

    /// Right click: Dock item menus, or closes an open menu. Returns true if consumed.
    pub fn pointer_secondary(&mut self, x: f32, y: f32) -> (bool, Vec<Action>) {
        self.pointer = (x, y);
        if self.locked || alert::active(self) {
            return (true, vec![]);
        }
        if let Some((item, r)) = menubar::tray_at(self, x, y) {
            return (true, tray::secondary_click(self, &item, r));
        }
        if let Some((i, _it, slot)) = dock::item_at(self, x, y) {
            self.close_transients();
            self.menu.open = Some(menu::MenuKind::Dock(i));
            self.menu.pos = (slot.cx(), slot.y);
            self.menu.hover = None;
            return (true, vec![Action::Redraw]);
        }
        if self.menu.open.is_some() {
            self.menu.open = None;
            return (true, vec![Action::Redraw]);
        }
        (self.wants_pointer(x, y), vec![])
    }

    /// Right click on the bare desktop.
    pub fn open_desktop_menu(&mut self, x: f32, y: f32) {
        self.close_transients();
        self.menu.open = Some(menu::MenuKind::Desktop);
        self.menu.pos = (x, y);
        self.menu.hover = None;
    }

    /// Scroll over shell UI.
    pub fn scroll(&mut self, x: f32, y: f32, dy: f32, wheel: bool) -> bool {
        if let Some((item, _)) = menubar::tray_at(self, x, y) {
            aqua_tray::scroll(&item.key, dy.round() as i32, true);
            return true;
        }
        charviewer::scroll(self, x, y, dy)
            || spotlight::scroll(self, x, y, dy, wheel)
            || launchpad::scroll(self, x, y, dy, wheel)
    }

    /// Keyboard input while the shell has a modal surface open. Returns true if consumed.
    pub fn key(&mut self, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
        let (c, a) = self.key_raw(key, text);
        (c, self.intercept(a))
    }

    pub(super) fn key_raw(&mut self, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
        if self.locked {
            return (true, lockscreen::key(self, key, text));
        }
        if self.shot.active() {
            return screenshot::key(self, key, text);
        }
        if alert::active(self) {
            return alert::key(self, key);
        }
        if self.clipboard.open {
            return clipboard::key(self, key, text);
        }
        if self.chars.open && key == Some(Key::Escape) {
            self.chars.close();
            return (true, vec![Action::Redraw]);
        }
        if self.menu.open.is_some() {
            return menu::key(self, key);
        }
        if self.notes.replying.is_some() {
            return notifications::key(self, key, text);
        }
        if self.notes.center_open && key == Some(Key::Escape) {
            self.notes.toggle_center();
            return (true, vec![]);
        }
        if self.control.visible() && key == Some(Key::Escape) {
            self.control.toggle();
            return (true, vec![]);
        }
        if self.spotlight.open {
            return spotlight::key(self, key, text);
        }
        if self.launchpad.visible() {
            return launchpad::key(self, key, text);
        }
        (false, vec![])
    }
}
