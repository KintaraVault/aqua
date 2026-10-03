//! Input configuration: XKB keymap with several layouts + switching shortcut,
//! libinput device settings (tap-to-click, natural scrolling, speed, DWT …) and
//! user-defined key bindings from `[[bindings]]` in aqua.toml.
use crate::state::Aqua;
use aqua_config::{Config, PointerCfg};
use smithay::input::keyboard::{xkb, Keysym, ModifiersState, XkbConfig};

#[derive(Clone, Debug, PartialEq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    /// Lower-case keysym (None = modifier-only chord such as "alt+shift").
    pub sym: Option<Keysym>,
}

/// Split "ctrl+shift+a" into lower-case parts; a trailing "+" after a separator is the
/// plus key itself ("ctrl++", "+").
fn split_chord(s: &str) -> Vec<String> {
    let s = s.trim();
    let (body, plus) = match s.strip_suffix('+') {
        Some(rest) if rest.is_empty() || rest.trim_end().ends_with('+') => {
            (rest.trim_end().trim_end_matches('+'), true)
        }
        _ => (s, false),
    };
    let mut parts: Vec<String> = body.split('+').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
    if plus {
        parts.push("plus".into());
    }
    parts
}

impl Chord {
    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord { ctrl: false, alt: false, shift: false, logo: false, sym: None };
        for part in split_chord(s) {
            match part.as_str() {
                "ctrl" | "control" | "ctl" => c.ctrl = true,
                "alt" | "opt" | "option" | "mod1" => c.alt = true,
                "shift" => c.shift = true,
                "super" | "logo" | "cmd" | "command" | "win" | "mod4" => c.logo = true,
                "" => {}
                k => {
                    let name = match k {
                        "space" => "space",
                        "enter" | "return" => "Return",
                        "esc" | "escape" => "Escape",
                        "tab" => "Tab",
                        "plus" | "equal" => "equal",
                        "minus" => "minus",
                        "backspace" => "BackSpace",
                        "delete" | "del" => "Delete",
                        "left" => "Left",
                        "right" => "Right",
                        "up" => "Up",
                        "down" => "Down",
                        "print" => "Print",
                        "caps" | "capslock" => "Caps_Lock",
                        other => other,
                    };
                    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
                    let sym =
                        if sym.raw() == 0 { xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE) } else { sym };
                    if sym.raw() == 0 {
                        return None;
                    }
                    c.sym = Some(sym);
                }
            }
        }
        Some(c)
    }

    /// Matches either the keysym of the active layout or the layout-independent
    /// (Latin / US-position) keysym of the same physical key, so ⌘Q, ⌘W, ⌘Space … keep
    /// working with Russian, Greek, Arabic … layouts active.
    pub fn matches_key(&self, m: &ModifiersState, key: &KeySyms) -> bool {
        self.matches(m, key.latin) || (key.current != key.latin && self.matches(m, key.current))
    }

    pub fn matches(&self, m: &ModifiersState, sym: Keysym) -> bool {
        let lower =
            Keysym::new(xkb::keysym_from_name(&xkb::keysym_get_name(sym).to_lowercase(), xkb::KEYSYM_NO_FLAGS).raw());
        self.ctrl == m.ctrl
            && self.alt == m.alt
            && self.shift == m.shift
            && self.logo == m.logo
            && self.sym.map(|s| s == lower || s == sym).unwrap_or(false)
    }
}

/// The keysyms of one pressed key: the active layout's unshifted sym and a
/// layout-independent Latin sym (used for shortcuts).
#[derive(Clone, Copy, Debug)]
pub struct KeySyms {
    pub current: Keysym,
    pub latin: Keysym,
}

thread_local! {
    /// A plain US keymap: last-resort source of Latin keysyms when no configured
    /// layout has a Latin letter on the key (e.g. only "ru" configured).
    static US_KEYMAP: Option<xkb::Keymap> = {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
    };
}

/// Layout-independent keysym for a key: a Latin keysym from any configured layout,
/// else the key's symbol in a US layout, else the current keysym.
pub fn latin_sym(h: &smithay::input::keyboard::KeysymHandle<'_>) -> Keysym {
    let current = h.raw_syms().first().copied().unwrap_or_else(|| h.modified_sym());
    let ascii = |k: Keysym| k.key_char().map(|c| c.is_ascii()).unwrap_or(true);
    if ascii(current) {
        return current;
    }
    if let Some(k) = h.raw_latin_sym_or_raw_current_sym().filter(|k| ascii(*k)) {
        return k;
    }
    let code = h.raw_code();
    US_KEYMAP
        .with(|km| km.as_ref().and_then(|km| km.key_get_syms_by_level(code, 0, 0).first().copied()))
        .filter(|k| ascii(*k))
        .unwrap_or(current)
}

pub struct InputCfg {
    pub layouts: Vec<String>,
    layout_s: String,
    variant_s: String,
    model: String,
    options: String,
    pub switch: String,
    pub switch_chord: Option<Chord>,
    pub repeat_delay: i32,
    pub repeat_rate: i32,
    pub numlock: bool,
    pub pointer: PointerCfg,
    pub bindings: Vec<(Chord, String)>,
    /// Remappable system shortcuts (defaults + `[shortcuts]` overrides).
    pub system: Vec<(Chord, &'static str)>,
}

impl InputCfg {
    pub fn from_config(cfg: &Config) -> Self {
        let k = &cfg.keyboard;
        let mut layouts: Vec<String> = k.layouts.iter().filter(|l| !l.trim().is_empty()).cloned().collect();
        if layouts.is_empty() {
            layouts.push("us".into());
        }
        let mut options = k.options.clone();
        if k.switch == "caps" && !options.contains("grp:") {
            if !options.is_empty() {
                options.push(',');
            }
            options.push_str("grp:caps_toggle");
        }
        let bindings = cfg
            .bindings
            .iter()
            .filter_map(|b| match Chord::parse(&b.keys) {
                Some(c) => Some((c, b.action.clone())),
                None => {
                    tracing::warn!("invalid key binding {:?}", b.keys);
                    None
                }
            })
            .collect();
        let mut system = vec![];
        for sc in aqua_config::shortcuts::SYSTEM {
            for c in aqua_config::shortcuts::chords(cfg, sc.id) {
                match Chord::parse(&c) {
                    Some(ch) => system.push((ch, sc.id)),
                    None => tracing::warn!("invalid shortcut {c:?} for {}", sc.id),
                }
            }
        }
        Self {
            layout_s: layouts.join(","),
            variant_s: k.variants.join(","),
            layouts,
            model: k.model.clone(),
            options,
            switch: k.switch.clone(),
            switch_chord: if k.switch == "caps" || k.switch.is_empty() { None } else { Chord::parse(&k.switch) },
            repeat_delay: k.repeat_delay.clamp(100, 2000),
            repeat_rate: k.repeat_rate.clamp(1, 100),
            numlock: k.numlock,
            pointer: cfg.pointer.clone(),
            bindings,
            system,
        }
    }

    /// System shortcut id for a key press. "switch-apps" also matches with Shift
    /// held (cycles backwards).
    pub fn system_match(&self, m: &ModifiersState, key: &KeySyms) -> Option<&'static str> {
        let tab = |s: Keysym| {
            if s.raw() == smithay::input::keyboard::keysyms::KEY_ISO_Left_Tab {
                Keysym::new(smithay::input::keyboard::keysyms::KEY_Tab)
            } else {
                s
            }
        };
        let key = &KeySyms { current: tab(key.current), latin: tab(key.latin) };
        for (c, id) in &self.system {
            if c.matches_key(m, key) {
                return Some(id);
            }
            if *id == "switch-apps" && m.shift && !c.shift {
                let mut m2 = *m;
                m2.shift = false;
                if c.matches_key(&m2, key) {
                    return Some(id);
                }
            }
        }
        None
    }

    pub fn xkb(&self) -> XkbConfig<'_> {
        XkbConfig {
            rules: "",
            model: &self.model,
            layout: &self.layout_s,
            variant: &self.variant_s,
            options: if self.options.is_empty() { None } else { Some(self.options.clone()) },
        }
    }

    /// Does the keymap differ from `other` (needs recompilation)?
    pub fn keymap_differs(&self, other: &InputCfg) -> bool {
        self.layout_s != other.layout_s
            || self.variant_s != other.variant_s
            || self.model != other.model
            || self.options != other.options
    }
}

/// Apply libinput settings to one device.
pub fn configure_device(dev: &mut smithay::reexports::input::Device, p: &PointerCfg) {
    use smithay::reexports::input::{AccelProfile, ClickMethod, DeviceCapability};
    let touchpad = dev.config_tap_finger_count() > 0;
    if dev.has_capability(DeviceCapability::Pointer) {
        let natural = if touchpad { p.natural_scroll } else { p.mouse_natural_scroll };
        if dev.config_scroll_has_natural_scroll() {
            let _ = dev.config_scroll_set_natural_scroll_enabled(natural);
        }
        if dev.config_accel_is_available() {
            let speed = if touchpad { p.speed } else { p.mouse_speed };
            let _ = dev.config_accel_set_speed(speed.clamp(-1.0, 1.0));
            let prof = if p.accel_profile == "flat" { AccelProfile::Flat } else { AccelProfile::Adaptive };
            let _ = dev.config_accel_set_profile(prof);
        }
    }
    if touchpad {
        let _ = dev.config_tap_set_enabled(p.tap_to_click);
        let _ = dev.config_dwt_set_enabled(p.disable_while_typing);
        let method = if p.secondary_click == "corner" { ClickMethod::ButtonAreas } else { ClickMethod::Clickfinger };
        let _ = dev.config_click_set_method(method);
    }
    tracing::info!("configured input device {:?} (touchpad: {touchpad})", dev.name());
}

impl Aqua {
    /// Number of layouts and the active one.
    pub fn active_layout(&mut self) -> usize {
        let Some(kb) = self.seat.get_keyboard() else { return 0 };
        kb.with_xkb_state(self, |ctx| ctx.xkb().lock().unwrap().active_layout().0 as usize)
    }

    pub fn set_layout(&mut self, idx: usize) {
        let Some(kb) = self.seat.get_keyboard() else { return };
        let n = self.input_cfg.layouts.len().max(1);
        let idx = idx % n;
        kb.with_xkb_state(self, |mut ctx| ctx.set_layout(smithay::input::keyboard::Layout(idx as u32)));
        self.after_layout_change();
    }

    pub fn next_layout(&mut self) {
        let cur = self.active_layout();
        self.set_layout(cur + 1);
    }

    /// Sync the shell (menu bar badge, lock screen, keyboard viewer) with the keymap.
    pub fn after_layout_change(&mut self) {
        let idx = self.active_layout();
        let changed = self.shell.layout_idx != idx;
        let variants: Vec<String> = self.cfg.keyboard.variants.clone();
        let codes: Vec<String> = self
            .input_cfg
            .layouts
            .iter()
            .enumerate()
            .map(|(i, l)| match variants.get(i).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                Some(v) => format!("{l}({v})"),
                None => l.clone(),
            })
            .collect();
        self.shell.layouts = codes.clone();
        self.shell.layout_idx = idx;
        self.shell.key_labels = self.key_labels();
        if changed {
            let name = aqua_shell::sysinfo::layout_name(codes.get(idx).map(|s| s.as_str()).unwrap_or("us"));
            self.shell.show_hud(aqua_shell::alert::HudKind::Layout, 0.0, name);
        }
        self.needs_redraw = true;
    }

    /// Characters produced by each evdev key in the active layout: `code` → base
    /// level, `code + 1000` → Shift level (Keyboard Viewer).
    pub fn key_labels(&mut self) -> std::collections::HashMap<u32, String> {
        let Some(kb) = self.seat.get_keyboard() else { return Default::default() };
        kb.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let layout = xkb.active_layout();
            // SAFETY: the keymap reference does not outlive the lock
            let keymap = unsafe { xkb.keymap() };
            let mut m = std::collections::HashMap::new();
            for code in 2u32..=58 {
                for level in 0..2u32 {
                    let syms = keymap.key_get_syms_by_level(xkb::Keycode::new(code + 8), layout.0, level);
                    if let Some(l) = syms.first().and_then(|s| keysym_label(*s)) {
                        m.insert(code + level * 1000, l);
                    }
                }
            }
            m
        })
    }

    /// Re-read input settings (config reload).
    pub fn apply_input_config(&mut self) {
        let new = InputCfg::from_config(&self.cfg);
        let keymap = new.keymap_differs(&self.input_cfg);
        let repeat = (new.repeat_delay, new.repeat_rate) != (self.input_cfg.repeat_delay, self.input_cfg.repeat_rate);
        self.input_cfg = new;
        if let Some(kb) = self.seat.get_keyboard() {
            if keymap {
                let xkb = self.input_cfg.xkb();
                let owned = XkbConfig {
                    rules: "",
                    model: xkb.model,
                    layout: xkb.layout,
                    variant: xkb.variant,
                    options: xkb.options.clone(),
                }
                .options;
                let (model, layout, variant) =
                    (self.input_cfg.model.clone(), self.input_cfg.layout_s.clone(), self.input_cfg.variant_s.clone());
                let conf = XkbConfig { rules: "", model: &model, layout: &layout, variant: &variant, options: owned };
                if let Err(e) = kb.set_xkb_config(self, conf) {
                    tracing::warn!("invalid keyboard config: {e:?}");
                }
            }
            if repeat {
                kb.change_repeat_info(self.input_cfg.repeat_rate, self.input_cfg.repeat_delay);
            }
        }
        if let Some(ud) = self.udev.as_mut() {
            for d in ud.input_devices.iter_mut() {
                configure_device(d, &self.input_cfg.pointer);
            }
        }
        self.after_layout_change();
    }
}

/// Printable label of a keysym: its character, or the spacing form of a dead key
/// (dead keys have no character, so their keys were blank in the Keyboard Viewer).
fn keysym_label(s: Keysym) -> Option<String> {
    if let Some(c) = s.key_char() {
        if !c.is_control() {
            return Some(c.to_string());
        }
    }
    let name = xkb::keysym_get_name(s);
    let c = match name.strip_prefix("dead_")? {
        "grave" => '`',
        "acute" => '´',
        "circumflex" => '^',
        "tilde" => '~',
        "macron" => '¯',
        "breve" => '˘',
        "abovedot" => '˙',
        "diaeresis" => '¨',
        "abovering" => '˚',
        "doubleacute" => '˝',
        "caron" => 'ˇ',
        "cedilla" => '¸',
        "ogonek" => '˛',
        "iota" => 'ͺ',
        "belowdot" => '.',
        "hook" => '̉',
        "horn" => '̛',
        "stroke" => '/',
        "currency" => '¤',
        "greek" => 'µ',
        _ => return None,
    };
    Some(c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, alt: bool, shift: bool, logo: bool) -> ModifiersState {
        ModifiersState { ctrl, alt, shift, logo, ..Default::default() }
    }

    fn sym(name: &str) -> Keysym {
        xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS)
    }

    #[test]
    fn parse_chords() {
        let c = Chord::parse("Super+Shift+3").unwrap();
        assert!(c.logo && c.shift && !c.ctrl && !c.alt);
        assert_eq!(c.sym, Some(sym("3")));
        assert_eq!(Chord::parse("ctrl+space").unwrap().sym, Some(sym("space")));
        assert_eq!(Chord::parse("cmd+Return").unwrap().sym, Some(sym("Return")));
        assert_eq!(Chord::parse("alt+shift").unwrap().sym, None, "modifier-only chord");
        assert_eq!(Chord::parse("super+F11").unwrap().sym, Some(sym("F11")));
        assert!(Chord::parse("super+notakey").is_none());
    }

    #[test]
    fn plus_key_chords() {
        assert_eq!(split_chord("ctrl++"), vec!["ctrl", "plus"]);
        assert_eq!(split_chord("Ctrl + Shift + +"), vec!["ctrl", "shift", "plus"]);
        assert_eq!(split_chord("+"), vec!["plus"]);
        assert_eq!(split_chord("ctrl+a"), vec!["ctrl", "a"]);
        assert_eq!(split_chord("ctrl+"), vec!["ctrl"]);
        let c = Chord::parse("ctrl++").unwrap();
        assert!(c.ctrl && !c.shift);
        assert_eq!(c.sym, Chord::parse("ctrl+plus").unwrap().sym);
        assert!(c.sym.is_some());
    }

    #[test]
    fn matching_ignores_case_but_not_modifiers() {
        let c = Chord::parse("super+q").unwrap();
        assert!(c.matches(&mods(false, false, false, true), sym("q")));
        assert!(c.matches(&mods(false, false, false, true), sym("Q")));
        assert!(!c.matches(&mods(false, false, true, true), sym("q")));
        assert!(!c.matches(&mods(false, false, false, false), sym("q")));
        assert!(!Chord::parse("alt+shift").unwrap().matches(&mods(false, true, true, false), sym("a")));
    }

    #[test]
    fn matches_latin_key_on_other_layouts() {
        let c = Chord::parse("super+q").unwrap();
        let keys = KeySyms { current: sym("Cyrillic_shorti"), latin: sym("q") };
        assert!(c.matches_key(&mods(false, false, false, true), &keys));
        let other = KeySyms { current: sym("Cyrillic_shorti"), latin: sym("w") };
        assert!(!c.matches_key(&mods(false, false, false, true), &other));
    }
}
