//! System status for menu-bar extras, backed by `aqua-sys` (NetworkManager/iwd,
//! UPower, BlueZ, PipeWire). Everything is cached and refreshed off-thread.
pub use aqua_sys::{Network, Power};

/// (Wi-Fi backend available, networks).
pub fn wifi_networks() -> (bool, Vec<Network>) {
    let s = aqua_sys::snapshot();
    (s.net.backend != aqua_sys::NetBackend::None && s.net.has_wifi, s.net.networks)
}

pub fn power() -> Power {
    aqua_sys::snapshot().power
}

/// Configured XKB layouts, e.g. ["us", "ru"] (from the environment when the shell runs standalone).
pub fn layouts() -> Vec<String> {
    let v = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_else(|_| "us".into());
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Display name of an XKB layout ("ru", "us(dvorak)"), for common ones,
/// otherwise the xkeyboard-config description.
pub fn layout_name(code: &str) -> &'static str {
    let base = code.split('(').next().unwrap_or(code);
    let plain = !code.contains('(');
    match base {
        "us" if plain => "ABC",
        "ru" if plain => "Russian – PC",
        _ => {
            use std::collections::HashMap;
            use std::sync::{Mutex, OnceLock};
            static CAT: OnceLock<aqua_config::xkb::Catalogue> = OnceLock::new();
            static NAMES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
            let names = NAMES.get_or_init(Default::default);
            let mut m = names.lock().unwrap();
            if let Some(n) = m.get(code) {
                return n;
            }
            let cat = CAT.get_or_init(aqua_config::xkb::catalogue);
            let n: &'static str = Box::leak(cat.describe(code, "").into_boxed_str());
            m.insert(code.to_string(), n);
            n
        }
    }
}

/// Two-letter badge shown in the menu bar input menu ("EN", "RU", …).
pub fn layout_badge(code: &str) -> String {
    let base = code.split('(').next().unwrap_or(code);
    match base {
        "us" | "gb" => "A".into(),
        b => b.chars().take(2).collect::<String>().to_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_badges() {
        assert_eq!(layout_badge("us"), "A");
        assert_eq!(layout_badge("gb(extd)"), "A");
        assert_eq!(layout_badge("ru"), "RU");
        assert_eq!(layout_badge("de(nodeadkeys)"), "DE");
    }

    #[test]
    fn layout_names() {
        assert_eq!(layout_name("us"), "ABC");
        assert_eq!(layout_name("ru"), "Russian – PC");
        assert!(!layout_name("us(dvorak)").is_empty());
        assert!(std::ptr::eq(layout_name("de"), layout_name("de")), "descriptions are cached");
    }
}
