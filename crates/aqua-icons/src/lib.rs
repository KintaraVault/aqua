//! aqua-icons: icon resolution pipeline.
//!
//! Lookup order for an app:
//! 0. **User-supplied genuine icons**: `~/.local/share/aqua/icons/<id>.icns|png`
//!    (e.g. copied out of a Mac's `/Applications/X.app/Contents/Resources`).
//! 1. **icon cache** (`~/.cache/aqua/icons/<id>.png`) – vendor versions
//!    fetched by [`fetch`] (Mac App Store artwork) or extracted from `.icns` files.
//! 2. **Built-in** procedural icons (`builtin:finder`, …) – [`builtin`].
//! 3. **Freedesktop theme** icon (PNG/SVG), adapted to the grid (squircle plate).
//! 4. Generated monogram placeholder.
//!
//! Every icon is normalised to the 1024 icon grid (824 px body, transparent margin,
//! baked contact shadow) so they line up, and keep full alpha so the
//! compositor's icon shaders can operate on them.

pub mod builtin;
pub mod equiv;
mod fetch;
pub mod glass;
pub mod glyph;
pub mod icns;
pub mod look;
pub mod normalize;
pub mod theme;

use aqua_gfx::{Fonts, Pixmap};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct IconProvider {
    cache_dir: PathBuf,
    fonts: Arc<Fonts>,
    /// Rendered icons with their last use (unused ones are trimmed).
    memo: HashMap<(String, u32), (Pixmap, std::time::Instant)>,
    look: look::Look,
    fetcher: Option<fetch::Fetcher>,
    /// Set when a background fetch finished (shell should re-request icons).
    pub dirty: Arc<Mutex<bool>>,
    /// Which apps may be shown with their counterpart's icon.
    policy: equiv::Policy,
}

/// What to show for an app.
#[derive(Clone, Debug, Default)]
pub struct IconRequest {
    /// Stable id (desktop file id without `.desktop`, or builtin key).
    pub id: String,
    /// Human readable name (used for store lookups / monograms).
    pub name: String,
    /// `Icon=` value from the desktop entry (name or path) or `builtin:xxx`.
    pub icon: String,
}

impl IconProvider {
    pub fn new(cache_dir: PathBuf, fonts: Arc<Fonts>, allow_fetch: bool) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        let dirty = Arc::new(Mutex::new(false));
        let fetcher = allow_fetch.then(|| fetch::Fetcher::spawn(cache_dir.clone(), dirty.clone()));
        Self {
            cache_dir,
            fonts,
            memo: HashMap::new(),
            look: look::Look::default(),
            fetcher,
            dirty,
            policy: equiv::Policy { mode: "all".into(), apps: vec![], owners: vec![] },
        }
    }

    /// Branded-icon replacement policy (System Settings → Appearance → App Icons).
    pub fn set_policy(&mut self, p: equiv::Policy) {
        if p != self.policy {
            self.policy = p;
            self.memo.clear();
        }
    }

    /// Icon style (Default / Dark / Clear / Tinted, glass rim).
    pub fn set_look(&mut self, l: look::Look) {
        if l != self.look {
            self.look = l;
            self.memo.clear();
        }
    }

    /// Forget icons not drawn for `max_age` (memory: every size of every app adds up).
    pub fn trim(&mut self, max_age: std::time::Duration, keep: &std::collections::HashSet<String>) -> usize {
        let mut freed = 0;
        self.memo.retain(|(id, _), (pm, t)| {
            let k = keep.contains(id) || t.elapsed() < max_age;
            if !k {
                freed += pm.data().len();
            }
            k
        });
        freed
    }

    pub fn policy(&self) -> &equiv::Policy {
        &self.policy
    }

    pub fn take_dirty(&self) -> bool {
        let mut d = self.dirty.lock().unwrap();
        std::mem::replace(&mut *d, false)
    }

    pub fn invalidate(&mut self) {
        self.memo.clear();
    }

    /// Return the icon at `px` physical pixels (square canvas, icon-grid).
    pub fn get(&mut self, req: &IconRequest, px: u32) -> Pixmap {
        let px = px.clamp(16, 2048);
        let key = (req.id.clone(), px);
        if let Some((p, t)) = self.memo.get_mut(&key) {
            *t = std::time::Instant::now();
            return p.clone();
        }
        let (mut pm, builtin) = self.resolve(req, px);
        if !builtin && self.look.style != look::Style::Default {
            let k = if px <= 160 {
                3
            } else if px <= 520 {
                2
            } else {
                1
            };
            if k > 1 {
                let (mut big, b2) = self.resolve(req, px * k);
                if !b2 && big.width() == px * k {
                    look::apply(&mut big, &self.look, false);
                    pm = glass::downsample(&big, k);
                } else {
                    look::apply(&mut pm, &self.look, builtin);
                }
            } else {
                look::apply(&mut pm, &self.look, builtin);
            }
        } else {
            look::apply(&mut pm, &self.look, builtin);
        }
        self.memo.insert(key, (pm.clone(), std::time::Instant::now()));
        pm
    }

    /// The icon and whether it is one of ours (built-in / monogram, with its own glass finish).
    fn resolve(&mut self, req: &IconRequest, px: u32) -> (Pixmap, bool) {
        if !req.icon.starts_with("builtin:") {
            if let Some(pm) = user_icon(&req.id, &req.icon) {
                return (normalize::fit_macos(&pm, px), false);
            }
        }
        match self.resolve_inner(req, px) {
            Ok(p) => (p, false),
            Err(p) => (p, true),
        }
    }

    /// Ok = foreign bitmap, Err = drawn by us.
    fn resolve_inner(&mut self, req: &IconRequest, px: u32) -> Result<Pixmap, Pixmap> {
        let mut apple_store: Option<&str> = None;
        if !req.icon.starts_with("builtin:") && self.policy.allows(&req.id, &req.icon) {
            match equiv::lookup(&req.id, &req.icon) {
                Some(equiv::Equiv::Builtin(b)) => {
                    if let Some(pm) = builtin::draw_look(b, px, &self.fonts, &self.look) {
                        return Err(pm);
                    }
                }
                Some(equiv::Equiv::Store(n)) => apple_store = Some(*n),
                None => {}
            }
        }
        if let Some(n) = apple_store {
            let cid = format!("apple-{}", sanitize(n));
            if let Some(pm) = aqua_gfx::load_image(&self.cache_dir.join(format!("{cid}.png"))) {
                return Ok(normalize::fit_macos(&pm, px));
            }
            if let Some(f) = &self.fetcher {
                f.request(&cid, n);
            }
        }
        let cached = self.cache_dir.join(format!("{}.png", sanitize(&req.id)));
        if let Some(pm) = aqua_gfx::load_image(&cached) {
            return Ok(normalize::fit_macos(&pm, px));
        }
        if let Some(name) = req.icon.strip_prefix("builtin:") {
            if let Some(pm) = builtin::draw_look(name, px, &self.fonts, &self.look) {
                return Err(pm);
            }
        }
        if let Some(f) = &self.fetcher {
            if apple_store.is_none() && !req.icon.starts_with("builtin:") && !req.name.is_empty() {
                f.request_app(&req.id, &req.name, &req.icon);
            }
        }
        if req.icon.ends_with(".icns") {
            if let Some(pm) = icns::load_best(std::path::Path::new(&req.icon)) {
                return Ok(normalize::fit_macos(&pm, px));
            }
        }
        if let Some(pm) = theme::lookup(&req.icon, px.max(128)) {
            if self.look.style != look::Style::Default {
                return Err(normalize::plate_look(&pm, px, &self.look));
            }
            return Ok(normalize::plate(&pm, px));
        }
        Err(builtin::monogram_look(&req.name, px, &self.fonts, &self.look))
    }
}

/// Genuine icons the user put in `~/.local/share/aqua/icons` (`.icns` from a Mac app
/// bundle, or a PNG), named after the desktop id or the `Icon=` name.
fn user_icon(id: &str, icon: &str) -> Option<Pixmap> {
    let dir = dirs::data_dir()?.join("aqua/icons");
    if !dir.is_dir() {
        return None;
    }
    let short = id.rsplit('.').next().unwrap_or(id);
    for n in [id, icon, short] {
        if n.is_empty() || n.contains('/') {
            continue;
        }
        for cand in [n.to_string(), n.to_lowercase()] {
            let base = sanitize(&cand);
            if let Some(pm) = icns::load_best(&dir.join(format!("{base}.icns"))) {
                return Some(pm);
            }
            if let Some(pm) = aqua_gfx::load_image(&dir.join(format!("{base}.png"))) {
                return Some(pm);
            }
        }
    }
    None
}

pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}
