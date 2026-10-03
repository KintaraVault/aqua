//! Software Update and Users & Groups panes.
use super::*;

/// Wire the system pane callbacks.
pub fn wire_system(ui: &SettingsWindow, cfg: &Rc<RefCell<Config>>) -> Rc<dyn Fn()> {
    let s = ui.global::<S>();
    let pm = aqua_config::xkb::package_manager();
    let check_updates = {
        let ui = ui.as_weak();
        move || {
            let Some(u) = ui.upgrade() else { return };
            let s = u.global::<S>();
            if s.get_update_busy() {
                return;
            }
            s.set_update_busy(true);
            s.set_update_status(tr("Checking for updates…").into());
            s.set_update_detail("".into());
            let weak = ui.clone();
            run_bg(
                move || aqua_ui::sysdata::pending_updates(pm),
                move |r| {
                    let Some(u) = weak.upgrade() else { return };
                    let s = u.global::<S>();
                    s.set_update_busy(false);
                    match r {
                        Ok(list) => {
                            s.set_update_count(list.len() as i32);
                            if list.is_empty() {
                                s.set_update_status(tr("Your system is up to date").into());
                                s.set_update_detail(aqua_ui::sysdata::os_name().into());
                            } else {
                                s.set_update_status(
                                    ntr("{n} update available", "{n} updates available", list.len() as i64).into(),
                                );
                                s.set_update_detail(
                                    trf(
                                        "Installs with “{cmd}” in a terminal (administrator password required).",
                                        &[("cmd", &aqua_ui::sysdata::update_command(pm))],
                                    )
                                    .into(),
                                );
                            }
                            let shown: Vec<SharedString> = list.into_iter().take(200).map(SharedString::from).collect();
                            s.set_update_list(ModelRc::new(VecModel::from(shown)));
                        }
                        Err(e) => {
                            s.set_update_status(tr("Could not check for updates").into());
                            s.set_update_detail(e.into());
                        }
                    }
                },
            );
        }
    };
    let check_updates = Rc::new(check_updates);
    s.on_check_updates({
        let c = check_updates.clone();
        move || c()
    });
    s.on_install_updates({
        let cfg = cfg.clone();
        let ui = ui.as_weak();
        move || {
            let cmd = aqua_ui::sysdata::update_command(pm);
            if cmd.is_empty() || !aqua_ui::sysdata::run_in_terminal(&cfg.borrow().terminal, cmd) {
                if let Some(u) = ui.upgrade() {
                    u.global::<S>().set_update_detail(tr("No terminal found to run the update.").into());
                }
            }
        }
    });
    s.on_change_password({
        let cfg = cfg.clone();
        move || {
            aqua_ui::sysdata::run_in_terminal(&cfg.borrow().terminal, "passwd");
        }
    });
    s.on_open_users_tool({
        let cfg = cfg.clone();
        move || {
            aqua_ui::sysdata::run_in_terminal(
                &cfg.borrow().terminal,
                "read -r -p 'New user name: ' u && sudo useradd -m -G \"$(getent group wheel >/dev/null && echo wheel || echo users)\" \"$u\" && sudo passwd \"$u\"",
            );
        }
    });
    check_updates
}
