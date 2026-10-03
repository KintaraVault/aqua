//! Displays pane: outputs, EDID names and modes.
use super::*;

/// Output list in the compositor's format (`name<TAB>WxH<TAB>scale<TAB>label<TAB>Hz<TAB>modes`):
/// published by Aqua in `$XDG_RUNTIME_DIR/aqua-outputs`; when that is missing (another
/// compositor, older session) the connected monitors are read from DRM sysfs + EDID.
pub fn outputs_text() -> String {
    let mut text = std::fs::read_to_string(aqua_config::paths::runtime_dir().join("aqua-outputs")).unwrap_or_default();
    if text.trim().is_empty() {
        text = std::fs::read_to_string("/tmp/aqua-outputs").unwrap_or_default();
    }
    if text.trim().is_empty() {
        text = sysfs_outputs();
    }
    text
}

/// Monitor name from an EDID blob (descriptor 0xFC), else manufacturer id.
pub fn edid_name(e: &[u8]) -> Option<String> {
    if e.len() < 128 {
        return None;
    }
    for i in 0..4 {
        let d = &e[54 + i * 18..72 + i * 18];
        if d[0] == 0 && d[1] == 0 && d[3] == 0xFC {
            let s: String = d[5..18].iter().take_while(|b| **b != 0x0a).map(|b| *b as char).collect();
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    let m = u16::from_be_bytes([e[8], e[9]]);
    let c = |v: u16| (b'A' - 1 + (v & 0x1f) as u8) as char;
    Some(trf("{vendor} display", &[("vendor", &format!("{}{}{}", c(m >> 10), c(m >> 5), c(m)))]))
}

/// All modes a monitor advertises in its EDID: established, standard and detailed
/// timings of the base block plus CTA-861 detailed timings and video codes.
pub fn edid_modes(e: &[u8]) -> Vec<(u32, u32, f64)> {
    let mut v: Vec<(u32, u32, f64)> = vec![];
    if e.len() < 128 || e[0..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return v;
    }
    let dtd = |d: &[u8]| -> Option<(u32, u32, f64)> {
        let clk = u16::from_le_bytes([d[0], d[1]]) as f64 * 10_000.0;
        if clk == 0.0 {
            return None;
        }
        let ha = d[2] as u32 | ((d[4] as u32 & 0xf0) << 4);
        let hb = d[3] as u32 | ((d[4] as u32 & 0x0f) << 8);
        let va = d[5] as u32 | ((d[7] as u32 & 0xf0) << 4);
        let vb = d[6] as u32 | ((d[7] as u32 & 0x0f) << 8);
        let interlaced = d[17] & 0x80 != 0;
        let tot = ((ha + hb) * (va + vb)) as f64;
        if ha == 0 || va == 0 || tot == 0.0 {
            return None;
        }
        let mut hz = clk / tot;
        let mut va = va;
        if interlaced {
            va *= 2;
            hz *= 2.0;
        }
        Some((ha, va, (hz * 100.0).round() / 100.0))
    };
    const EST: [(u32, u32, f64); 17] = [
        (720, 400, 70.0),
        (720, 400, 88.0),
        (640, 480, 60.0),
        (640, 480, 67.0),
        (640, 480, 72.0),
        (640, 480, 75.0),
        (800, 600, 56.0),
        (800, 600, 60.0),
        (800, 600, 72.0),
        (800, 600, 75.0),
        (832, 624, 75.0),
        (1024, 768, 87.0),
        (1024, 768, 60.0),
        (1024, 768, 70.0),
        (1024, 768, 75.0),
        (1280, 1024, 75.0),
        (1152, 870, 75.0),
    ];
    let bits = (e[35] as u32) << 9 | (e[36] as u32) << 1 | (e[37] as u32 >> 7);
    for (i, m) in EST.iter().enumerate() {
        if bits & (1 << (16 - i)) != 0 && m.0 >= 800 && m.2 != 87.0 {
            v.push(*m);
        }
    }
    for i in 0..8 {
        let (b0, b1) = (e[38 + i * 2], e[39 + i * 2]);
        if b0 == 1 && b1 == 1 || b0 == 0 {
            continue;
        }
        let w = (b0 as u32 + 31) * 8;
        let h = match b1 >> 6 {
            0 => w * 10 / 16,
            1 => w * 3 / 4,
            2 => w * 4 / 5,
            _ => w * 9 / 16,
        };
        v.push((w, h, (b1 & 0x3f) as f64 + 60.0));
    }
    for i in 0..4 {
        if let Some(m) = dtd(&e[54 + i * 18..72 + i * 18]) {
            v.push(m);
        }
    }
    let n = e[126] as usize;
    for k in 1..=n {
        let b = match e.get(k * 128..k * 128 + 128) {
            Some(b) => b,
            None => break,
        };
        if b[0] != 0x02 {
            continue;
        }
        let d = b[2] as usize;
        let mut i = 4;
        while i < d.min(127) {
            let tag = b[i] >> 5;
            let len = (b[i] & 0x1f) as usize;
            if tag == 2 {
                for &svd in &b[i + 1..(i + 1 + len).min(127)] {
                    let vic = if svd & 0x7f <= 64 { svd & 0x7f } else { svd };
                    let m = match vic {
                        4 => Some((1280, 720, 60.0)),
                        16 => Some((1920, 1080, 60.0)),
                        19 => Some((1280, 720, 50.0)),
                        31 => Some((1920, 1080, 50.0)),
                        32 => Some((1920, 1080, 24.0)),
                        33 => Some((1920, 1080, 25.0)),
                        34 => Some((1920, 1080, 30.0)),
                        63 => Some((1920, 1080, 120.0)),
                        64 => Some((1920, 1080, 100.0)),
                        93 => Some((3840, 2160, 24.0)),
                        94 => Some((3840, 2160, 25.0)),
                        95 => Some((3840, 2160, 30.0)),
                        96 => Some((3840, 2160, 50.0)),
                        97 => Some((3840, 2160, 60.0)),
                        117 => Some((3840, 2160, 100.0)),
                        118 => Some((3840, 2160, 120.0)),
                        _ => None,
                    };
                    if let Some(m) = m {
                        v.push(m);
                    }
                }
            }
            i += len + 1;
        }
        let mut o = d;
        while d >= 4 && o + 18 <= 127 {
            match dtd(&b[o..o + 18]) {
                Some(m) => v.push(m),
                None => break,
            }
            o += 18;
        }
    }
    v
}

/// Connected monitors read straight from DRM sysfs (`modes` + EDID), used when the
/// compositor has not published its output list (or published no mode list).
pub fn sysfs_outputs() -> String {
    let mut out = String::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return out };
    let mut dirs: Vec<std::path::PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.join("status").exists()).collect();
    dirs.sort();
    for p in dirs {
        if std::fs::read_to_string(p.join("status")).map(|s| s.trim() != "connected").unwrap_or(true) {
            continue;
        }
        let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let name = file.split_once('-').map(|(_, n)| n.to_string()).unwrap_or(file);
        let sys_modes: Vec<String> = std::fs::read_to_string(p.join("modes"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().trim_end_matches('i').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let edid = std::fs::read(p.join("edid")).unwrap_or_default();
        let em = edid_modes(&edid);
        let cur = sys_modes
            .first()
            .cloned()
            .or_else(|| em.iter().max_by_key(|m| m.0 * m.1).map(|m| format!("{}x{}", m.0, m.1)))
            .unwrap_or_default();
        let mut modes: Vec<(String, f64)> = vec![];
        for (w, h, hz) in &em {
            modes.push((format!("{w}x{h}"), *hz));
        }
        for r in &sys_modes {
            if !modes.iter().any(|(m, _)| m == r) {
                modes.push((r.clone(), 60.0));
            }
        }
        modes.sort_by(|a, b| {
            let px = |s: &str| {
                s.split_once('x')
                    .map(|(w, h)| w.parse::<u32>().unwrap_or(0) * h.parse::<u32>().unwrap_or(0))
                    .unwrap_or(0)
            };
            px(&b.0).cmp(&px(&a.0)).then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        modes.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 0.5);
        let hz = modes.iter().find(|(m, _)| *m == cur).map(|m| m.1).unwrap_or(0.0);
        let label = edid_name(&edid).unwrap_or_else(|| {
            if name.starts_with("eDP") || name.starts_with("LVDS") {
                tr("Built-in Display").into()
            } else {
                tr("Display").into()
            }
        });
        let list: Vec<String> = modes.iter().map(|(m, h)| format!("{m}@{}", fmt_hz(*h))).collect();
        out.push_str(&format!(
            "{name}\t{cur}\t1\t{label}\t{}\t{}\n",
            if hz > 0.0 { fmt_hz(hz) } else { String::new() },
            list.join(",")
        ));
    }
    out
}

/// Fill in missing mode lists from sysfs (matching connector names) and, as a last
/// resort, the current mode — so Resolution / Refresh rate are always shown.
pub fn complete_modes(name: &str, size: &str, hz: f64, modes: &mut Vec<(String, f64)>) {
    if modes.is_empty() {
        let sys = sysfs_outputs();
        for l in sys.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.first() == Some(&name) {
                *modes = parse_modes(f.get(5).copied());
            }
        }
    }
    if !size.is_empty() && !modes.iter().any(|(m, _)| m == size) {
        modes.insert(0, (size.to_string(), if hz > 0.0 { hz } else { 60.0 }));
    } else if hz > 0.0 && !modes.iter().any(|(m, h)| m == size && (h - hz).abs() < 0.05) {
        modes.push((size.to_string(), hz));
    }
}

pub fn parse_modes(f: Option<&str>) -> Vec<(String, f64)> {
    f.map(|m| {
        m.split(',')
            .filter_map(|x| x.split_once('@').and_then(|(r, h)| Some((r.to_string(), h.parse::<f64>().ok()?))))
            .collect()
    })
    .unwrap_or_default()
}

/// Live outputs published by the compositor (`$XDG_RUNTIME_DIR/aqua-outputs`), merged with config.
pub fn outputs(cfg: &Config) -> Vec<OutputInfo> {
    let text = outputs_text();
    let primary_cfg = cfg.outputs.iter().find(|o| o.primary).map(|o| o.name.clone());
    let mut v: Vec<OutputInfo> = text
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let f: Vec<&str> = l.split('\t').collect();
            let name = f.first()?.to_string();
            let oc = cfg.outputs.iter().find(|o| o.name == name);
            let scale = oc.map(|o| o.scale).unwrap_or(0.0);
            let live: f64 = f.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let idx = SCALES.iter().position(|s| (*s - scale).abs() < 0.01).unwrap_or(0) as i32;
            let label = f.get(3).map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or(tr("Display")).to_string();
            let size = f.get(1).copied().unwrap_or("").to_string();
            let hz: f64 = f.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let mut modes = parse_modes(f.get(5).copied());
            complete_modes(&name, &size, hz, &mut modes);
            let (resolutions, res_idx, rates, rate_idx) = mode_lists(&modes, &size, hz);
            Some(OutputInfo {
                label: label.into(),
                mode: if hz > 0.0 {
                    format!("{size} @ {} Hz · {:.0}%", fmt_hz(hz), live * 100.0)
                } else {
                    format!("{size} @ {:.0}%", live * 100.0)
                }
                .into(),
                scale_idx: idx,
                enabled: oc.map(|o| o.enabled).unwrap_or(true),
                primary: primary_cfg.as_deref().map(|p| p == name).unwrap_or(i == 0),
                can_modeset: !modes.is_empty(),
                resolutions: ModelRc::new(VecModel::from(
                    resolutions.into_iter().map(SharedString::from).collect::<Vec<_>>(),
                )),
                res_idx,
                rates: ModelRc::new(VecModel::from(
                    rates.into_iter().map(|r| SharedString::from(format!("{} Hz", fmt_hz(r)))).collect::<Vec<_>>(),
                )),
                rate_idx,
                vrr: oc.map(|o| o.vrr).unwrap_or(false),
                name: name.into(),
            })
        })
        .collect();
    for o in &cfg.outputs {
        if o.name != "*" && !o.enabled && !v.iter().any(|x| x.name == o.name.as_str()) {
            v.push(OutputInfo {
                name: o.name.clone().into(),
                label: tr("Disabled display").into(),
                mode: "off".into(),
                scale_idx: 0,
                enabled: false,
                primary: false,
                ..Default::default()
            });
        }
    }
    v
}

pub fn fmt_hz(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.005 {
        format!("{}", hz.round() as i64)
    } else {
        format!("{hz:.2}")
    }
}

/// Modes published by the compositor ("WxH", Hz) → (resolutions, current resolution index,
/// refresh rates of the current resolution, current rate index).
pub fn mode_lists(modes: &[(String, f64)], size: &str, hz: f64) -> (Vec<String>, i32, Vec<f64>, i32) {
    let mut res: Vec<String> = vec![];
    for (r, _) in modes {
        if !res.contains(r) {
            res.push(r.clone());
        }
    }
    let res_idx = res.iter().position(|r| r == size).map(|i| i as i32).unwrap_or(-1);
    let rates = rates_for(modes, size);
    let rate_idx = rates
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - hz).abs().partial_cmp(&(b.1 - hz).abs()).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i as i32)
        .unwrap_or(-1);
    (res, res_idx, rates, rate_idx)
}

pub fn rates_for(modes: &[(String, f64)], size: &str) -> Vec<f64> {
    let mut v: Vec<f64> = modes.iter().filter(|(r, _)| r == size).map(|(_, h)| *h).collect();
    v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup_by(|a, b| (*a - *b).abs() < 0.005);
    v
}

/// Raw mode list of an output from `$XDG_RUNTIME_DIR/aqua-outputs`: (modes, current size, current Hz).
pub fn live_modes(name: &str) -> (Vec<(String, f64)>, String, f64) {
    let text = outputs_text();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.first() == Some(&name) {
            let size = f.get(1).unwrap_or(&"").to_string();
            let hz = f.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let mut modes = parse_modes(f.get(5).copied());
            complete_modes(name, &size, hz, &mut modes);
            return (modes, size, hz);
        }
    }
    (vec![], String::new(), 0.0)
}

/// Wire the displays pane callbacks.
pub fn wire_displays(ui: &SettingsWindow, cfg: &Rc<RefCell<Config>>, walls: &Rc<Vec<WallItem>>) -> Rc<dyn Fn()> {
    let s = ui.global::<S>();
    let refresh_outputs = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        move || {
            let ui = ui.unwrap();
            ui.global::<S>().set_outputs(ModelRc::new(VecModel::from(outputs(&cfg.borrow()))));
        }
    };
    s.on_set_scale({
        let cfg = cfg.clone();
        let r = refresh_outputs.clone();
        move |name, i| {
            output_cfg(&mut cfg.borrow_mut(), &name).scale = SCALES[i.clamp(0, 5) as usize];
            let _ = cfg.borrow().save();
            r();
        }
    });
    s.on_set_output_enabled({
        let cfg = cfg.clone();
        let r = refresh_outputs.clone();
        move |name, on| {
            let live = outputs(&cfg.borrow()).iter().filter(|o| o.enabled).count();
            if !on && live <= 1 {
                return;
            }
            output_cfg(&mut cfg.borrow_mut(), &name).enabled = on;
            let _ = cfg.borrow().save();
            r();
        }
    });
    s.on_set_primary({
        let cfg = cfg.clone();
        let r = refresh_outputs.clone();
        move |name| {
            {
                let mut c = cfg.borrow_mut();
                for o in c.outputs.iter_mut() {
                    o.primary = false;
                }
                output_cfg(&mut c, &name).primary = true;
                let _ = c.save();
            }
            r();
        }
    });
    s.on_set_wallpaper({
        let cfg = cfg.clone();
        let walls = walls.clone();
        move |i| {
            let p = walls.get(i as usize).map(|w| w.path.to_string()).unwrap_or_default();
            cfg.borrow_mut().wallpaper = if p.is_empty() { None } else { Some(p.into()) };
            let _ = cfg.borrow().save();
        }
    });

    let prev_mode: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    let revert_timer = Rc::new(slint::Timer::default());
    let apply_mode = {
        let cfg = cfg.clone();
        let ui = ui.as_weak();
        let prev = prev_mode.clone();
        let timer = revert_timer.clone();
        let r = refresh_outputs.clone();
        move |name: String, mode: String| {
            let old = {
                let mut c = cfg.borrow_mut();
                let oc = output_cfg(&mut c, &name);
                let old = oc.mode.clone();
                oc.mode = mode.clone();
                old
            };
            if let Err(e) = cfg.borrow().save() {
                ui.unwrap().global::<S>().set_display_status(trf("Could not save: {e}", &[("e", &e)]).into());
                return;
            }
            if prev.borrow().is_none() {
                *prev.borrow_mut() = Some((name.clone(), old));
            }
            let u = ui.unwrap();
            u.global::<S>().set_mode_pending(true);
            u.global::<S>().set_display_status(
                format!(
                    "{name} switched to {}. Reverting in 15 seconds unless you keep it.",
                    mode.replace('@', " @ ") + " Hz"
                )
                .into(),
            );
            let (cfg2, ui2, prev2, r2) = (cfg.clone(), ui.clone(), prev.clone(), r.clone());
            timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_secs(15), move || {
                if let Some((n, m)) = prev2.borrow_mut().take() {
                    output_cfg(&mut cfg2.borrow_mut(), &n).mode = m;
                    let _ = cfg2.borrow().save();
                }
                if let Some(u) = ui2.upgrade() {
                    u.global::<S>().set_mode_pending(false);
                    u.global::<S>().set_display_status(tr("Previous display settings restored.").into());
                }
                r2();
            });
            r();
        }
    };
    s.on_set_resolution({
        let apply = apply_mode.clone();
        move |name, i| {
            let (modes, _cur, hz) = live_modes(&name);
            let mut res: Vec<String> = vec![];
            for (r, _) in &modes {
                if !res.contains(r) {
                    res.push(r.clone());
                }
            }
            let Some(r) = res.get(i.max(0) as usize).cloned() else { return };
            let rates = rates_for(&modes, &r);
            let rate = rates.iter().copied().find(|x| (x - hz).abs() < 0.6).or(rates.first().copied()).unwrap_or(60.0);
            apply(name.to_string(), format!("{r}@{}", fmt_hz(rate)));
        }
    });
    s.on_set_rate({
        let apply = apply_mode.clone();
        move |name, i| {
            let (modes, cur, _) = live_modes(&name);
            let rates = rates_for(&modes, &cur);
            let Some(rate) = rates.get(i.max(0) as usize) else { return };
            apply(name.to_string(), format!("{cur}@{}", fmt_hz(*rate)));
        }
    });
    s.on_keep_mode({
        let ui = ui.as_weak();
        let prev = prev_mode.clone();
        let timer = revert_timer.clone();
        move || {
            timer.stop();
            prev.borrow_mut().take();
            let u = ui.unwrap();
            u.global::<S>().set_mode_pending(false);
            u.global::<S>().set_display_status("".into());
        }
    });
    s.on_revert_mode({
        let ui = ui.as_weak();
        let prev = prev_mode.clone();
        let timer = revert_timer.clone();
        let cfg = cfg.clone();
        let r = refresh_outputs.clone();
        move || {
            timer.stop();
            if let Some((n, m)) = prev.borrow_mut().take() {
                output_cfg(&mut cfg.borrow_mut(), &n).mode = m;
                let _ = cfg.borrow().save();
            }
            let u = ui.unwrap();
            u.global::<S>().set_mode_pending(false);
            u.global::<S>().set_display_status("".into());
            r();
        }
    });
    s.on_set_vrr({
        let cfg = cfg.clone();
        let r = refresh_outputs.clone();
        move |name, on| {
            output_cfg(&mut cfg.borrow_mut(), &name).vrr = on;
            let _ = cfg.borrow().save();
            r();
        }
    });
    Rc::new(refresh_outputs)
}
