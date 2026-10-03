//! Well-known directories: the session runtime dir and the XDG user directories.
use std::path::{Path, PathBuf};

/// `$XDG_RUNTIME_DIR`, else `/tmp`.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| "/tmp".into())
}

/// Control socket of the running compositor: `$AQUA_SOCKET`, else
/// `$XDG_RUNTIME_DIR/aqua.sock` (`aqua-$WAYLAND_DISPLAY.sock` with `AQUA_SOCKET_PER_DISPLAY`).
pub fn control_socket() -> PathBuf {
    if let Some(p) = std::env::var_os("AQUA_SOCKET") {
        return PathBuf::from(p);
    }
    let disp = std::env::var("WAYLAND_DISPLAY").ok().filter(|d| !d.contains('/'));
    match disp {
        Some(d) if std::env::var_os("AQUA_SOCKET_PER_DISPLAY").is_some() => {
            runtime_dir().join(format!("aqua-{d}.sock"))
        }
        _ => runtime_dir().join("aqua.sock"),
    }
}

/// `$HOME`, else `/`.
pub fn home() -> PathBuf {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| "/".into())
}

/// An XDG user directory (`name` = "DESKTOP", "DOWNLOAD", "PICTURES", "VIDEOS", …) from
/// `~/.config/user-dirs.dirs`, else `~/<fallback>`.
pub fn user_dir(name: &str, fallback: &str) -> PathBuf {
    let home = home();
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(cfg.join("user-dirs.dirs")).unwrap_or_default();
    parse_user_dir(&text, &home, name).unwrap_or_else(|| home.join(fallback))
}

/// Look up `XDG_<name>_DIR` in the contents of a `user-dirs.dirs` file.
pub fn parse_user_dir(text: &str, home: &Path, name: &str) -> Option<PathBuf> {
    let key = format!("XDG_{name}_DIR=");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.strip_prefix(&key))
        .map(|v| v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy()))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_file() {
        let text =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Рабочий стол\"\nXDG_DOWNLOAD_DIR=\"/data/dl\"\nXDG_MUSIC_DIR=\"\"\n";
        let home = Path::new("/home/u");
        assert_eq!(parse_user_dir(text, home, "DESKTOP"), Some(PathBuf::from("/home/u/Рабочий стол")));
        assert_eq!(parse_user_dir(text, home, "DOWNLOAD"), Some(PathBuf::from("/data/dl")));
        assert_eq!(parse_user_dir(text, home, "MUSIC"), None);
        assert_eq!(parse_user_dir(text, home, "PICTURES"), None);
        assert_eq!(parse_user_dir("#XDG_DESKTOP_DIR=\"/x\"", home, "DESKTOP"), None);
    }
}
