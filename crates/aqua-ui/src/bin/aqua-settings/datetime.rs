//! Date & Time and Language & Region panes.
use super::*;

/// Wire the datetime pane callbacks.
pub fn wire_datetime(ui: &SettingsWindow) -> Rc<dyn Fn()> {
    let s = ui.global::<S>();
    let tz_list: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
    let refresh_time = {
        let ui = ui.as_weak();
        let tz_list = tz_list.clone();
        move || {
            let Some(u) = ui.upgrade() else { return };
            let s = u.global::<S>();
            let t = aqua_ui::sysdata::time_info();
            if tz_list.borrow().is_empty() {
                let mut v = aqua_ui::sysdata::timezones();
                if !t.timezone.is_empty() && !v.contains(&t.timezone) {
                    v.insert(0, t.timezone.clone());
                }
                *tz_list.borrow_mut() = v;
                s.set_timezones(ModelRc::new(VecModel::from(
                    tz_list.borrow().iter().map(|z| SharedString::from(z.replace('_', " "))).collect::<Vec<_>>(),
                )));
            }
            s.set_tz_idx(tz_list.borrow().iter().position(|z| *z == t.timezone).map(|i| i as i32).unwrap_or(-1));
            s.set_ntp(t.ntp);
            s.set_can_ntp(t.can_ntp || aqua_sys::have("timedatectl"));
            s.set_now_text(aqua_ui::sysdata::now_text(s.get_clock_24h()).into());
        }
    };
    let refresh_time = Rc::new(refresh_time);
    let timedatectl = {
        let ui = ui.as_weak();
        let refresh = refresh_time.clone();
        move |args: Vec<String>, ok: &'static str| {
            if let Some(u) = ui.upgrade() {
                u.global::<S>().set_dt_status(tr("Applying… (administrator password may be required)").into());
            }
            let weak = ui.clone();
            let refresh = refresh.clone();
            run_bg(
                move || match std::process::Command::new("timedatectl").args(&args).output() {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                    Err(e) => Err(e.to_string()),
                },
                move |r: Result<(), String>| {
                    if let Some(u) = weak.upgrade() {
                        u.global::<S>().set_dt_status(match r {
                            Ok(()) => ok.into(),
                            Err(e) => trf("Could not change the setting: {e}", &[("e", &e)]).into(),
                        });
                    }
                    refresh();
                },
            );
        }
    };
    let timedatectl = Rc::new(timedatectl);
    s.on_set_ntp({
        let t = timedatectl.clone();
        move |on| {
            t(
                vec!["set-ntp".into(), on.to_string()],
                tr(if on { "Time is set automatically." } else { "Automatic time turned off." }),
            )
        }
    });
    s.on_set_timezone({
        let t = timedatectl.clone();
        let tz_list = tz_list.clone();
        move |i| {
            if let Some(z) = tz_list.borrow().get(i.max(0) as usize).cloned() {
                t(vec!["set-timezone".into(), z], tr("Time zone changed."));
            }
        }
    });
    refresh_time
}

/// Wire the language pane callbacks.
pub fn wire_language(ui: &SettingsWindow) -> Rc<dyn Fn()> {
    let s = ui.global::<S>();
    let locale_list: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
    let refresh_lang = {
        let ui = ui.as_weak();
        let list = locale_list.clone();
        move || {
            let Some(u) = ui.upgrade() else { return };
            let s = u.global::<S>();
            let (l, cur) = aqua_ui::sysdata::locales();
            s.set_locales(ModelRc::new(VecModel::from(
                l.iter()
                    .map(|c| SharedString::from(format!("{} — {c}", aqua_ui::sysdata::describe_locale(c))))
                    .collect::<Vec<_>>(),
            )));
            s.set_locale_idx(l.iter().position(|c| *c == cur).map(|i| i as i32).unwrap_or(-1));
            s.set_region_text(aqua_ui::sysdata::describe_locale(&cur).into());
            s.set_locale_example(aqua_ui::sysdata::locale_example(&cur).into());
            *list.borrow_mut() = l;
        }
    };
    let refresh_lang = Rc::new(refresh_lang);
    s.on_set_locale({
        let ui = ui.as_weak();
        let list = locale_list.clone();
        let refresh = refresh_lang.clone();
        move |i| {
            let Some(code) = list.borrow().get(i.max(0) as usize).cloned() else { return };
            if let Some(u) = ui.upgrade() {
                u.global::<S>().set_lang_status(tr("Applying… (administrator password may be required)").into());
            }
            let weak = ui.clone();
            let refresh = refresh.clone();
            run_bg(
                move || match std::process::Command::new("localectl")
                    .args(["set-locale", &format!("LANG={code}")])
                    .output()
                {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                    Err(e) => Err(e.to_string()),
                },
                move |r: Result<(), String>| {
                    if let Some(u) = weak.upgrade() {
                        u.global::<S>().set_lang_status(match r {
                            Ok(()) => tr("Saved. Log out and back in to use the new language everywhere.").into(),
                            Err(e) => trf("Could not change the language: {e}", &[("e", &e)]).into(),
                        });
                    }
                    refresh();
                },
            );
        }
    });
    refresh_lang
}
