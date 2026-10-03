//! Real system information for System Settings pages (General → About/Storage/Software
//! Update/Date & Time/Language & Region, Users & Groups). Everything is read from the
//! running system; nothing is invented.
use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    if !o.status.success() && o.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).to_string())
}

pub fn os_name() -> String {
    let t = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let get =
        |k: &str| t.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_string());
    let name = get("PRETTY_NAME").or_else(|| get("NAME")).unwrap_or_else(|| "Linux".into());
    let kernel =
        std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default();
    if kernel.is_empty() {
        name
    } else {
        format!("{name} · Linux {kernel}")
    }
}

pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e12 {
        format!("{:.2} {}", b / 1e12, crate::tr("TB"))
    } else if b >= 1e9 {
        format!("{:.1} {}", b / 1e9, crate::tr("GB"))
    } else if b >= 1e6 {
        format!("{:.0} {}", b / 1e6, crate::tr("MB"))
    } else {
        format!("{:.0} {}", b / 1e3, crate::tr("KB"))
    }
}

/// Mounted local file systems: (name, mount point, "X GB of Y GB used", used fraction).
pub fn volumes() -> Vec<(String, String, String, f32)> {
    let Some(out) = run(
        "df",
        &[
            "-B1",
            "--output=source,fstype,size,used,target",
            "-x",
            "tmpfs",
            "-x",
            "devtmpfs",
            "-x",
            "squashfs",
            "-x",
            "overlay",
            "-x",
            "efivarfs",
            "-x",
            "ramfs",
        ],
    ) else {
        return vec![];
    };
    let mut v = vec![];
    let mut seen_src: Vec<String> = vec![];
    for l in out.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let (src, fs, size, used) = (f[0], f[1], f[2].parse::<u64>().unwrap_or(0), f[3].parse::<u64>().unwrap_or(0));
        let target = f[4..].join(" ");
        if !std::path::Path::new(&target).is_dir()
            || ["/etc", "/proc", "/sys", "/dev", "/run", "/var/lib/docker"]
                .iter()
                .any(|p| target == *p || target.starts_with(&format!("{p}/")))
        {
            continue;
        }
        if size < 256 * 1024 * 1024
            || !src.starts_with('/')
            || seen_src.iter().any(|s| s == src)
            || target.starts_with("/boot") && size < 4_000_000_000
        {
            continue;
        }
        seen_src.push(src.to_string());
        let label = std::fs::read_dir("/dev/disk/by-label").ok().and_then(|rd| {
            rd.flatten()
                .find(|e| std::fs::canonicalize(e.path()).ok().map(|p| p.to_string_lossy() == src).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().replace("\\x20", " "))
        });
        let name = label.unwrap_or_else(|| match target.as_str() {
            "/" => crate::tr("System").into(),
            "/home" => crate::tr("Home").into(),
            t => t.rsplit('/').next().unwrap_or(t).to_string(),
        });
        v.push((
            name,
            format!("{target} · {src} · {fs}"),
            crate::trf("{used} of {size} used", &[("used", &human_bytes(used)), ("size", &human_bytes(size))]),
            used as f32 / size.max(1) as f32,
        ));
    }
    v
}

/// Local user accounts: (full name, login, "Admin"/"Standard", is current, initials).
pub fn users() -> Vec<(String, String, String, bool, String)> {
    let me = std::env::var("USER").unwrap_or_default();
    let groups = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let admins: Vec<String> = groups
        .lines()
        .filter(|l| ["wheel:", "sudo:", "admin:"].iter().any(|g| l.starts_with(g)))
        .flat_map(|l| l.rsplit(':').next().unwrap_or("").split(',').map(str::to_string).collect::<Vec<_>>())
        .collect();
    let mut v = vec![];
    for l in std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines() {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() < 7 {
            continue;
        }
        let uid: u32 = f[2].parse().unwrap_or(0);
        let shell = f[6];
        if !(1000..60000).contains(&uid) && f[0] != me || shell.ends_with("nologin") || shell.ends_with("false") {
            continue;
        }
        let full = f[4].split(',').next().unwrap_or("").trim();
        let name = if full.is_empty() { f[0].to_string() } else { full.to_string() };
        let initials: String =
            name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
        let admin = admins.iter().any(|a| a == f[0]);
        v.push((
            name,
            f[0].to_string(),
            crate::tr(if admin { "Admin" } else { "Standard" }).into(),
            f[0] == me,
            initials,
        ));
    }
    v.sort_by_key(|u| !u.3);
    v
}

pub fn avatar() -> Option<std::path::PathBuf> {
    let me = std::env::var("USER").unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    [
        home.join(".face"),
        home.join(".face.icon"),
        std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{me}")),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub struct TimeInfo {
    pub timezone: String,
    pub ntp: bool,
    pub can_ntp: bool,
}

pub fn time_info() -> TimeInfo {
    let mut t = TimeInfo { timezone: String::new(), ntp: false, can_ntp: false };
    if let Some(out) = run("timedatectl", &["show"]) {
        for l in out.lines() {
            match l.split_once('=') {
                Some(("Timezone", v)) => t.timezone = v.to_string(),
                Some(("NTP", v)) => t.ntp = v == "yes",
                Some(("CanNTP", v)) => t.can_ntp = v == "yes",
                _ => {}
            }
        }
    }
    if t.timezone.is_empty() {
        t.timezone = std::fs::read_link("/etc/localtime")
            .ok()
            .map(|p| p.to_string_lossy().split("zoneinfo/").nth(1).unwrap_or("UTC").to_string())
            .unwrap_or_else(|| "UTC".into());
    }
    t
}

pub fn timezones() -> Vec<String> {
    if let Some(out) = run("timedatectl", &["list-timezones", "--no-pager"]) {
        let v: Vec<String> = out.lines().map(str::to_string).filter(|s| !s.is_empty()).collect();
        if !v.is_empty() {
            return v;
        }
    }
    let tab = std::fs::read_to_string("/usr/share/zoneinfo/tzdata.zi").unwrap_or_default();
    let mut v: Vec<String> = tab
        .lines()
        .filter_map(|l| l.strip_prefix("Z ").or_else(|| l.strip_prefix("L ").and_then(|x| x.split_whitespace().nth(1))))
        .map(|z| z.split_whitespace().next().unwrap_or("").to_string())
        .filter(|z| z.contains('/'))
        .collect();
    if !v.iter().any(|z| z == "UTC") {
        v.push("UTC".into());
    }
    v.sort();
    v.dedup();
    v
}

/// Current local date and time, formatted with the user's locale (`date '+%x %X'`).
pub fn now_text(h24: bool) -> String {
    let fmt = if h24 { "+%A %e %B %Y, %H:%M" } else { "+%A %e %B %Y, %l:%M %p" };
    run("date", &[fmt]).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// (available UTF-8 locales, system locale)
pub fn locales() -> (Vec<String>, String) {
    let mut list: Vec<String> = run("localectl", &["list-locales", "--no-pager"])
        .map(|o| o.lines().map(str::to_string).collect())
        .unwrap_or_default();
    if list.is_empty() {
        list = run("locale", &["-a"])
            .map(|o| o.lines().filter(|l| l.to_lowercase().contains("utf")).map(str::to_string).collect())
            .unwrap_or_default();
    }
    let mut cur = String::new();
    if let Some(o) = run("localectl", &["status", "--no-pager"]) {
        if let Some(l) = o.lines().find(|l| l.contains("LANG=")) {
            cur = l.split("LANG=").nth(1).unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
        }
    }
    if cur.is_empty() {
        cur = std::fs::read_to_string("/etc/locale.conf")
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("LANG=").map(|v| v.trim_matches('"').to_string())))
            .unwrap_or_default();
    }
    if cur.is_empty() {
        cur = std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into());
    }
    if !list.contains(&cur) {
        list.insert(0, cur.clone());
    }
    (list, cur)
}

/// "Russian (Russia)"-style description of a locale code.
pub fn describe_locale(code: &str) -> String {
    let base = code.split('.').next().unwrap_or(code);
    let (lang, region) = base.split_once('_').unwrap_or((base, ""));
    let l = match lang {
        "en" => "English",
        "ru" => "Russian",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "uk" => "Ukrainian",
        "be" => "Belarusian",
        "kk" => "Kazakh",
        "pl" => "Polish",
        "cs" => "Czech",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "fi" => "Finnish",
        "da" => "Danish",
        "nb" | "no" => "Norwegian",
        "tr" => "Turkish",
        "el" => "Greek",
        "he" => "Hebrew",
        "ar" => "Arabic",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        "hi" => "Hindi",
        "C" | "POSIX" => "Default",
        x => x,
    };
    let l = crate::tr(l);
    if region.is_empty() {
        l.to_string()
    } else {
        format!("{l} ({region})")
    }
}

/// Example of the date/number format of a locale (rendered by `date`/`printf` with LC_ALL).
pub fn locale_example(code: &str) -> String {
    Command::new("date")
        .arg("+%x  %X")
        .env("LC_ALL", code)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Pending package updates from the system package manager ("name old -> new" lines).
pub fn pending_updates(pm: &str) -> Result<Vec<String>, String> {
    let lines = |o: Option<String>| {
        o.unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect::<Vec<_>>()
    };
    match pm {
        "pacman" => {
            if aqua_sys::have("checkupdates") {
                let o = Command::new("checkupdates").output().map_err(|e| e.to_string())?;
                if o.status.code() == Some(1) {
                    return Err(String::from_utf8_lossy(&o.stderr)
                        .trim()
                        .lines()
                        .last()
                        .unwrap_or("checkupdates failed")
                        .to_string());
                }
                Ok(lines(Some(String::from_utf8_lossy(&o.stdout).to_string())))
            } else {
                Ok(lines(run("pacman", &["-Qu"])))
            }
        }
        "dnf" => {
            let o = Command::new("dnf").args(["-q", "check-update"]).output().map_err(|e| e.to_string())?;
            Ok(String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.split_whitespace().count() == 3 && !l.starts_with(' '))
                .map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
                .collect())
        }
        "apt" => {
            Ok(lines(run("apt", &["list", "--upgradable"])).into_iter().filter(|l| !l.starts_with("Listing")).collect())
        }
        "zypper" => Ok(lines(run("zypper", &["-q", "lu"]))
            .into_iter()
            .filter(|l| l.contains('|') && !l.starts_with("S ") && !l.starts_with("--"))
            .collect()),
        "" => Err(crate::tr("No supported package manager found").into()),
        _ => Err(crate::trf("{pm} is not supported", &[("pm", &pm)])),
    }
}

pub fn update_command(pm: &str) -> &'static str {
    match pm {
        "pacman" => "sudo pacman -Syu",
        "dnf" => "sudo dnf upgrade",
        "apt" => "sudo apt update && sudo apt upgrade",
        "zypper" => "sudo zypper update",
        _ => "",
    }
}

/// Run a shell command in the user's terminal (first installed of `cfg.terminal`).
pub fn run_in_terminal(terminals: &str, cmd: &str) -> bool {
    let script = format!("{cmd}; echo; read -r -p 'Press Enter to close…' _");
    for t in terminals.split('|').map(str::trim).filter(|t| !t.is_empty()) {
        let bin = t.split_whitespace().next().unwrap_or(t);
        if !aqua_sys::have(bin) {
            continue;
        }
        let sep: &[&str] = match bin {
            "gnome-terminal" | "kgx" | "ptyxis" | "kitty" | "foot" => &["--"],
            "wezterm" => &["start", "--"],
            _ => &["-e"],
        };
        let mut c = Command::new(bin);
        c.args(sep).args(["sh", "-c", &script]);
        if c.spawn().is_ok() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_descriptions() {
        let t = crate::tr;
        assert_eq!(describe_locale("en_US.UTF-8"), format!("{} (US)", t("English")));
        assert_eq!(describe_locale("ru_RU.UTF-8"), format!("{} (RU)", t("Russian")));
        assert_eq!(describe_locale("de"), t("German"));
        assert_eq!(describe_locale("C.UTF-8"), t("Default"));
        assert_eq!(describe_locale("xx_YY"), "xx (YY)");
    }

    #[test]
    fn update_commands() {
        assert_eq!(update_command("pacman"), "sudo pacman -Syu");
        assert!(update_command("apt").contains("apt upgrade"));
        assert_eq!(update_command("unknown"), "");
    }
}
