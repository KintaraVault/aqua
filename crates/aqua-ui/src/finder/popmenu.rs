//! Finder menus as their own windows (`FinderMenuWindow`), so they can extend past the
//! Finder window. Used under Aqua (`crate::native_menus`); otherwise the in-window
//! `MenuOverlay` shows the same `F.menu` / `F.sub-menu` models.
use super::App;
use crate::{FMenuItem, FinderMenuWindow, F};
use slint::{ComponentHandle, Model, ModelRc};

#[derive(Default)]
pub(super) struct NativeMenus {
    main: Option<FinderMenuWindow>,
    sub: Option<FinderMenuWindow>,
}

impl NativeMenus {
    pub(super) fn is_open(&self) -> bool {
        self.main.is_some()
    }
}

/// Copy the appearance (dark mode, accent …) of the Finder window to a menu window.
fn copy_theme(from: &crate::Theme, to: &crate::Theme) {
    to.set_dark(from.get_dark());
    to.set_accent(from.get_accent());
    to.set_glass_controls(from.get_glass_controls());
    to.set_glass_lights(from.get_glass_lights());
    to.set_motion(from.get_motion());
}

impl App {
    fn new_menu_window(&self, items: ModelRc<FMenuItem>, nested: bool, title: String) -> Option<FinderMenuWindow> {
        let ui_ = self.ui();
        let win = match FinderMenuWindow::new() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("cannot create a menu window: {e}");
                return None;
            }
        };
        copy_theme(&ui_.global::<crate::Theme>(), &win.global::<crate::Theme>());
        let (mf, pf) = (ui_.global::<F>(), win.global::<F>());
        pf.set_tag_defs(mf.get_tag_defs());
        pf.set_menu_tags(mf.get_menu_tags());
        pf.set_menu_open(true);
        win.set_items(items);
        win.set_nested(nested);
        win.set_popup_title(title.into());
        let glass = crate::glass_supported();
        win.set_glass(glass);
        let me = self.me.clone();
        pf.on_menu_action(move |id| {
            let Some(app) = me.upgrade() else { return };
            let ui_ = app.borrow().ui();
            // Closing the Finder-side menu state closes the menu windows (F.menu-closed).
            ui_.global::<F>().set_sub_open(false);
            ui_.global::<F>().set_menu_open(false);
            if let Ok(mut a) = app.try_borrow_mut() {
                a.action(&id);
            };
        });
        let me = self.me.clone();
        pf.on_menu_sub(move |id, y| {
            let Some(app) = me.upgrade() else { return };
            if let Ok(mut a) = app.try_borrow_mut() {
                a.menu_sub(&id, y);
            };
        });
        let me = self.me.clone();
        let weak = win.as_weak();
        pf.on_tag_toggle(move |t| {
            let Some(app) = me.upgrade() else { return };
            if let Ok(mut a) = app.try_borrow_mut() {
                a.tag_toggle(&t);
                if let Some(w) = weak.upgrade() {
                    w.global::<F>().set_tag_defs(a.ui().global::<F>().get_tag_defs());
                }
            };
        });
        if !nested {
            let me = self.me.clone();
            win.on_sub_closed(move || {
                let Some(app) = me.upgrade() else { return };
                if let Ok(mut a) = app.try_borrow_mut() {
                    a.close_native_sub();
                };
            });
        }
        let me = self.me.clone();
        win.window().on_close_requested(move || {
            // Aqua asks menus to close when the user clicks elsewhere.
            if let Some(app) = me.upgrade() {
                if let Ok(a) = app.try_borrow() {
                    a.ui().global::<F>().set_menu_open(false);
                }
            }
            slint::CloseRequestResponse::HideWindow
        });
        let w = win.get_want_w();
        let h = win.get_want_h();
        win.window().set_size(slint::LogicalSize::new(w, h));
        if glass {
            crate::enable_glass(&win.as_weak());
        }
        if let Err(e) = win.show() {
            tracing::warn!("cannot show a menu window: {e}");
            return None;
        }
        Some(win)
    }

    /// `show_menu` under Aqua: the menu in its own window at window position (x, y).
    pub(super) fn show_native_menu(&mut self, x: f32, y: f32) {
        self.close_native_menus();
        let items = self.ui().global::<F>().get_menu();
        if items.row_count() == 0 {
            return;
        }
        let title = crate::popup_title(x, y, None, false);
        self.pop.main = self.new_menu_window(items, false, title);
        self.ui().global::<F>().set_menu_native(self.pop.main.is_some());
    }

    /// Submenu `id` (items already in `F.sub-menu`) next to the row at y (menu window coords).
    pub(super) fn show_native_sub(&mut self, id: &str, y: f32) {
        let Some(main) = &self.pop.main else { return };
        let mf = main.global::<F>();
        if mf.get_sub_open() && mf.get_sub_id() == id && self.pop.sub.is_some() {
            return;
        }
        if let Some(s) = self.pop.sub.take() {
            let _ = s.hide();
        }
        let items = self.ui().global::<F>().get_sub_menu();
        let mw = main.get_want_w();
        let title = crate::popup_title(mw - 4.0, y - 5.0, Some(-240.0 + 4.0), true);
        mf.set_sub_id(id.into());
        mf.set_sub_open(true);
        self.pop.sub = self.new_menu_window(items, true, title);
    }

    pub(super) fn close_native_sub(&mut self) {
        if let Some(s) = self.pop.sub.take() {
            let _ = s.hide();
        }
    }

    /// Hide every menu window (the menu closed: item chosen, Escape, click elsewhere).
    pub(super) fn close_native_menus(&mut self) {
        self.close_native_sub();
        if let Some(m) = self.pop.main.take() {
            let _ = m.hide();
        }
    }
}
