//! ⌘Tab application switcher: glass strip of running app icons.
use crate::{hash_of, style, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};
use aqua_icons::IconRequest;

#[derive(Default)]
pub struct Switcher {
    pub open: bool,
    /// App ids, most recently used first.
    pub apps: Vec<String>,
    pub sel: usize,
    pub t: f32,
}

impl Switcher {
    pub fn start(&mut self, apps: Vec<String>, back: bool) {
        if apps.is_empty() {
            return;
        }
        let n = apps.len();
        self.apps = apps;
        self.open = true;
        self.sel = if n > 1 {
            if back {
                n - 1
            } else {
                1
            }
        } else {
            0
        };
    }
    pub fn step(&mut self, back: bool) {
        let n = self.apps.len().max(1);
        self.sel = if back { (self.sel + n - 1) % n } else { (self.sel + 1) % n };
    }
    /// Close and return the chosen app id.
    pub fn commit(&mut self) -> Option<String> {
        if !self.open {
            return None;
        }
        self.open = false;
        self.t = 0.0;
        self.apps.get(self.sel).cloned()
    }
    pub fn cancel(&mut self) {
        self.open = false;
        self.t = 0.0;
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        if !self.open || self.t >= 1.0 {
            return false;
        }
        self.t = (self.t + dt * 9.0).min(1.0);
        true
    }
}

const ICON: f32 = 92.0;
const PAD: f32 = 18.0;
const GAP: f32 = 10.0;

fn rect(sh: &Shell) -> Rect {
    let n = sh.switcher.apps.len() as f32;
    let w = (n * (ICON + GAP) - GAP + 2.0 * PAD).min(sh.w - 40.0);
    let h = ICON + 2.0 * PAD + 30.0;
    Rect::new((sh.w - w) / 2.0, (sh.h - h) / 2.0, w, h)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.switcher.open {
        return None;
    }
    let r = rect(sh);
    let dark = sh.style.is_dark_glass(r);
    let key = hash_of(&(sh.switcher.apps.clone(), sh.switcher.sel, r.w as i32, dark, sh.icons_serial()));
    let (pm, serial) = sh.cached(LayerId::Switcher, key, r.w, r.h, |c, sh| {
        let f = sh.fonts.clone();
        let ipx = (ICON * sh.scale).round() as u32;
        let apps = sh.switcher.apps.clone();
        for (i, id) in apps.iter().enumerate() {
            let x = PAD + i as f32 * (ICON + GAP);
            let cell = Rect::new(x, PAD, ICON, ICON);
            if i == sh.switcher.sel {
                c.fill_rrect(
                    Rect::new(x - 6.0, PAD - 6.0, ICON + 12.0, ICON + 12.0),
                    20.0,
                    rgba(255, 255, 255, if dark { 0.16 } else { 0.38 }),
                );
                c.stroke_rrect(
                    Rect::new(x - 6.0, PAD - 6.0, ICON + 12.0, ICON + 12.0),
                    20.0,
                    rgba(255, 255, 255, 0.45),
                    1.0,
                );
                let name = sh.app_display_name(id);
                c.text_in(
                    &f,
                    Rect::new(0.0, PAD + ICON + 6.0, r.w, 24.0),
                    0.5,
                    14.0,
                    Weight::Medium,
                    style::text_primary(dark),
                    &name,
                );
            }
            let app = aqua_apps::match_app_id(&sh.apps, id);
            let req = IconRequest {
                id: id.clone(),
                name: app.map(|a| a.name.clone()).unwrap_or_else(|| sh.app_display_name(id)),
                icon: app.map(|a| a.icon.clone()).unwrap_or_else(|| id.clone()),
            };
            let icon = sh.icons.get(&req, ipx);
            c.draw_pixmap(&icon, cell, 1.0);
        }
    });
    let t = sh.switcher.t;
    let mut g = style::glass_panel(&sh.cfg.glass, 30.0);
    g.max_luma = 0.8;
    Some(Layer {
        id: LayerId::Switcher,
        rect: r,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: (t * 1.5).min(1.0),
        zoom: 0.97 + 0.03 * t,
    })
}
