//! logind power actions.
use crate::system_bus;
use zbus::blocking::Proxy;

fn manager(c: &zbus::blocking::Connection) -> Option<Proxy<'_>> {
    Proxy::new(c, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").ok()
}

fn call(method: &'static str) -> Result<(), String> {
    if std::env::var_os("AQUA_DRY_POWER").is_some() {
        tracing::info!("AQUA_DRY_POWER: would call logind {method}");
        return Ok(());
    }
    let c = system_bus().ok_or("no system bus")?;
    let m = manager(&c).ok_or("no logind")?;
    m.call::<_, _, ()>(method, &(true,)).map_err(|e| e.to_string())
}

pub fn suspend() -> Result<(), String> {
    call("Suspend")
}
pub fn power_off() -> Result<(), String> {
    call("PowerOff")
}
pub fn reboot() -> Result<(), String> {
    call("Reboot")
}
pub fn can(method: &str) -> bool {
    let Some(c) = system_bus() else { return false };
    manager(&c)
        .and_then(|m| m.call::<_, _, String>(method, &()).ok())
        .map(|s| s == "yes" || s == "challenge")
        .unwrap_or(false)
}

/// Machine description for "About This Computer".
pub fn about() -> Vec<(String, String)> {
    let mut v = vec![];
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let get =
        |k: &str| os.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|s| s.trim_matches('"').to_string());
    v.push(("OS".into(), get("PRETTY_NAME").unwrap_or_else(|| "Linux".into())));
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "Unknown".into());
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    v.push(("Chip".into(), format!("{cpu} ({ncpu} cores)")));
    let mem = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| s.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|k| k.parse::<f64>().ok()))
        .map(|kb| format!("{:.0} GB", kb / 1024.0 / 1024.0))
        .unwrap_or_default();
    v.push(("Memory".into(), mem));
    let host = ["/etc/hostname", "/proc/sys/kernel/hostname"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    v.push(("Name".into(), host));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    v.push(("Kernel".into(), kernel));
    for g in gpus() {
        v.push(("Graphics".into(), g));
    }
    if let Some(st) = storage() {
        v.push(("Storage".into(), st));
    }
    v
}

/// Graphics adapters: `lspci` names when available, else vendor + kernel driver from sysfs.
pub fn gpus() -> Vec<String> {
    if let Some(out) = crate::run("lspci", &[]) {
        let v: Vec<String> = out
            .lines()
            .filter(|l| {
                l.contains("VGA compatible controller")
                    || l.contains("3D controller")
                    || l.contains("Display controller")
            })
            .filter_map(|l| {
                l.split_once(": ").map(|x| x.1).map(|s| s.split(" (rev").next().unwrap_or(s).trim().to_string())
            })
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("card") && !n.contains('-'))
            .collect();
        names.sort();
        for n in names {
            let dev = format!("/sys/class/drm/{n}/device");
            let vendor = std::fs::read_to_string(format!("{dev}/vendor")).unwrap_or_default();
            let vendor = match vendor.trim() {
                "0x10de" => "NVIDIA",
                "0x1002" => "AMD",
                "0x8086" => "Intel",
                "0x1af4" => "Virtio",
                "0x15ad" => "VMware",
                "0x1234" => "QEMU",
                _ => "GPU",
            };
            let driver = std::fs::read_link(format!("{dev}/driver"))
                .ok()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
                .unwrap_or_default();
            v.push(if driver.is_empty() { vendor.to_string() } else { format!("{vendor} ({driver})") });
        }
    }
    v
}

/// "123 GB available of 512 GB" for the root filesystem.
pub fn storage() -> Option<String> {
    let out = crate::run("df", &["-k", "--output=size,avail", "/"])?;
    let line = out.lines().nth(1)?;
    let mut it = line.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
    let (size, avail) = (it.next()?, it.next()?);
    let gb = |k: f64| k / 1024.0 / 1024.0;
    Some(format!("{:.0} GB available of {:.0} GB", gb(avail), gb(size)))
}
