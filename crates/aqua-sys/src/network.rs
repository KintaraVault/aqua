//! Wi-Fi through NetworkManager or iwd (D-Bus), with `nmcli` as last resort.
use crate::{run, spawn_cmd, system_bus};
use std::collections::HashMap;
use zbus::blocking::Proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NetBackend {
    NetworkManager,
    Iwd,
    Nmcli,
    #[default]
    None,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// 0..=100
    pub signal: u8,
    pub secure: bool,
    pub active: bool,
    pub known: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct NetState {
    pub backend: NetBackend,
    pub wifi_enabled: bool,
    pub has_wifi: bool,
    pub networks: Vec<Network>,
    /// Wired/other active connection name.
    pub wired: Option<String>,
}

impl NetState {
    pub fn active(&self) -> Option<&Network> {
        self.networks.iter().find(|n| n.active)
    }
}

const NM: &str = "org.freedesktop.NetworkManager";
const IWD: &str = "net.connman.iwd";

fn nm_proxy<'a>(c: &'a zbus::blocking::Connection, path: &'a str, iface: &'a str) -> Option<Proxy<'a>> {
    Proxy::new(c, NM, path.to_string(), iface.to_string()).ok()
}

fn name_has_owner(c: &zbus::blocking::Connection, name: &str) -> bool {
    let Ok(p) = Proxy::new(c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus") else {
        return false;
    };
    p.call::<_, _, bool>("NameHasOwner", &(name,)).unwrap_or(false)
}

fn nm_wifi_devices(c: &zbus::blocking::Connection) -> Vec<OwnedObjectPath> {
    let Some(nm) = nm_proxy(c, "/org/freedesktop/NetworkManager", NM) else { return vec![] };
    let devs: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).unwrap_or_default();
    devs.into_iter()
        .filter(|d| {
            nm_proxy(c, d.as_str(), "org.freedesktop.NetworkManager.Device")
                .and_then(|p| p.get_property::<u32>("DeviceType").ok())
                == Some(2)
        })
        .collect()
}

fn probe_nm(c: &zbus::blocking::Connection) -> Option<NetState> {
    if !name_has_owner(c, NM) {
        return None;
    }
    let nm = nm_proxy(c, "/org/freedesktop/NetworkManager", NM)?;
    let mut st = NetState {
        backend: NetBackend::NetworkManager,
        wifi_enabled: nm.get_property("WirelessEnabled").unwrap_or(false),
        ..Default::default()
    };
    let mut known = std::collections::HashSet::new();
    if let Some(settings) =
        nm_proxy(c, "/org/freedesktop/NetworkManager/Settings", "org.freedesktop.NetworkManager.Settings")
    {
        let conns: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).unwrap_or_default();
        for p in conns {
            if let Some(cp) = nm_proxy(c, p.as_str(), "org.freedesktop.NetworkManager.Settings.Connection") {
                if let Ok(s) = cp.call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &()) {
                    if let Some(ssid) = s
                        .get("802-11-wireless")
                        .and_then(|w| w.get("ssid"))
                        .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
                    {
                        known.insert(String::from_utf8_lossy(&ssid).into_owned());
                    }
                }
            }
        }
    }
    let actives: Vec<OwnedObjectPath> = nm.get_property("ActiveConnections").unwrap_or_default();
    for a in &actives {
        if let Some(ap) = nm_proxy(c, a.as_str(), "org.freedesktop.NetworkManager.Connection.Active") {
            let ty: String = ap.get_property("Type").unwrap_or_default();
            if ty == "802-3-ethernet" {
                st.wired = ap.get_property("Id").ok();
            }
        }
    }
    for dev in nm_wifi_devices(c) {
        st.has_wifi = true;
        let Some(w) = nm_proxy(c, dev.as_str(), "org.freedesktop.NetworkManager.Device.Wireless") else { continue };
        let active: OwnedObjectPath =
            w.get_property("ActiveAccessPoint").unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());
        let aps: Vec<OwnedObjectPath> = w.call("GetAllAccessPoints", &()).unwrap_or_default();
        for ap in aps {
            let Some(p) = nm_proxy(c, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint") else { continue };
            let ssid: Vec<u8> = p.get_property("Ssid").unwrap_or_default();
            if ssid.is_empty() {
                continue;
            }
            let ssid = String::from_utf8_lossy(&ssid).into_owned();
            let wpa: u32 = p.get_property("WpaFlags").unwrap_or(0);
            let rsn: u32 = p.get_property("RsnFlags").unwrap_or(0);
            let flags: u32 = p.get_property("Flags").unwrap_or(0);
            st.networks.push(Network {
                known: known.contains(&ssid),
                ssid,
                signal: p.get_property::<u8>("Strength").unwrap_or(0),
                secure: wpa != 0 || rsn != 0 || flags & 1 != 0,
                active: ap == active,
            });
        }
    }
    Some(st)
}

type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn managed_objects(c: &zbus::blocking::Connection, svc: &str) -> Option<Managed> {
    let p = Proxy::new(c, svc.to_string(), "/", "org.freedesktop.DBus.ObjectManager").ok()?;
    p.call("GetManagedObjects", &()).ok()
}

fn ov_str(v: Option<&OwnedValue>) -> String {
    v.and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
}
fn ov_bool(v: Option<&OwnedValue>) -> bool {
    v.and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false)
}

fn probe_iwd(c: &zbus::blocking::Connection) -> Option<NetState> {
    if !name_has_owner(c, IWD) {
        return None;
    }
    let objs = managed_objects(c, IWD)?;
    let mut st = NetState { backend: NetBackend::Iwd, ..Default::default() };
    for ifaces in objs.values() {
        if let Some(dev) = ifaces.get("net.connman.iwd.Device") {
            st.has_wifi = true;
            st.wifi_enabled |= ov_bool(dev.get("Powered"));
        }
    }
    for (path, ifaces) in &objs {
        if !ifaces.contains_key("net.connman.iwd.Station") {
            continue;
        }
        let Ok(p) = Proxy::new(c, IWD, path.as_str().to_string(), "net.connman.iwd.Station") else { continue };
        let ordered: Vec<(OwnedObjectPath, i16)> = p.call("GetOrderedNetworks", &()).unwrap_or_default();
        for (np, dbm) in ordered {
            let Some(n) = objs.get(&np).and_then(|i| i.get("net.connman.iwd.Network")) else { continue };
            let signal = ((dbm as f32 / 100.0 + 90.0) / 60.0 * 100.0).clamp(0.0, 100.0) as u8;
            st.networks.push(Network {
                ssid: ov_str(n.get("Name")),
                signal,
                secure: ov_str(n.get("Type")) != "open",
                active: ov_bool(n.get("Connected")),
                known: n.contains_key("KnownNetwork"),
            });
        }
    }
    Some(st)
}

fn probe_nmcli() -> Option<NetState> {
    let radio = run("nmcli", &["-t", "radio", "wifi"])?;
    let mut st = NetState {
        backend: NetBackend::Nmcli,
        wifi_enabled: radio.trim() == "enabled",
        has_wifi: true,
        ..Default::default()
    };
    let out = run("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list"]).unwrap_or_default();
    let known = run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).unwrap_or_default();
    let (networks, wired) = parse_nmcli(&out, &known);
    st.networks = networks;
    st.wired = wired;
    Some(st)
}

/// Split one `nmcli -t` line on unescaped colons.
fn nmcli_fields(l: &str) -> Vec<String> {
    l.replace("\\:", "\u{1}").split(':').map(|s| s.replace('\u{1}', ":")).collect()
}

/// Networks from `nmcli -t dev wifi list` (one row per SSID, strongest access point) and the
/// active wired connection from `nmcli -t connection show`.
fn parse_nmcli(list: &str, known: &str) -> (Vec<Network>, Option<String>) {
    let mut nets: Vec<Network> = vec![];
    for l in list.lines() {
        let parts = nmcli_fields(l);
        if parts.len() < 4 || parts[1].is_empty() {
            continue;
        }
        let n = Network {
            active: parts[0] == "yes",
            ssid: parts[1].clone(),
            signal: parts[2].parse().unwrap_or(0),
            secure: !parts[3].is_empty() && parts[3] != "--",
            known: parts[0] == "yes",
        };
        match nets.iter_mut().find(|o| o.ssid == n.ssid) {
            Some(o) => {
                o.active |= n.active;
                o.known |= n.known;
                o.secure |= n.secure;
                o.signal = o.signal.max(n.signal);
            }
            None => nets.push(n),
        }
    }
    let mut wired = None;
    for l in known.lines() {
        let parts = nmcli_fields(l);
        let [name, ty] = parts.as_slice() else { continue };
        if ty.contains("wireless") {
            for n in nets.iter_mut().filter(|n| &n.ssid == name) {
                n.known = true;
            }
        } else if ty.contains("ethernet") && wired.is_none() {
            wired = Some(name.clone());
        }
    }
    (nets, wired)
}

pub fn probe() -> NetState {
    let bus = system_bus();
    let mut st = bus
        .as_ref()
        .and_then(probe_nm)
        .or_else(|| bus.as_ref().and_then(probe_iwd))
        .or_else(|| if crate::have("nmcli") { probe_nmcli() } else { None })
        .unwrap_or_default();
    st.networks.sort_by(|a, b| b.active.cmp(&a.active).then(b.signal.cmp(&a.signal)));
    st.networks.dedup_by(|a, b| a.ssid == b.ssid);
    st
}

pub fn set_wifi_enabled(on: bool) {
    crate::patch(|s| s.net.wifi_enabled = on);
    spawn_cmd(move || {
        let Some(c) = system_bus() else { return };
        if name_has_owner(&c, NM) {
            if let Some(nm) = nm_proxy(&c, "/org/freedesktop/NetworkManager", "org.freedesktop.DBus.Properties") {
                let _ = nm.call::<_, _, ()>("Set", &(NM, "WirelessEnabled", Value::from(on)));
            }
        } else if name_has_owner(&c, IWD) {
            if let Some(objs) = managed_objects(&c, IWD) {
                for (p, i) in objs {
                    if i.contains_key("net.connman.iwd.Device") {
                        if let Ok(pp) = Proxy::new(&c, IWD, p.as_str().to_string(), "org.freedesktop.DBus.Properties") {
                            let _ = pp.call::<_, _, ()>("Set", &("net.connman.iwd.Device", "Powered", Value::from(on)));
                        }
                    }
                }
            }
        } else {
            let _ = run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
        }
    });
}

/// Ask the backend to rescan access points.
pub fn scan() {
    spawn_cmd(|| {
        let Some(c) = system_bus() else { return };
        for d in nm_wifi_devices(&c) {
            if let Some(w) = nm_proxy(&c, d.as_str(), "org.freedesktop.NetworkManager.Device.Wireless") {
                let opts: HashMap<&str, Value> = HashMap::new();
                let _ = w.call::<_, _, ()>("RequestScan", &(opts,));
            }
        }
        if let Some(objs) = managed_objects(&c, IWD) {
            for (p, i) in objs {
                if i.contains_key("net.connman.iwd.Station") {
                    if let Ok(s) = Proxy::new(&c, IWD, p.as_str().to_string(), "net.connman.iwd.Station") {
                        let _ = s.call::<_, _, ()>("Scan", &());
                    }
                }
            }
        }
    });
}

/// Result of a connection attempt, delivered to `done`.
pub fn connect(ssid: String, password: Option<String>, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    spawn_cmd(move || {
        let r = connect_blocking(&ssid, password.as_deref());
        done(r);
    });
}

fn connect_blocking(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let c = system_bus();
    if let Some(c) = c.as_ref().filter(|c| name_has_owner(c, NM)) {
        let dev = nm_wifi_devices(c).into_iter().next().ok_or("no Wi-Fi device")?;
        let w = nm_proxy(c, dev.as_str(), "org.freedesktop.NetworkManager.Device.Wireless").ok_or("device")?;
        let aps: Vec<OwnedObjectPath> = w.call("GetAllAccessPoints", &()).map_err(|e| e.to_string())?;
        let ap = aps
            .into_iter()
            .find(|ap| {
                nm_proxy(c, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")
                    .and_then(|p| p.get_property::<Vec<u8>>("Ssid").ok())
                    .map(|s| s == ssid.as_bytes())
                    .unwrap_or(false)
            })
            .ok_or("network not found")?;
        let nm = nm_proxy(c, "/org/freedesktop/NetworkManager", NM).ok_or("nm")?;
        let mut conn: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
        if let Some(pw) = password {
            let mut sec = HashMap::new();
            sec.insert("key-mgmt", Value::from("wpa-psk"));
            sec.insert("psk", Value::from(pw.to_string()));
            conn.insert("802-11-wireless-security", sec);
        }
        nm.call::<_, _, (OwnedObjectPath, OwnedObjectPath)>("AddAndActivateConnection", &(conn, dev.clone(), ap))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(c) = c.as_ref().filter(|c| name_has_owner(c, IWD)) {
        let objs = managed_objects(c, IWD).ok_or("iwd")?;
        let (np, n) = objs
            .iter()
            .find_map(|(p, i)| {
                i.get("net.connman.iwd.Network").filter(|n| ov_str(n.get("Name")) == ssid).map(|n| (p.clone(), n))
            })
            .ok_or("network not found")?;
        if let (Some(pw), false) = (password, n.contains_key("KnownNetwork")) {
            let dev = objs
                .iter()
                .find_map(|(_, i)| i.get("net.connman.iwd.Device").map(|d| ov_str(d.get("Name"))))
                .unwrap_or_else(|| "wlan0".into());
            return run("iwctl", &["--passphrase", pw, "station", &dev, "connect", ssid])
                .map(|_| ())
                .ok_or_else(|| "iwctl failed".into());
        }
        let p = Proxy::new(c, IWD, np.as_str().to_string(), "net.connman.iwd.Network").map_err(|e| e.to_string())?;
        return p.call::<_, _, ()>("Connect", &()).map_err(|e| e.to_string());
    }
    let mut args = vec!["dev", "wifi", "connect", ssid];
    if let Some(pw) = password {
        args.extend(["password", pw]);
    }
    run("nmcli", &args).map(|_| ()).ok_or_else(|| "connection failed".into())
}

pub fn disconnect() {
    spawn_cmd(|| {
        if let Some(c) = system_bus() {
            for d in nm_wifi_devices(&c) {
                if let Some(p) = nm_proxy(&c, d.as_str(), "org.freedesktop.NetworkManager.Device") {
                    let _ = p.call::<_, _, ()>("Disconnect", &());
                    return;
                }
            }
            if let Some(objs) = managed_objects(&c, IWD) {
                for (p, i) in objs {
                    if i.contains_key("net.connman.iwd.Station") {
                        if let Ok(s) = Proxy::new(&c, IWD, p.as_str().to_string(), "net.connman.iwd.Station") {
                            let _ = s.call::<_, _, ()>("Disconnect", &());
                            return;
                        }
                    }
                }
            }
        }
        if let Some(st) = probe_nmcli() {
            if let Some(n) = st.active() {
                let _ = run("nmcli", &["connection", "down", &n.ssid]);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_parsing() {
        let list =
            "yes:Home:80:WPA2\nno:Home:40:WPA2\nno:Cafe\\:Free:55:--\nno::30:WPA2\nno:Office:20:WPA1 WPA2\nbad\n";
        let known = "Home:802-11-wireless\nOffice:802-11-wireless\nWired connection 1:802-3-ethernet\nlo:loopback\n";
        let (nets, wired) = parse_nmcli(list, known);
        let ssids: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(ssids, vec!["Home", "Cafe:Free", "Office"], "deduplicated, hidden SSIDs skipped");
        assert!(nets[0].active && nets[0].known && nets[0].secure);
        assert_eq!(nets[0].signal, 80);
        assert!(!nets[1].secure && !nets[1].known);
        assert!(nets[2].known && !nets[2].active);
        assert_eq!(wired.as_deref(), Some("Wired connection 1"));
    }

    #[test]
    fn nmcli_field_escapes() {
        assert_eq!(nmcli_fields("a\\:b:c"), vec!["a:b", "c"]);
        assert_eq!(nmcli_fields(""), vec![""]);
    }
}
