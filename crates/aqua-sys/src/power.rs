//! Battery / AC state from UPower, falling back to sysfs.
use crate::system_bus;
use zbus::blocking::Proxy;

#[derive(Clone, Debug, PartialEq)]
pub struct Power {
    /// 0..1, None when there is no battery.
    pub level: Option<f32>,
    pub charging: bool,
    pub on_ac: bool,
    /// Minutes until empty/full.
    pub minutes: Option<u32>,
    pub source: &'static str,
    pub low_power: bool,
}

impl Default for Power {
    fn default() -> Self {
        Self { level: None, charging: false, on_ac: true, minutes: None, source: "none", low_power: false }
    }
}

fn upower() -> Option<Power> {
    let c = system_bus()?;
    let dev = Proxy::new(
        &c,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/devices/DisplayDevice",
        "org.freedesktop.UPower.Device",
    )
    .ok()?;
    let present: bool = dev.get_property("IsPresent").ok()?;
    let up = Proxy::new(&c, "org.freedesktop.UPower", "/org/freedesktop/UPower", "org.freedesktop.UPower").ok()?;
    let on_battery: bool = up.get_property("OnBattery").unwrap_or(false);
    if !present {
        return Some(Power { on_ac: true, source: "upower", ..Default::default() });
    }
    let pct: f64 = dev.get_property("Percentage").unwrap_or(0.0);
    let state: u32 = dev.get_property("State").unwrap_or(0);
    let tte: i64 = dev.get_property("TimeToEmpty").unwrap_or(0);
    let ttf: i64 = dev.get_property("TimeToFull").unwrap_or(0);
    let charging = state == 1;
    let secs = if charging { ttf } else { tte };
    let low_power = Proxy::new(&c, "net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles")
        .ok()
        .and_then(|p| p.get_property::<String>("ActiveProfile").ok())
        .map(|p| p == "power-saver")
        .unwrap_or(false);
    Some(Power {
        level: Some(pct as f32 / 100.0),
        charging,
        on_ac: !on_battery,
        minutes: (secs > 0).then_some((secs / 60) as u32),
        source: "upower",
        low_power,
    })
}

fn sysfs() -> Power {
    let mut p = Power { source: "sysfs", ..Default::default() };
    let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return p };
    let mut has_bat = false;
    let mut ac = false;
    for e in rd.flatten() {
        let d = e.path();
        let rd = |f: &str| std::fs::read_to_string(d.join(f)).unwrap_or_default().trim().to_string();
        match rd("type").as_str() {
            "Battery" if rd("scope") != "Device" => {
                has_bat = true;
                p.level = rd("capacity").parse::<f32>().ok().map(|v| v / 100.0);
                p.charging = rd("status") == "Charging";
            }
            "Mains" | "USB" => ac |= rd("online") == "1",
            _ => {}
        }
    }
    p.on_ac = !has_bat || ac;
    p
}

pub fn probe() -> Power {
    upower()
        .filter(|p| {
            p.level.is_some() || std::fs::read_dir("/sys/class/power_supply").map(|r| r.count() == 0).unwrap_or(true)
        })
        .unwrap_or_else(sysfs)
}

/// Toggle power-profiles-daemon's power-saver profile ("Low Power Mode").
pub fn set_low_power(on: bool) {
    crate::spawn_cmd(move || {
        let Some(c) = system_bus() else { return };
        if let Ok(p) =
            Proxy::new(&c, "net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles")
        {
            let _ = p.set_property("ActiveProfile", if on { "power-saver" } else { "balanced" });
        }
    });
}
