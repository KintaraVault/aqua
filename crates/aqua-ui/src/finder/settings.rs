//! Persistent Finder preferences.
use super::fs;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub view: i32,
    pub preview: bool,
    pub sort: (i32, bool),
    pub hidden: bool,
    pub size: (f32, f32),
    pub chooser_dir: Option<PathBuf>,
    pub group: i32,
    pub icon: f32,
    pub gap: f32,
    pub icon_text: f32,
    pub item_info: bool,
    pub icon_preview: bool,
    pub list_cols: Vec<i32>,
    pub col_w: HashMap<i32, f32>,
    pub list_text: f32,
    pub rel_dates: bool,
    pub calc_sizes: bool,
    pub col_icons: bool,
    pub col_preview: bool,
    pub path_bar: bool,
    pub status_bar: bool,
    pub tab_bar: bool,
    pub sidebar: bool,
    /// 0: floating glass island, 1: solid full-height sidebar.
    pub side_w: f32,
    pub collapsed: Vec<String>,
    pub side_hidden: Vec<String>,
    pub tags_hidden: Vec<String>,
    pub new_window: i32,
    pub open_tabs: bool,
    pub show_ext: bool,
    pub warn_ext: bool,
    pub warn_trash: bool,
    pub trash_30: bool,
    pub folders_first: bool,
    pub scope: i32,
    pub last_scope: i32,
    pub open_with: HashMap<String, String>,
    pub side_smart: Vec<String>,
    pub toolbar: Vec<String>,
    pub servers: Vec<String>,
    pub recent_servers: Vec<String>,
}

pub const TOOLBAR_ITEMS: [&str; 13] = [
    "back",
    "view",
    "group",
    "action",
    "share",
    "tags",
    "newfolder",
    "delete",
    "info",
    "ql",
    "connect",
    "eject",
    "search",
];
pub const TOOLBAR_DEFAULT: [&str; 7] = ["back", "view", "group", "action", "share", "tags", "search"];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            view: 0,
            preview: false,
            sort: (0, false),
            hidden: false,
            size: (1000.0, 600.0),
            chooser_dir: None,
            group: 0,
            icon: 64.0,
            gap: 50.0,
            icon_text: 12.0,
            item_info: false,
            icon_preview: true,
            list_cols: vec![2, 3, 1],
            col_w: HashMap::new(),
            list_text: 13.0,
            rel_dates: true,
            calc_sizes: false,
            col_icons: true,
            col_preview: true,
            path_bar: false,
            status_bar: false,
            tab_bar: false,
            sidebar: true,
            side_w: 157.0,
            collapsed: vec![],
            side_hidden: vec![],
            tags_hidden: vec![],
            new_window: 1,
            open_tabs: true,
            show_ext: true,
            warn_ext: true,
            warn_trash: true,
            trash_30: false,
            folders_first: false,
            scope: 0,
            last_scope: 0,
            open_with: HashMap::new(),
            side_smart: vec![],
            toolbar: TOOLBAR_DEFAULT.iter().map(|s| s.to_string()).collect(),
            servers: vec![],
            recent_servers: vec![],
        }
    }
}

pub fn settings_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| fs::home().join(".config")).join("aqua/finder.json")
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

impl Settings {
    pub fn from_value(v: &Value) -> Self {
        let d = Settings::default();
        let g = |k: &str| v.get(k);
        let i = |k: &str, def: i32| g(k).and_then(|x| x.as_i64()).map(|x| x as i32).unwrap_or(def);
        let f = |k: &str, def: f32| g(k).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(def);
        let b = |k: &str, def: bool| g(k).and_then(|x| x.as_bool()).unwrap_or(def);
        Settings {
            view: i("view", d.view).clamp(0, 3),
            preview: b("preview", d.preview),
            sort: (i("sort", 0).clamp(0, 5), b("desc", false)),
            hidden: b("hidden", false),
            size: (f("w", d.size.0), f("h", d.size.1)),
            chooser_dir: g("chooser_dir").and_then(|x| x.as_str()).map(PathBuf::from),
            group: i("group", 0).clamp(0, 7),
            icon: f("icon", d.icon).clamp(16.0, 512.0),
            gap: f("gap", d.gap).clamp(0.0, 100.0),
            icon_text: f("icon_text", d.icon_text).clamp(10.0, 16.0),
            item_info: b("item_info", d.item_info),
            icon_preview: b("icon_preview", d.icon_preview),
            list_cols: g("list_cols")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter().filter_map(|k| k.as_i64()).map(|k| k as i32).filter(|k| (1..=6).contains(k)).collect()
                })
                .unwrap_or(d.list_cols),
            col_w: g("col_w")
                .and_then(|x| x.as_object())
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.parse::<i32>().ok()?, v.as_f64()? as f32))).collect())
                .unwrap_or_default(),
            list_text: f("list_text", d.list_text).clamp(10.0, 16.0),
            rel_dates: b("rel_dates", d.rel_dates),
            calc_sizes: b("calc_sizes", d.calc_sizes),
            col_icons: b("col_icons", d.col_icons),
            col_preview: b("col_preview", d.col_preview),
            path_bar: b("path_bar", d.path_bar),
            status_bar: b("status_bar", d.status_bar),
            tab_bar: b("tab_bar", d.tab_bar),
            sidebar: b("sidebar", d.sidebar),
            side_w: f("side_w", d.side_w).clamp(140.0, 360.0),
            collapsed: strings(g("collapsed")).unwrap_or_default(),
            side_hidden: strings(g("side_hidden")).unwrap_or_default(),
            tags_hidden: strings(g("tags_hidden")).unwrap_or_default(),
            new_window: i("new_window", d.new_window).clamp(0, 4),
            open_tabs: b("open_tabs", d.open_tabs),
            show_ext: b("show_ext", d.show_ext),
            warn_ext: b("warn_ext", d.warn_ext),
            warn_trash: b("warn_trash", d.warn_trash),
            trash_30: b("trash_30", d.trash_30),
            folders_first: b("folders_first", d.folders_first),
            scope: i("scope", d.scope).clamp(0, 2),
            last_scope: i("last_scope", 0).clamp(0, 1),
            open_with: g("open_with")
                .and_then(|x| x.as_object())
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default(),
            side_smart: strings(g("side_smart")).unwrap_or_default(),
            toolbar: strings(g("toolbar"))
                .map(|v| v.into_iter().filter(|t| TOOLBAR_ITEMS.contains(&t.as_str())).collect())
                .unwrap_or(d.toolbar),
            servers: strings(g("servers")).unwrap_or_default(),
            recent_servers: strings(g("recent_servers")).unwrap_or_default(),
        }
    }

    pub fn to_value(&self) -> Value {
        let col_w: serde_json::Map<String, Value> = self.col_w.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let mut v = json!({
            "view": self.view, "preview": self.preview, "sort": self.sort.0, "desc": self.sort.1, "hidden": self.hidden,
            "w": self.size.0, "h": self.size.1,
            "chooser_dir": self.chooser_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "group": self.group, "icon": self.icon, "gap": self.gap, "icon_text": self.icon_text,
            "item_info": self.item_info, "icon_preview": self.icon_preview, "list_cols": self.list_cols, "col_w": col_w,
            "list_text": self.list_text, "rel_dates": self.rel_dates, "calc_sizes": self.calc_sizes,
            "col_icons": self.col_icons, "col_preview": self.col_preview, "path_bar": self.path_bar,
            "status_bar": self.status_bar, "tab_bar": self.tab_bar, "sidebar": self.sidebar, "side_w": self.side_w,
            "collapsed": self.collapsed, "side_hidden": self.side_hidden, "tags_hidden": self.tags_hidden,
            "new_window": self.new_window, "open_tabs": self.open_tabs, "show_ext": self.show_ext,
            "warn_ext": self.warn_ext, "warn_trash": self.warn_trash, "trash_30": self.trash_30,
            "folders_first": self.folders_first, "scope": self.scope, "last_scope": self.last_scope,
            "open_with": self.open_with,
        });
        v["side_smart"] = json!(self.side_smart);
        v["toolbar"] = json!(self.toolbar);
        v["servers"] = json!(self.servers);
        v["recent_servers"] = json!(self.recent_servers);
        v
    }

    pub fn load() -> Self {
        let v: Value = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self::from_value(&v)
    }

    pub fn save(&self) {
        if std::env::var_os("AQUA_FINDER_NO_SAVE").is_some() {
            return;
        }
        let p = settings_path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_string_pretty(&self.to_value()).unwrap_or_default());
    }

    pub fn spacing(&self) -> f32 {
        (self.gap - 50.0) * 0.5
    }

    pub fn zoom(&self) -> f32 {
        zoom_of(self.icon)
    }
}

pub fn zoom_of(icon: f32) -> f32 {
    ((icon.clamp(16.0, 512.0) - 16.0) / 496.0).sqrt()
}

pub fn icon_of(zoom: f32) -> f32 {
    let z = zoom.clamp(0.0, 1.0);
    (16.0 + 496.0 * z * z).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = Settings { view: 2, group: 3, icon: 128.0, list_cols: vec![2, 4, 6], ..Default::default() };
        s.col_w.insert(2, 190.0);
        s.collapsed = vec!["tags".into()];
        s.open_with.insert("/a/b.txt".into(), "org.gnome.TextEditor.desktop".into());
        s.path_bar = true;
        s.toolbar = vec!["back".into(), "delete".into()];
        s.servers = vec!["smb://nas/share".into()];
        s.side_smart = vec!["/x/Big.json".into()];
        let back = Settings::from_value(&s.to_value());
        assert_eq!(back, s);
    }

    #[test]
    fn defaults_and_clamping() {
        let s = Settings::from_value(&json!({"view": 9, "icon": 2000, "list_cols": [0, 3, 99], "side_w": 10}));
        assert_eq!(s.view, 3);
        assert_eq!(s.icon, 512.0);
        assert_eq!(s.list_cols, vec![3]);
        assert_eq!(s.side_w, 140.0);
        assert!(Settings::from_value(&Value::Null).sidebar);
    }

    #[test]
    fn zoom_mapping() {
        for icon in [16.0, 32.0, 64.0, 128.0, 256.0, 512.0] {
            assert_eq!(icon_of(zoom_of(icon)), icon);
        }
        assert_eq!(icon_of(-1.0), 16.0);
        assert_eq!(icon_of(2.0), 512.0);
        assert_eq!(Settings::default().spacing(), 0.0);
    }
}
