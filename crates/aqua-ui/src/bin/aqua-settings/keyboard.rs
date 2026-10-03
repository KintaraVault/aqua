//! Keyboard pane: input sources and input-method setup.
use super::*;

pub fn layout_rows(cfg: &Config, cat: &aqua_config::xkb::Catalogue) -> Vec<LayoutRow> {
    cfg.keyboard
        .layouts
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let var = cfg.keyboard.variants.get(i).cloned().unwrap_or_default();
            let vars = cat.variants.get(l).cloned().unwrap_or_default();
            let mut names: Vec<SharedString> = vec![tr("Default").into()];
            names.extend(vars.iter().map(|(_, d)| SharedString::from(d.as_str())));
            let vi = if var.is_empty() {
                0
            } else {
                vars.iter().position(|(c, _)| *c == var).map(|p| p as i32 + 1).unwrap_or(0)
            };
            LayoutRow {
                code: if var.is_empty() { l.clone().into() } else { format!("{l} ({var})").into() },
                name: cat.describe(l, &var).into(),
                variants: ModelRc::new(VecModel::from(names)),
                variant_idx: vi,
            }
        })
        .collect()
}

/// Packages still missing for the configured layouts.
pub fn missing_packages(cfg: &Config) -> Vec<&'static str> {
    let pm = aqua_config::xkb::package_manager();
    if pm.is_empty() {
        return vec![];
    }
    let mut want: Vec<&'static str> = vec![];
    for l in &cfg.keyboard.layouts {
        for p in aqua_config::xkb::dependencies(l, pm) {
            if !want.contains(&p) {
                want.push(p);
            }
        }
    }
    want.into_iter().filter(|p| !aqua_config::xkb::installed(p, pm)).collect()
}

/// fcitx5 for CJK layouts: autostart it, point XWayland apps at it and give it a
/// profile with the matching input method (only when the user has none yet).
pub fn configure_ime(cfg: &mut Config) {
    let ime: Vec<&str> =
        cfg.keyboard.layouts.iter().filter(|l| aqua_config::xkb::needs_ime(l)).map(|s| s.as_str()).collect();
    if ime.is_empty() || !aqua_sys::have("fcitx5") {
        return;
    }
    let cmd = "fcitx5 -d --replace".to_string();
    if !cfg.autostart.iter().any(|a| a.starts_with("fcitx5")) {
        cfg.autostart.push(cmd.clone());
        std::thread::spawn(move || {
            let _ = std::process::Command::new("sh").arg("-c").arg(cmd).status();
        });
    }
    let home = dirs::home_dir().unwrap_or_default();
    let env = home.join(".config/aqua/env");
    let old = std::fs::read_to_string(&env).unwrap_or_default();
    if !old.contains("XMODIFIERS") {
        let _ = std::fs::create_dir_all(env.parent().unwrap());
        let _ = std::fs::write(
            &env,
            format!(
                "{old}{}export XMODIFIERS=@im=fcitx\n",
                if old.is_empty() || old.ends_with('\n') { "" } else { "\n" }
            ),
        );
    }
    let profile = home.join(".config/fcitx5/profile");
    if !profile.exists() {
        let first = cfg
            .keyboard
            .layouts
            .iter()
            .find(|l| !aqua_config::xkb::needs_ime(l))
            .cloned()
            .unwrap_or_else(|| "us".into());
        let mut items = format!("[Groups/0/Items/0]\nName=keyboard-{first}\nLayout=\n\n");
        for (i, l) in ime.iter().enumerate() {
            let im = match *l {
                "jp" => "mozc",
                "kr" => "hangul",
                _ => "pinyin",
            };
            items.push_str(&format!("[Groups/0/Items/{}]\nName={im}\nLayout=\n\n", i + 1));
        }
        let txt = format!("[Groups/0]\nName=Default\nDefault Layout={first}\nDefaultIM=keyboard-{first}\n\n{items}[GroupOrder]\n0=Default\n");
        let _ = std::fs::create_dir_all(profile.parent().unwrap());
        let _ = std::fs::write(&profile, txt);
    }
}

/// Wire the keyboard pane callbacks.
pub fn wire_keyboard(ui: &SettingsWindow, cfg: &Rc<RefCell<Config>>) {
    let s = ui.global::<S>();
    let cat = Rc::new(aqua_config::xkb::catalogue());
    s.set_all_layouts(ModelRc::new(VecModel::from(
        cat.layouts.iter().map(|(c, d)| SharedString::from(format!("{d} — {c}"))).collect::<Vec<_>>(),
    )));
    let refresh_kb = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        let cat = cat.clone();
        move || {
            let u = ui.unwrap();
            let c = cfg.borrow();
            u.global::<S>().set_layout_rows(ModelRc::new(VecModel::from(layout_rows(&c, &cat))));
            u.global::<S>().set_layouts(c.keyboard.layouts.join(", ").into());
            let miss = missing_packages(&c);
            u.global::<S>().set_kb_missing(miss.join(" ").into());
        }
    };
    if !cat.from_system {
        s.set_kb_status("xkeyboard-config is not installed: only a basic list of layouts is shown.".into());
    }
    let install_deps = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        let refresh = refresh_kb.clone();
        move || {
            let pm = aqua_config::xkb::package_manager();
            let miss = missing_packages(&cfg.borrow());
            let Some(cmd) = aqua_config::xkb::install_command(&miss, pm) else {
                if !miss.is_empty() {
                    ui.unwrap()
                        .global::<S>()
                        .set_kb_status(trf("Install manually: {pkgs}", &[("pkgs", &miss.join(" "))]).into());
                }
                return;
            };
            let u = ui.unwrap();
            u.global::<S>().set_kb_busy(true);
            u.global::<S>().set_kb_status(
                trf("Installing {pkgs}… (administrator password required)", &[("pkgs", &miss.join(" "))]).into(),
            );
            let weak = ui.clone();
            let cfg2 = cfg.clone();
            let refresh2 = refresh.clone();
            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            std::thread::spawn(move || {
                let r = std::process::Command::new(&cmd[0]).args(&cmd[1..]).output();
                let res = match r {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .last()
                        .unwrap_or("installation failed")
                        .to_string()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(res);
            });
            let poll = Rc::new(slint::Timer::default());
            let poll2 = poll.clone();
            poll.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(400), move || {
                let Ok(res) = rx.try_recv() else { return };
                poll2.stop();
                let Some(u) = weak.upgrade() else { return };
                u.global::<S>().set_kb_busy(false);
                match res {
                    Ok(()) => {
                        configure_ime(&mut cfg2.borrow_mut());
                        let _ = cfg2.borrow().save();
                        u.global::<S>().set_kb_status(tr("Installed. The new input sources are ready.").into());
                    }
                    Err(e) => u.global::<S>().set_kb_status(trf("Installation failed: {e}", &[("e", &e)]).into()),
                }
                refresh2();
            });
        }
    };
    s.on_install_kb_deps(install_deps.clone());
    s.on_add_layout({
        let cfg = cfg.clone();
        let cat = cat.clone();
        let refresh = refresh_kb.clone();
        let install = install_deps.clone();
        let ui = ui.as_weak();
        move |i| {
            let Some((code, name)) = cat.layouts.get(i.max(0) as usize).cloned() else { return };
            {
                let mut c = cfg.borrow_mut();
                if c.keyboard
                    .layouts
                    .iter()
                    .zip(c.keyboard.variants.iter().chain(std::iter::repeat(&String::new())))
                    .any(|(l, v)| *l == code && v.is_empty())
                {
                    return;
                }
                let n = c.keyboard.layouts.len();
                c.keyboard.variants.resize(n, String::new());
                c.keyboard.layouts.push(code.clone());
                c.keyboard.variants.push(String::new());
                if c.keyboard.switch.is_empty() {
                    c.keyboard.switch = "ctrl+space".into();
                }
                configure_ime(&mut c);
                let _ = c.save();
            }
            ui.unwrap().global::<S>().set_kb_status(trf("Added {name}.", &[("name", &name)]).into());
            refresh();
            if !missing_packages(&cfg.borrow()).is_empty() {
                install();
            }
        }
    });
    s.on_remove_layout({
        let cfg = cfg.clone();
        let refresh = refresh_kb.clone();
        move |i| {
            {
                let mut c = cfg.borrow_mut();
                let i = i.max(0) as usize;
                if c.keyboard.layouts.len() <= 1 || i >= c.keyboard.layouts.len() {
                    return;
                }
                let n = c.keyboard.layouts.len();
                c.keyboard.variants.resize(n, String::new());
                c.keyboard.layouts.remove(i);
                c.keyboard.variants.remove(i);
                while c.keyboard.variants.last().map(|v| v.is_empty()).unwrap_or(false) {
                    c.keyboard.variants.pop();
                }
                let _ = c.save();
            }
            refresh();
        }
    });
    s.on_move_layout_up({
        let cfg = cfg.clone();
        let refresh = refresh_kb.clone();
        move |i| {
            {
                let mut c = cfg.borrow_mut();
                let i = i.max(0) as usize;
                if i == 0 || i >= c.keyboard.layouts.len() {
                    return;
                }
                let n = c.keyboard.layouts.len();
                c.keyboard.variants.resize(n, String::new());
                c.keyboard.layouts.swap(i, i - 1);
                c.keyboard.variants.swap(i, i - 1);
                while c.keyboard.variants.last().map(|v| v.is_empty()).unwrap_or(false) {
                    c.keyboard.variants.pop();
                }
                let _ = c.save();
            }
            refresh();
        }
    });
    s.on_set_variant({
        let cfg = cfg.clone();
        let cat = cat.clone();
        let refresh = refresh_kb.clone();
        move |i, v| {
            {
                let mut c = cfg.borrow_mut();
                let i = i.max(0) as usize;
                let Some(l) = c.keyboard.layouts.get(i).cloned() else { return };
                let var = if v <= 0 {
                    String::new()
                } else {
                    cat.variants.get(&l).and_then(|vs| vs.get(v as usize - 1)).map(|x| x.0.clone()).unwrap_or_default()
                };
                let n = c.keyboard.layouts.len();
                c.keyboard.variants.resize(n, String::new());
                c.keyboard.variants[i] = var;
                while c.keyboard.variants.last().map(|v| v.is_empty()).unwrap_or(false) {
                    c.keyboard.variants.pop();
                }
                let _ = c.save();
            }
            refresh();
        }
    });
    s.on_apply_system_keymap({
        let cfg = cfg.clone();
        let ui = ui.as_weak();
        move || {
            let c = cfg.borrow();
            let args = vec![
                "set-x11-keymap".to_string(),
                c.keyboard.layouts.join(","),
                c.keyboard.model.clone(),
                c.keyboard.variants.join(","),
                c.keyboard.options.clone(),
            ];
            ui.unwrap().global::<S>().set_kb_status(tr("Applying to the login screen and console…").into());
            let weak = ui.clone();
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                let msg = match std::process::Command::new("localectl").args(&args).output() {
                    Ok(o) if o.status.success() => {
                        tr("Login screen and console use these input sources now.").to_string()
                    }
                    Ok(o) => trf("localectl failed: {e}", &[("e", &String::from_utf8_lossy(&o.stderr).trim())]),
                    Err(e) => trf("localectl not available: {e}", &[("e", &e)]),
                };
                let _ = tx.send(msg);
            });
            let t = Rc::new(slint::Timer::default());
            let t2 = t.clone();
            t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(300), move || {
                if let Ok(m) = rx.try_recv() {
                    t2.stop();
                    if let Some(u) = weak.upgrade() {
                        u.global::<S>().set_kb_status(m.into());
                    }
                }
            });
        }
    });
    refresh_kb();
}
