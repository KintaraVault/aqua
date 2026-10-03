//! Spotlight result sources and ranking.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Row {
    Calc(String),
    App(usize),
    /// System Settings pane: (pane id, title).
    Setting(&'static str, &'static str),
    Act(usize),
    File(PathBuf, bool),
    Clip(u64, String, String),
}

/// System Settings panes Spotlight can open directly: (pane, title, keywords).
pub(super) const SETTINGS: &[(&str, &str, &str)] = &[
    ("wifi", "Wi-Fi", "wireless network internet wlan вайфай сеть"),
    ("bluetooth", "Bluetooth", "wireless devices блютуз"),
    ("network", "Network", "ethernet vpn proxy internet сеть"),
    ("battery", "Battery", "energy power low power батарея аккумулятор"),
    ("general", "General", "основные"),
    ("about", "About", "computer system version info об этом компьютере"),
    ("update", "Software Update", "upgrade updates обновление"),
    ("storage", "Storage", "disk space хранилище диск"),
    ("datetime", "Date & Time", "clock timezone дата время"),
    ("language", "Language & Region", "locale язык регион"),
    ("login", "Login Items", "startup autostart автозапуск"),
    ("appearance", "Appearance", "dark mode light accent theme оформление тема"),
    ("menubar", "Menu Bar", "status items строка меню"),
    ("dock", "Desktop & Dock", "dock magnification hide рабочий стол"),
    ("displays", "Displays", "monitor resolution scale screen дисплей монитор"),
    ("wallpaper", "Wallpaper", "background desktop picture обои"),
    ("sound", "Sound", "volume audio output input звук"),
    ("lock", "Lock Screen", "screen saver password блокировка"),
    ("users", "Users & Groups", "accounts пользователи"),
    ("keyboard", "Keyboard", "layout input sources клавиатура раскладка"),
    ("shortcuts", "Keyboard Shortcuts", "hotkeys keybindings сочетания клавиш"),
    ("mouse", "Mouse", "pointer scroll speed мышь"),
    ("trackpad", "Trackpad", "touchpad gestures тачпад"),
    ("notifications", "Notifications", "alerts banners уведомления"),
    ("focus", "Focus", "do not disturb не беспокоить"),
    ("accessibility", "Accessibility", "reduce motion zoom универсальный доступ"),
];

/// Actions (the "Actions" filter, ⌘3): (id, title, keywords).
pub(super) const ACTIONS: &[(&str, &str, &str)] = &[
    ("screenshot", "Take Screenshot", "capture screen снимок экрана скриншот"),
    ("screenshot-ui", "Screenshot and Recording Options", "capture toolbar record снимок запись"),
    ("record", "Record Screen", "video recording screencast запись экрана видео"),
    ("dark", "Toggle Dark Mode", "appearance light theme тёмная темная тема"),
    ("dnd", "Toggle Do Not Disturb", "focus notifications не беспокоить"),
    ("lock", "Lock Screen", "security блокировка заблокировать"),
    ("mission", "Mission Control", "windows overview spaces окна"),
    ("apps", "Show Applications", "launchpad apps программы приложения"),
    ("terminal", "New Terminal Window", "shell console терминал"),
    ("clipboard", "Show Clipboard History", "paste copy буфер обмена"),
    ("chars", "Emoji & Symbols", "characters emoji символы эмодзи"),
    ("notifications", "Show Notification Center", "widgets уведомления"),
    ("control", "Show Control Center", "wifi volume brightness пункт управления"),
    ("widgets", "Show or Hide Desktop Widgets", "виджеты"),
    ("trash", "Empty Trash", "delete корзина очистить"),
    ("about", "About This Computer", "system info об этом компьютере"),
    ("sleep", "Sleep", "suspend сон"),
    ("restart", "Restart…", "reboot перезагрузка"),
    ("shutdown", "Shut Down…", "power off poweroff выключить"),
    ("logout", "Log Out…", "sign out выйти"),
];

/// Match `q` (lower-case) against a name and extra keywords; lower = better.
pub(super) fn score(q: &str, name: &str, keywords: &str) -> Option<i32> {
    let n = name.to_lowercase();
    if n.starts_with(q) {
        return Some(0);
    }
    if n.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        return Some(1);
    }
    if n.contains(q) {
        return Some(2);
    }
    let hay = format!("{n} {}", keywords.to_lowercase());
    let words: Vec<&str> = hay.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if q.split_whitespace().all(|qw| words.iter().any(|w| w.starts_with(qw))) {
        return Some(3);
    }
    if hay.contains(q) {
        return Some(4);
    }
    None
}

pub(super) fn app_rows(sh: &Shell, q: &str, limit: usize) -> Vec<Row> {
    let mut scored: Vec<(i32, usize)> = sh
        .apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            if q.is_empty() {
                return Some((0, i));
            }
            let kw = format!("{} {} {}", a.keywords.join(" "), a.categories.join(" "), a.id);
            score(q, &a.name, &kw).map(|s| (s, i))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| sh.apps[a.1].name.to_lowercase().cmp(&sh.apps[b.1].name.to_lowercase()))
    });
    scored.into_iter().take(limit).map(|(_, i)| Row::App(i)).collect()
}

pub(super) fn setting_rows(q: &str, limit: usize) -> Vec<Row> {
    if q.is_empty() {
        return vec![];
    }
    let mut v: Vec<(i32, Row)> =
        SETTINGS.iter().filter_map(|(p, t, k)| score(q, t, k).map(|s| (s, Row::Setting(p, t)))).collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().take(limit).map(|x| x.1).collect()
}

pub(super) fn action_rows(q: &str, limit: usize) -> Vec<Row> {
    let mut v: Vec<(i32, Row)> = ACTIONS
        .iter()
        .enumerate()
        .filter_map(
            |(i, (_, t, k))| {
                if q.is_empty() {
                    Some((0, Row::Act(i)))
                } else {
                    score(q, t, k).map(|s| (s, Row::Act(i)))
                }
            },
        )
        .collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().take(limit).map(|x| x.1).collect()
}

pub(super) fn clip_rows(sh: &Shell, q: &str, limit: usize) -> Vec<Row> {
    sh.clipboard
        .items
        .iter()
        .filter(|it| q.is_empty() || it.title.to_lowercase().contains(q) || it.detail.to_lowercase().contains(q))
        .take(limit)
        .map(|it| Row::Clip(it.id, it.title.clone(), it.detail.clone()))
        .collect()
}

pub(super) fn rows(sh: &Shell) -> Vec<Row> {
    let q = sh.spotlight.query.trim().to_lowercase();
    match sh.spotlight.filter {
        Some(Filter::Apps) => return app_rows(sh, &q, 400),
        Some(Filter::Actions) => {
            let mut v = action_rows(&q, 60);
            v.extend(setting_rows(&q, 20));
            return v;
        }
        Some(Filter::Files) => return files::search(&q, 60).into_iter().map(|(p, d)| Row::File(p, d)).collect(),
        Some(Filter::Clipboard) => return clip_rows(sh, &q, 60),
        None => {}
    }
    if q.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    if let Some(v) = calc(&q) {
        out.push(Row::Calc(fmt_num(v)));
    }
    out.extend(app_rows(sh, &q, 8));
    out.extend(setting_rows(&q, 3));
    out.extend(action_rows(&q, 3));
    if q.chars().count() >= 2 {
        out.extend(files::search(&q, 5).into_iter().map(|(p, d)| Row::File(p, d)));
    }
    out
}

pub(super) fn row_kind(row: &Row) -> &'static str {
    match row {
        Row::Calc(_) => "Calculator",
        Row::App(_) => "Application",
        Row::Setting(..) => "System Settings",
        Row::Act(_) => "Action",
        Row::File(_, true) => "Folder",
        Row::File(..) => "Document",
        Row::Clip(..) => "Clipboard",
    }
}

pub(super) fn tilde(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().into_owned();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && p.starts_with(&h) => format!("~{}", &s[h.trim_end_matches('/').len()..]),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring() {
        assert_eq!(score("sys", "System Settings", ""), Some(0));
        assert_eq!(score("set", "System Settings", ""), Some(1));
        assert_eq!(score("tem", "System Settings", ""), Some(2));
        assert_eq!(score("системные пар", "System Settings", "системные параметры"), Some(3));
        assert_eq!(score("prefs", "System Settings", "prefs"), Some(3));
        assert_eq!(score("ystem set", "System Settings", ""), Some(2));
        assert_eq!(score("zzz", "System Settings", "prefs"), None);
    }

    #[test]
    fn static_tables_are_consistent() {
        for (id, title, _) in SETTINGS.iter().chain(ACTIONS) {
            assert!(!id.is_empty() && !title.is_empty());
        }
        let mut ids: Vec<&str> = ACTIONS.iter().map(|a| a.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ACTIONS.len(), "action ids are unique");
    }

    #[test]
    fn home_is_abbreviated() {
        if let Ok(h) = std::env::var("HOME") {
            let h = std::path::PathBuf::from(h);
            assert_eq!(tilde(&h.join("Documents/a.txt")), "~/Documents/a.txt");
            assert_eq!(tilde(&h), "~");
            let sibling = format!("{}x/file", h.display());
            assert_eq!(tilde(std::path::Path::new(&sibling)), sibling, "only whole path components match");
        }
        assert_eq!(tilde(std::path::Path::new("/etc/hosts")), "/etc/hosts");
    }
}
