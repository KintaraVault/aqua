//! Keyboard shortcuts editor.
use super::*;

use aqua_config::shortcuts as scs;

pub const CUSTOM_GROUP: &str = "Custom Shortcuts";

pub fn sc_groups() -> Vec<&'static str> {
    let mut g: Vec<&'static str> = vec![];
    for s in scs::SYSTEM {
        if !g.contains(&s.group) {
            g.push(s.group);
        }
    }
    g.push(CUSTOM_GROUP);
    g
}

/// After handing the pointer to the compositor (interactive move / resize) the toolkit only gets
/// `wl_pointer.leave`, never the button release.
pub fn release_pointer_later(u: &SettingsWindow) {
    let w = u.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(u) = w.upgrade() else { return };
        use slint::platform::{PointerEventButton, WindowEvent};
        let outside = slint::LogicalPosition::new(-10000.0, -10000.0);
        u.window().dispatch_event(WindowEvent::PointerReleased { position: outside, button: PointerEventButton::Left });
        u.window().dispatch_event(WindowEvent::PointerExited);
    });
}

/// `$XDG_RUNTIME_DIR/aqua-shortcut-capture`: while it exists (and is fresh) the compositor
/// passes every chord to us instead of running system shortcuts.
pub fn capture_marker(on: bool) {
    let p = aqua_config::paths::runtime_dir().join("aqua-shortcut-capture");
    if on {
        let _ = std::fs::write(&p, std::process::id().to_string());
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

pub enum Rec {
    /// only modifiers so far
    Pending(String),
    Cancel,
    Clear,
    Chord(String),
    Invalid(&'static str),
}

pub fn mods_prefix(ctrl: bool, alt: bool, shift: bool, logo: bool) -> String {
    let mut s = String::new();
    for (on, n) in [(ctrl, "ctrl+"), (alt, "alt+"), (shift, "shift+"), (logo, "super+")] {
        if on {
            s.push_str(n);
        }
    }
    s
}

/// Russian ЙЦУКЕН → the Latin key on the same position, so ⌘Ф records as ⌘A.
pub fn cyr_to_lat(c: char) -> Option<char> {
    const CYR: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";
    const LAT: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
    CYR.chars().position(|x| x == c).and_then(|i| LAT.chars().nth(i))
}

pub fn key_name(c: char) -> Option<String> {
    let shifted = "!@#$%^&*()_+{}|:\"<>?~";
    let base = "1234567890-=[]\\;',./`";
    let c = c.to_lowercase().next().unwrap_or(c);
    let c = cyr_to_lat(c).unwrap_or(c);
    let c = shifted.chars().position(|x| x == c).and_then(|i| base.chars().nth(i)).unwrap_or(c);
    Some(match c {
        'a'..='z' | '0'..='9' => c.to_string(),
        '-' => "minus".into(),
        '=' => "equal".into(),
        '[' => "bracketleft".into(),
        ']' => "bracketright".into(),
        '\\' => "backslash".into(),
        ';' => "semicolon".into(),
        '\'' => "apostrophe".into(),
        ',' => "comma".into(),
        '.' => "period".into(),
        '/' => "slash".into(),
        '`' => "grave".into(),
        ' ' => "space".into(),
        _ => return None,
    })
}

pub fn record_key(text: &str, ctrl: bool, alt: bool, shift: bool, logo: bool) -> Rec {
    use slint::platform::Key as K;
    let Some(c) = text.chars().next() else { return Rec::Pending(mods_prefix(ctrl, alt, shift, logo)) };
    let is = |k: K| c == char::from(k);
    let mods = [K::Shift, K::ShiftR, K::Control, K::ControlR, K::Alt, K::AltGr, K::Meta, K::MetaR, K::CapsLock];
    if mods.into_iter().any(is) {
        return Rec::Pending(mods_prefix(ctrl, alt, shift, logo));
    }
    let any_mod = ctrl || alt || logo;
    if is(K::Escape) && !any_mod && !shift {
        return Rec::Cancel;
    }
    if (is(K::Backspace) || is(K::Delete)) && !any_mod && !shift {
        return Rec::Clear;
    }
    let mut fkey = false;
    let named: Option<String> = if is(K::Return) {
        Some("Return".into())
    } else if is(K::Tab) || is(K::Backtab) {
        Some("Tab".into())
    } else if is(K::Escape) {
        Some("Escape".into())
    } else if is(K::Backspace) {
        Some("BackSpace".into())
    } else if is(K::Delete) {
        Some("Delete".into())
    } else if is(K::UpArrow) {
        Some("up".into())
    } else if is(K::DownArrow) {
        Some("down".into())
    } else if is(K::LeftArrow) {
        Some("left".into())
    } else if is(K::RightArrow) {
        Some("right".into())
    } else if is(K::Home) {
        Some("Home".into())
    } else if is(K::End) {
        Some("End".into())
    } else if is(K::PageUp) {
        Some("Prior".into())
    } else if is(K::PageDown) {
        Some("Next".into())
    } else if is(K::Insert) {
        Some("Insert".into())
    } else if is(K::SysReq) {
        fkey = true;
        Some("Print".into())
    } else {
        let fks = [
            K::F1,
            K::F2,
            K::F3,
            K::F4,
            K::F5,
            K::F6,
            K::F7,
            K::F8,
            K::F9,
            K::F10,
            K::F11,
            K::F12,
            K::F13,
            K::F14,
            K::F15,
            K::F16,
            K::F17,
            K::F18,
            K::F19,
            K::F20,
            K::F21,
            K::F22,
            K::F23,
            K::F24,
        ];
        match fks.into_iter().position(is) {
            Some(i) => {
                fkey = true;
                Some(format!("F{}", i + 1))
            }
            None => key_name(c),
        }
    };
    let Some(key) = named else { return Rec::Invalid(tr("That key can't be used in a shortcut.")) };
    if !any_mod && !fkey {
        return Rec::Invalid(tr("Shortcuts need ⌘, ⌃ or ⌥ (function keys work on their own)."));
    }
    Rec::Chord(format!("{}{}", mods_prefix(ctrl, alt, shift, logo), key))
}

pub fn pretty_list(chords: &[String]) -> String {
    chords.iter().map(|c| scs::pretty(c)).collect::<Vec<_>>().join("  ")
}

pub fn custom_rows(cfg: &Config) -> Vec<CustomRow> {
    cfg.bindings
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let idx = scs::ACTIONS.iter().position(|(a, _)| *a == b.action);
            CustomRow {
                keys: if b.keys.trim().is_empty() { "".into() } else { scs::pretty(&b.keys).into() },
                action_idx: idx.unwrap_or(scs::ACTIONS.len()) as i32,
                command: if idx.is_some() { "".into() } else { b.action.clone().into() },
                conflict: if b.keys.trim().is_empty() {
                    "".into()
                } else {
                    scs::conflict(cfg, &b.keys, None, Some(i)).unwrap_or_default().into()
                },
            }
        })
        .collect()
}

pub fn system_rows(cfg: &Config, group: &str) -> Vec<ShortcutRow> {
    scs::SYSTEM
        .iter()
        .filter(|s| s.group == group)
        .map(|s| {
            let ch = scs::chords(cfg, s.id);
            let conflict = ch.iter().find_map(|c| scs::conflict(cfg, c, Some(s.id), None)).unwrap_or_default();
            ShortcutRow {
                id: s.id.into(),
                label: tr(s.label).into(),
                keys: pretty_list(&ch).into(),
                enabled: !ch.is_empty(),
                is_default: scs::is_default(cfg, s.id),
                conflict: conflict.into(),
            }
        })
        .collect()
}

pub fn refresh_shortcuts(ui: &SettingsWindow, cfg: &Config) {
    let s = ui.global::<S>();
    let groups = sc_groups();
    let gi = (s.get_sc_group().max(0) as usize).min(groups.len() - 1);
    s.set_sc_rows(ModelRc::new(VecModel::from(if groups[gi] == CUSTOM_GROUP {
        vec![]
    } else {
        system_rows(cfg, groups[gi])
    })));
    s.set_sc_custom(ModelRc::new(VecModel::from(custom_rows(cfg))));
}

pub fn shortcut_editor(ui: &SettingsWindow, cfg: Rc<RefCell<Config>>) {
    let s = ui.global::<S>();
    s.set_sc_groups(ModelRc::new(VecModel::from(
        sc_groups().into_iter().map(|g| SharedString::from(tr(g))).collect::<Vec<_>>(),
    )));
    let mut acts: Vec<SharedString> = scs::ACTIONS.iter().map(|(_, l)| SharedString::from(tr(l))).collect();
    acts.push(tr("Custom command…").into());
    s.set_sc_actions(ModelRc::new(VecModel::from(acts)));
    refresh_shortcuts(ui, &cfg.borrow());
    let parked: Rc<RefCell<std::collections::HashMap<String, String>>> = Default::default();

    let persist = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        move || {
            let Some(u) = ui.upgrade() else { return };
            let c = cfg.borrow();
            if let Err(e) = c.save() {
                u.global::<S>().set_status(trf("Could not save settings: {e}", &[("e", &e)]).into());
            }
            refresh_shortcuts(&u, &c);
        }
    };
    let persist = Rc::new(persist);

    s.on_sc_select_group({
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        move |i| {
            let u = ui.unwrap();
            let s = u.global::<S>();
            s.set_sc_group(i);
            s.set_sc_message("".into());
            refresh_shortcuts(&u, &cfg.borrow());
        }
    });
    s.on_sc_record({
        let ui = ui.as_weak();
        move |target| {
            let u = ui.unwrap();
            let s = u.global::<S>();
            s.set_sc_live("".into());
            s.set_sc_message("".into());
            s.set_sc_recording(target.clone());
            capture_marker(!target.is_empty());
        }
    });
    let keepalive = slint::Timer::default();
    {
        let ui = ui.as_weak();
        keepalive.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(20), move || {
            if let Some(u) = ui.upgrade() {
                if !u.global::<S>().get_sc_recording().is_empty() {
                    capture_marker(true);
                }
            }
        });
    }
    std::mem::forget(keepalive);
    s.on_sc_mods({
        let ui = ui.as_weak();
        move |c, a, sh, l| {
            let p = mods_prefix(c, a, sh, l);
            ui.unwrap().global::<S>().set_sc_live(if p.is_empty() { "".into() } else { scs::pretty(&p).into() });
        }
    });
    s.on_sc_key({
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        let persist = persist.clone();
        move |text, c, a, sh, l| {
            let u = ui.unwrap();
            let s = u.global::<S>();
            let target = s.get_sc_recording().to_string();
            if target.is_empty() {
                return false;
            }
            let finish = |s: &S| {
                s.set_sc_recording("".into());
                s.set_sc_live("".into());
                capture_marker(false);
            };
            match record_key(&text, c, a, sh, l) {
                Rec::Pending(p) => {
                    s.set_sc_live(if p.is_empty() { "".into() } else { scs::pretty(&p).into() });
                }
                Rec::Invalid(why) => s.set_sc_message(why.into()),
                Rec::Cancel => finish(&s),
                Rec::Clear => {
                    finish(&s);
                    {
                        let mut cf = cfg.borrow_mut();
                        if let Some(id) = target.strip_prefix("sys:") {
                            cf.shortcuts.insert(id.to_string(), String::new());
                        } else if let Some(i) = target.strip_prefix("custom:").and_then(|i| i.parse::<usize>().ok()) {
                            if let Some(b) = cf.bindings.get_mut(i) {
                                b.keys.clear();
                            }
                        }
                    }
                    persist();
                }
                Rec::Chord(ch) => {
                    finish(&s);
                    let warn = {
                        let mut cf = cfg.borrow_mut();
                        if let Some(id) = target.strip_prefix("sys:") {
                            let w = scs::conflict(&cf, &ch, Some(id), None);
                            let def = scs::find(id)
                                .map(|d| d.defaults.len() == 1 && scs::normalize(d.defaults[0]) == scs::normalize(&ch))
                                .unwrap_or(false);
                            if def {
                                cf.shortcuts.remove(id);
                            } else {
                                cf.shortcuts.insert(id.to_string(), ch.clone());
                            }
                            w
                        } else if let Some(i) = target.strip_prefix("custom:").and_then(|i| i.parse::<usize>().ok()) {
                            let w = scs::conflict(&cf, &ch, None, Some(i));
                            if let Some(b) = cf.bindings.get_mut(i) {
                                b.keys = ch.clone();
                            }
                            w
                        } else {
                            None
                        }
                    };
                    persist();
                    if let Some(w) = warn {
                        s.set_sc_message(
                            trf("{keys} is also used by “{name}”.", &[("keys", &scs::pretty(&ch)), ("name", &tr(&w))])
                                .into(),
                        );
                    }
                }
            }
            true
        }
    });
    s.on_sc_toggle({
        let cfg = cfg.clone();
        let persist = persist.clone();
        let parked = parked.clone();
        move |id, on| {
            let id = id.to_string();
            {
                let mut cf = cfg.borrow_mut();
                if on {
                    match parked.borrow_mut().remove(&id) {
                        Some(v) => {
                            cf.shortcuts.insert(id.clone(), v);
                        }
                        None => {
                            cf.shortcuts.remove(&id);
                        }
                    }
                } else {
                    if let Some(v) = cf.shortcuts.get(&id).filter(|v| !v.is_empty()) {
                        parked.borrow_mut().insert(id.clone(), v.clone());
                    }
                    cf.shortcuts.insert(id.clone(), String::new());
                }
            }
            persist();
            on && scs::chords(&cfg.borrow(), &id).is_empty()
        }
    });
    s.on_sc_reset({
        let cfg = cfg.clone();
        let persist = persist.clone();
        move |id| {
            cfg.borrow_mut().shortcuts.remove(id.as_str());
            persist();
        }
    });
    s.on_sc_reset_all({
        let cfg = cfg.clone();
        let persist = persist.clone();
        let ui = ui.as_weak();
        move || {
            cfg.borrow_mut().shortcuts.clear();
            ui.unwrap().global::<S>().set_sc_message("".into());
            persist();
        }
    });
    s.on_sc_add({
        let cfg = cfg.clone();
        let persist = persist.clone();
        let ui = ui.as_weak();
        move || {
            let n = {
                let mut cf = cfg.borrow_mut();
                cf.bindings.push(aqua_config::Binding { keys: String::new(), action: scs::ACTIONS[0].0.to_string() });
                cf.bindings.len() - 1
            };
            persist();
            let u = ui.unwrap();
            u.global::<S>().invoke_sc_record(format!("custom:{n}").into());
        }
    });
    s.on_sc_remove({
        let cfg = cfg.clone();
        let persist = persist.clone();
        let ui = ui.as_weak();
        move |i| {
            let u = ui.unwrap();
            u.global::<S>().invoke_sc_record("".into());
            let i = i as usize;
            {
                let mut cf = cfg.borrow_mut();
                if i < cf.bindings.len() {
                    cf.bindings.remove(i);
                }
            }
            persist();
        }
    });
    s.on_sc_set_action({
        let cfg = cfg.clone();
        let persist = persist.clone();
        move |i, a| {
            {
                let mut cf = cfg.borrow_mut();
                let Some(b) = cf.bindings.get_mut(i as usize) else { return };
                b.action = scs::ACTIONS.get(a as usize).map(|(id, _)| id.to_string()).unwrap_or_default();
            }
            persist();
        }
    });
    s.on_sc_set_command({
        let cfg = cfg.clone();
        move |i, t| {
            let mut cf = cfg.borrow_mut();
            let Some(b) = cf.bindings.get_mut(i as usize) else { return };
            b.action = t.trim().to_string();
            let _ = cf.save();
        }
    });
}
