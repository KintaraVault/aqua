//! Bluetooth via BlueZ (org.bluez on the system bus).
use crate::{spawn_cmd, system_bus};
use std::collections::HashMap;
use zbus::blocking::Proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BtDevice {
    pub path: String,
    pub name: String,
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bluetooth {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub adapter: String,
    pub devices: Vec<BtDevice>,
}

type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn objects(c: &zbus::blocking::Connection) -> Option<Managed> {
    Proxy::new(c, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager").ok()?.call("GetManagedObjects", &()).ok()
}

fn s(v: Option<&OwnedValue>) -> String {
    v.and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
}
fn b(v: Option<&OwnedValue>) -> bool {
    v.and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false)
}

pub fn probe() -> Bluetooth {
    let mut bt = Bluetooth::default();
    let Some(c) = system_bus() else { return bt };
    let Some(objs) = objects(&c) else { return bt };
    for (p, i) in &objs {
        if let Some(a) = i.get("org.bluez.Adapter1") {
            if bt.adapter.is_empty() {
                bt.available = true;
                bt.adapter = p.as_str().to_string();
                bt.powered = b(a.get("Powered"));
                bt.discovering = b(a.get("Discovering"));
            }
        }
    }
    for (p, i) in &objs {
        if let Some(d) = i.get("org.bluez.Device1") {
            let name = {
                let a = s(d.get("Alias"));
                if a.is_empty() {
                    s(d.get("Name"))
                } else {
                    a
                }
            };
            if name.is_empty() {
                continue;
            }
            let battery = i
                .get("org.bluez.Battery1")
                .and_then(|bb| bb.get("Percentage"))
                .and_then(|v| u8::try_from(v.clone()).ok());
            bt.devices.push(BtDevice {
                path: p.as_str().to_string(),
                name,
                icon: s(d.get("Icon")),
                paired: b(d.get("Paired")),
                connected: b(d.get("Connected")),
                battery,
            });
        }
    }
    bt.devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then(a.name.cmp(&b.name)));
    bt
}

pub fn set_powered(on: bool) {
    crate::patch(|s| s.bt.powered = on);
    spawn_cmd(move || {
        let Some(c) = system_bus() else { return };
        let ad = probe().adapter;
        if ad.is_empty() {
            return;
        }
        if let Ok(p) = Proxy::new(&c, "org.bluez", ad, "org.freedesktop.DBus.Properties") {
            let _ = p.call::<_, _, ()>("Set", &("org.bluez.Adapter1", "Powered", Value::from(on)));
        }
    });
}

pub fn set_discovering(on: bool) {
    spawn_cmd(move || {
        let Some(c) = system_bus() else { return };
        let ad = probe().adapter;
        if let Ok(p) = Proxy::new(&c, "org.bluez", ad, "org.bluez.Adapter1") {
            let _ = p.call::<_, _, ()>(if on { "StartDiscovery" } else { "StopDiscovery" }, &());
        }
    });
}

/// Connect (pairing first if needed) or disconnect a device.
pub fn toggle_device(path: String, connect: bool) {
    spawn_cmd(move || {
        let Some(c) = system_bus() else { return };
        let Ok(p) = Proxy::new(&c, "org.bluez", path, "org.bluez.Device1") else { return };
        if connect {
            if !p.get_property::<bool>("Paired").unwrap_or(false) {
                let _ = p.call::<_, _, ()>("Pair", &());
                let _ = p.set_property("Trusted", true);
            }
            let _ = p.call::<_, _, ()>("Connect", &());
        } else {
            let _ = p.call::<_, _, ()>("Disconnect", &());
        }
    });
}
