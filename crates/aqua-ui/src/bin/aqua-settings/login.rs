//! Login Items pane.
use super::*;

/// Wire the login pane callbacks.
pub fn wire_login(ui: &SettingsWindow, cfg: &Rc<RefCell<Config>>) -> Rc<dyn Fn()> {
    let s = ui.global::<S>();
    let login_entries: Rc<RefCell<Vec<LoginSrc>>> = Rc::new(RefCell::new(vec![]));
    let app_list: Rc<RefCell<Vec<(String, std::path::PathBuf)>>> = Rc::new(RefCell::new(vec![]));
    let refresh_login = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        let entries = login_entries.clone();
        let apps = app_list.clone();
        move || {
            let Some(u) = ui.upgrade() else { return };
            let s = u.global::<S>();
            let mut rows = vec![];
            let mut src = vec![];
            for c in &cfg.borrow().autostart {
                let bin = c.split_whitespace().next().unwrap_or(c).rsplit('/').next().unwrap_or(c).to_string();
                rows.push(LoginItem {
                    name: bin.into(),
                    detail: format!("Aqua · {c}").into(),
                    enabled: true,
                    removable: true,
                });
                src.push(LoginSrc::Cfg(c.clone()));
            }
            for e in aqua_config::autostart::entries() {
                rows.push(LoginItem {
                    name: e.name.clone().into(),
                    detail: e.exec.clone().into(),
                    enabled: e.enabled,
                    removable: e.user_only,
                });
                src.push(LoginSrc::Xdg(e.id.clone()));
            }
            *entries.borrow_mut() = src;
            s.set_login_items(ModelRc::new(VecModel::from(rows)));
            if apps.borrow().is_empty() {
                *apps.borrow_mut() = aqua_config::autostart::apps();
                s.set_app_names(ModelRc::new(VecModel::from(
                    apps.borrow().iter().map(|(n, _)| SharedString::from(n.as_str())).collect::<Vec<_>>(),
                )));
            }
        }
    };
    let refresh_login = Rc::new(refresh_login);
    s.on_toggle_login({
        let cfg = cfg.clone();
        let entries = login_entries.clone();
        let refresh = refresh_login.clone();
        move |i, on| {
            let e = entries.borrow().get(i.max(0) as usize).cloned();
            match e {
                Some(LoginSrc::Xdg(id)) => {
                    let _ = aqua_config::autostart::set_enabled(&id, on);
                }
                Some(LoginSrc::Cfg(c)) if !on => {
                    cfg.borrow_mut().autostart.retain(|x| *x != c);
                    let _ = cfg.borrow().save();
                }
                _ => {}
            }
            refresh();
        }
    });
    s.on_remove_login({
        let cfg = cfg.clone();
        let entries = login_entries.clone();
        let refresh = refresh_login.clone();
        move |i| {
            let e = entries.borrow().get(i.max(0) as usize).cloned();
            match e {
                Some(LoginSrc::Xdg(id)) => {
                    let _ = aqua_config::autostart::remove(&id);
                }
                Some(LoginSrc::Cfg(c)) => {
                    cfg.borrow_mut().autostart.retain(|x| *x != c);
                    let _ = cfg.borrow().save();
                }
                None => {}
            }
            refresh();
        }
    });
    s.on_add_login({
        let apps = app_list.clone();
        let refresh = refresh_login.clone();
        move |i| {
            if let Some((_, p)) = apps.borrow().get(i.max(0) as usize) {
                let _ = aqua_config::autostart::add(p);
            }
            refresh();
        }
    });
    refresh_login
}
