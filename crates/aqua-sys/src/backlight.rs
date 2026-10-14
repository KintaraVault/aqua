//! Display brightness via /sys/class/backlight. Writing uses logind's
//! `Session.SetBrightness` (works unprivileged for the active session) and falls
//! back to writing sysfs directly (udev rule / root).
use crate::system_bus;
use std::path::PathBuf;

fn device() -> Option<(String, PathBuf)> {
    if let Ok(p) = std::env::var("AQUA_BACKLIGHT") {
        let pb = PathBuf::from(&p);
        return Some((pb.file_name()?.to_string_lossy().into_owned(), pb));
    }
    let mut best: Option<(i32, String, PathBuf)> = None;
    for e in std::fs::read_dir("/sys/class/backlight").ok()?.flatten() {
        let ty = std::fs::read_to_string(e.path().join("type")).unwrap_or_default();
        let prio = match ty.trim() {
            "firmware" => 3,
            "platform" => 2,
            _ => 1,
        };
        if best.as_ref().map(|b| prio > b.0).unwrap_or(true) {
            best = Some((prio, e.file_name().to_string_lossy().into_owned(), e.path()));
        }
    }
    best.map(|b| (b.1, b.2))
}

fn read(p: &std::path::Path, f: &str) -> Option<f32> {
    std::fs::read_to_string(p.join(f)).ok()?.trim().parse().ok()
}

pub fn get() -> Option<f32> {
    let (_, p) = device()?;
    let max = read(&p, "max_brightness")?;
    let cur = read(&p, "brightness")?;
    (max > 0.0).then(|| cur / max)
}

pub fn set(v: f32) {
    let v = v.clamp(0.02, 1.0);
    crate::patch(|s| s.brightness = Some(v));
    std::thread::spawn(move || {
        let Some((name, p)) = device() else { return };
        let Some(max) = read(&p, "max_brightness") else { return };
        let raw = (v * max).round().max(1.0) as u32;
        let ok = system_bus()
            .and_then(|c| {
                zbus::blocking::Proxy::new(
                    &c,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                )
                .ok()
                .map(|p| p.call::<_, _, ()>("SetBrightness", &("backlight", name.as_str(), raw)).is_ok())
            })
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::write(p.join("brightness"), raw.to_string());
        }
    });
}

fn kbd_device() -> Option<(String, PathBuf)> {
    let mut found: Vec<_> = std::fs::read_dir("/sys/class/leds")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("kbd_backlight"))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Keyboard backlight level (0..=1), `None` when the keyboard has no backlight.
pub fn kbd_get() -> Option<f32> {
    let (_, p) = kbd_device()?;
    let max = read(&p, "max_brightness")?;
    let cur = read(&p, "brightness")?;
    (max > 0.0).then(|| cur / max)
}

/// Step the keyboard backlight by `steps` hardware levels (or 1/16 on fine-grained
/// LEDs); returns the new level.
pub fn kbd_step(steps: i32) -> Option<f32> {
    let (name, p) = kbd_device()?;
    let max = read(&p, "max_brightness")?;
    let cur = read(&p, "brightness")?;
    if max <= 0.0 {
        return None;
    }
    let unit = if max > 16.0 { (max / 16.0).round() } else { 1.0 };
    let raw = (cur + steps as f32 * unit).clamp(0.0, max).round() as u32;
    std::thread::spawn(move || {
        let ok = system_bus()
            .and_then(|c| {
                zbus::blocking::Proxy::new(
                    &c,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                )
                .ok()
                .map(|p| p.call::<_, _, ()>("SetBrightness", &("leds", name.as_str(), raw)).is_ok())
            })
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::write(p.join("brightness"), raw.to_string());
        }
    });
    Some(raw as f32 / max)
}
