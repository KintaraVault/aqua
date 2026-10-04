//! Shell-side handling of actions before they reach the compositor.
use super::*;

impl Shell {
    /// Handle shell-internal actions (alerts, panels); returns the rest for the compositor.
    pub fn intercept(&mut self, acts: Vec<Action>) -> Vec<Action> {
        let mut out = vec![];
        for a in acts {
            match a {
                Action::Restart | Action::ShutDown | Action::LogOut if !alert::active(self) && !self.locked => {
                    let user = aqua_sys_user();
                    self.show_alert(alert::power_confirm(a, &user));
                    out.push(Action::Redraw);
                }
                Action::ShowAbout(id) => {
                    self.show_alert(self.about_alert(&id));
                    out.push(Action::Redraw);
                }
                Action::ShowChars => {
                    self.chars.toggle(charviewer::Mode::Chars);
                    out.push(Action::Redraw);
                }
                Action::ShowKeyboardViewer => {
                    self.chars.toggle(charviewer::Mode::Keyboard);
                    out.push(Action::Redraw);
                }
                Action::ShowApps => {
                    if !self.launchpad.visible() {
                        self.toggle_launchpad();
                    }
                    out.push(Action::Redraw);
                }
                Action::ShowClipboard => {
                    self.toggle_clipboard();
                    out.push(Action::Redraw);
                }
                Action::TrayMenu(key, id) => {
                    aqua_tray::menu_clicked(&key, id);
                    out.push(Action::Redraw);
                }
                Action::AppMenu(id) => {
                    aqua_tray::appmenu::clicked(id);
                    out.push(Action::Redraw);
                }
                Action::ShowRecent => {
                    self.menu.open = Some(menu::MenuKind::Recent);
                    self.menu.anchor = 44.0;
                    self.menu.hover = None;
                    out.push(Action::Redraw);
                }
                Action::ClearRecent => menu::clear_recent(),
                Action::KeepInDock(app) => {
                    self.keep_in_dock(&app);
                    out.push(Action::Redraw);
                }
                Action::RemoveFromDock(app) => {
                    self.cfg.dock.retain(|d| d.app != app);
                    self.save_cfg();
                    self.apply_config();
                    out.push(Action::Redraw);
                }
                Action::ToggleLogin(exec) => {
                    if self.cfg.autostart.iter().any(|a| a == &exec) {
                        self.cfg.autostart.retain(|a| a != &exec);
                    } else {
                        self.cfg.autostart.push(exec);
                    }
                    self.save_cfg();
                    out.push(Action::Redraw);
                }
                Action::ToggleWidgets => {
                    let show = !(self.cfg.show_widgets && !self.widgets.hidden);
                    self.widgets.hidden = false;
                    self.cfg.show_widgets = show;
                    self.save_cfg();
                    self.cache.clear();
                    out.push(Action::Redraw);
                }
                Action::NewFolder(path) => {
                    if let Err(e) = std::fs::create_dir_all(&path) {
                        let a = alert::Alert::new(
                            alert::AlertIcon::Computer,
                            "Could not create the folder",
                            &e.to_string(),
                        )
                        .button("OK", vec![], true);
                        self.show_alert(a);
                    }
                    out.push(Action::Redraw);
                }
                Action::EmptyTrash => {
                    let a = alert::Alert::new(
                        alert::AlertIcon::Computer,
                        "Are you sure you want to permanently erase the items in the Trash?",
                        "You can’t undo this action.",
                    )
                    .button("Cancel", vec![], false)
                    .button("Empty Trash", vec![Action::EmptyTrashConfirmed], true);
                    self.show_alert(a);
                    out.push(Action::Redraw);
                }
                Action::EmptyTrashConfirmed => out.push(Action::Launch(dock::TRASH_EMPTY.into())),
                Action::WifiPower(on) => aqua_sys::network::set_wifi_enabled(on),
                Action::WifiDisconnect => aqua_sys::network::disconnect(),
                Action::LowPower(on) => aqua_sys::power::set_low_power(on),
                Action::WifiConnect(ssid, needs_password) => {
                    if needs_password {
                        out.push(Action::OpenSettings(format!("wifi:{ssid}")));
                    } else {
                        aqua_sys::network::connect(ssid, None, |_| {});
                    }
                }
                Action::ForceQuit(id) => {
                    let name = self.app_display_name(&id);
                    let icon = aqua_apps::match_app_id(&self.apps, &id).map(|a| a.icon.clone()).unwrap_or_default();
                    let a = alert::Alert::new(
                        alert::AlertIcon::App { id: id.clone(), name: name.clone(), icon },
                        &crate::trf("Do you want to force “{name}” to quit?", &[("name", &name)]),
                        "You will lose any unsaved changes.",
                    )
                    .button("Cancel", vec![], false)
                    .button("Force Quit", vec![Action::ForceQuitConfirmed(id)], true);
                    self.show_alert(a);
                    out.push(Action::Redraw);
                }
                a => out.push(a),
            }
        }
        out
    }

    /// Persist `self.cfg`; the compositor's config watcher sees our own write too, which
    /// is harmless (same content).
    pub(super) fn save_cfg(&self) {
        if let Err(e) = self.cfg.save() {
            eprintln!("aqua-shell: could not save config: {e}");
        }
    }

    /// Icon style from the config and the current appearance.
    pub(super) fn update_icon_look(&mut self) {
        let dark = self.style.dark;
        let look = aqua_icons::look::Look {
            style: aqua_icons::look::Style::from_config(&self.cfg.icon_style, dark),
            dark,
            tint: self.cfg.accent_rgb(),
            glass: self.cfg.icon_glass,
        };
        self.icons.set_look(look);
    }

    /// Persist a changed `cfg.dock` and rebuild the Dock (animations carry over).
    pub(crate) fn commit_dock(&mut self) {
        self.save_cfg();
        let mut d = dock::Dock::new(&self.cfg, &self.apps);
        d.carry_over(&mut self.dock);
        self.dock = d;
        self.icons.set_policy(aqua_config::apple_icons::Policy {
            owners: dock::icon_owners(&self.dock, &self.apps),
            ..aqua_config::apple_icons::Policy::from_config(&self.cfg)
        });
        self.serial += 1;
    }

    /// Pin a running app to the Dock (stored as a `[[dock]]` entry).
    pub(super) fn keep_in_dock(&mut self, app_id: &str) {
        if self.cfg.dock.iter().any(|d| d.app == app_id) {
            return;
        }
        let item = self.dock_entry(app_id);
        self.cfg.dock.push(item);
        self.save_cfg();
        self.apply_config();
    }

    /// The `[[dock]]` entry pinning the app behind `app_id`.
    pub(crate) fn dock_entry(&self, app_id: &str) -> aqua_config::DockItem {
        let app = aqua_apps::match_app_id(&self.apps, app_id).cloned();
        aqua_config::DockItem {
            name: app.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| self.app_display_name(app_id)),
            app: app.as_ref().map(|a| a.id.clone()).unwrap_or_else(|| app_id.to_string()),
            exec: app.as_ref().map(|a| a.command()).unwrap_or_default(),
            icon: app.as_ref().map(|a| a.icon.clone()).unwrap_or_default(),
            ids: if app.as_ref().map(|a| a.id != app_id).unwrap_or(false) { vec![app_id.to_string()] } else { vec![] },
        }
    }

    pub(super) fn about_alert(&self, id: &str) -> alert::Alert {
        if id.is_empty() {
            let info = aqua_sys::session::about();
            let name = info.iter().find(|(k, _)| k == "Name").map(|x| x.1.clone()).unwrap_or_default();
            let os = info.iter().find(|(k, _)| k == "OS").map(|x| x.1.clone()).unwrap_or_default();
            let mut a = alert::Alert::new(
                alert::AlertIcon::Computer,
                if name.is_empty() { "Aqua" } else { &name },
                &format!("{os}\nAqua Desktop {}", env!("CARGO_PKG_VERSION")),
            );
            a.info = info.into_iter().filter(|(k, _)| k != "Name" && k != "OS").collect();
            return a.button("More Info…", vec![Action::OpenSettings("about".into())], false).button(
                "OK",
                vec![],
                true,
            );
        }
        let name = self.app_display_name(id);
        let app = aqua_apps::match_app_id(&self.apps, id);
        let icon = app.map(|a| a.icon.clone()).unwrap_or_default();
        let mut body = String::new();
        if let Some(a) = app {
            body = a.categories.iter().filter(|c| !c.is_empty()).take(3).cloned().collect::<Vec<_>>().join(" · ");
        }
        let mut al =
            alert::Alert::new(alert::AlertIcon::App { id: id.to_string(), name: name.clone(), icon }, &name, &body);
        al.info.push(("Identifier".into(), id.to_string()));
        if let Some(a) = app {
            al.info.push(("Executable".into(), a.exec.split_whitespace().next().unwrap_or("").to_string()));
        }
        al.button("OK", vec![], true)
    }

    /// The confirmed variant of a power action (skips the alert).
    pub fn confirmed(a: &Action) -> bool {
        matches!(a, Action::RestartNow | Action::ShutDownNow | Action::LogOutNow)
    }
}
