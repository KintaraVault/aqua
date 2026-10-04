use crate::model::Scope;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub auto_update: bool,
    pub auto_check: bool,
    pub notify: bool,
    pub include_system: bool,
    pub flatpak_scope: Scope,
    pub use_flathub: bool,
    pub use_native: bool,
    pub use_aur: bool,
    pub show_reviews: bool,
    pub preferred: String,
    pub reviewer_name: String,
    pub hidden: Vec<String>,
    pub dismissed: Vec<String>,
    pub last_check: i64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            auto_update: false,
            auto_check: true,
            notify: true,
            include_system: true,
            flatpak_scope: Scope::User,
            use_flathub: true,
            use_native: true,
            use_aur: true,
            show_reviews: true,
            preferred: "flatpak".into(),
            reviewer_name: String::new(),
            hidden: vec![],
            dismissed: vec![],
            last_check: 0,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("AQUA_STORE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("aqua/store.toml"))
}

impl Prefs {
    pub fn load() -> Prefs {
        Prefs::load_from(&path())
    }

    pub fn load_from(p: &std::path::Path) -> Prefs {
        std::fs::read_to_string(p).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        self.save_to(&path());
    }

    pub fn save_to(&self, p: &std::path::Path) {
        if let Ok(s) = toml::to_string_pretty(self) {
            crate::http::write_atomic(p, s.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial() {
        let dir = std::env::temp_dir().join(format!("aqua-store-prefs-{}", std::process::id()));
        let p = dir.join("store.toml");
        let a = Prefs {
            auto_update: true,
            flatpak_scope: Scope::System,
            hidden: vec!["flatpak:x.y".into()],
            ..Default::default()
        };
        a.save_to(&p);
        assert_eq!(Prefs::load_from(&p), a);
        std::fs::write(&p, "use_aur = false\n").unwrap();
        let b = Prefs::load_from(&p);
        assert!(!b.use_aur);
        assert!(b.auto_check);
        let _ = std::fs::remove_dir_all(dir);
    }
}
