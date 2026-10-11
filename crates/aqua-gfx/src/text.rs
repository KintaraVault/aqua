//! SF Pro text rendering on top of ab_glyph.

use ab_glyph::{Font, FontArc, Glyph, GlyphId, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tiny_skia::{Color, Pixmap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
}

/// UI translation applied to every string that is measured or drawn (set once by the
/// shell; English UI strings found in the catalog are replaced, anything else is unchanged).
static TRANSLATE: std::sync::OnceLock<fn(&str) -> &str> = std::sync::OnceLock::new();

pub fn set_translator(f: fn(&str) -> &str) {
    let _ = TRANSLATE.set(f);
}

/// Translated form of `s` (identity without a translator).
pub fn translate(s: &str) -> &str {
    match TRANSLATE.get() {
        Some(f) => f(s),
        None => s,
    }
}

pub struct Fonts {
    faces: Vec<(Weight, FontArc)>,
    /// The Mac system font (`SFNS.ttf`, variable: wght + opsz + `trak` table) when
    /// present: instances per (weight, optical size) are created on demand like CoreText does.
    var: Option<VarFace>,
    /// Fallback fonts for characters SF Pro does not have (arrows, math, ⌘⌥⇧⌃,
    /// emoji, CJK…): without them those characters rendered as empty boxes.
    fallback: Mutex<Fallback>,
}

#[derive(Default)]
struct Fallback {
    /// Candidate font files in priority order (bundled first, then system fonts).
    paths: Vec<PathBuf>,
    /// Lazily loaded fonts, parallel to `paths` (None = not loaded yet / failed).
    loaded: Vec<Option<Option<FontArc>>>,
    /// Which fallback font draws a character (None = no font has it).
    by_char: HashMap<char, Option<usize>>,
    /// System font files (file name → path) discovered on demand.
    system: Option<HashMap<String, PathBuf>>,
    /// Decoded colour glyphs (font index, glyph id) at their strike resolution.
    color: HashMap<(usize, u16), Option<Arc<Pixmap>>>,
}

struct VarFace {
    data: &'static [u8],
    /// `trak` track 0: (point size, offset in em).
    trak: Vec<(f32, f32)>,
    inst: Mutex<HashMap<(Weight, u16), FontArc>>,
}

/// Optical sizes instantiated (CoreText uses opsz = point size; the axis is 17…96).
const OPSZ: &[f32] = &[17.0, 20.0, 22.0, 24.0, 28.0, 32.0, 36.0, 48.0, 64.0, 96.0];

impl VarFace {
    fn open(path: &Path) -> Option<Self> {
        // one copy per process (every Fonts instance shares it; instances only hold views)
        static DATA: Mutex<Vec<(PathBuf, &'static [u8])>> = Mutex::new(Vec::new());
        let mut loaded = DATA.lock().unwrap_or_else(|e| e.into_inner());
        let data = match loaded.iter().find(|(p, _)| p == path) {
            Some((_, d)) => *d,
            None => {
                let bytes = std::fs::read(path).ok()?;
                let d: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                loaded.push((path.to_path_buf(), d));
                d
            }
        };
        drop(loaded);
        let f = ab_glyph::FontRef::try_from_slice(data).ok()?;
        f.glyph_id('a').0.ne(&0).then_some(())?;
        Some(Self { data, trak: parse_trak(data).unwrap_or_default(), inst: Mutex::new(HashMap::new()) })
    }

    fn wght(w: Weight) -> f32 {
        // SFNS named instances
        match w {
            Weight::Light => 274.315,
            Weight::Regular => 400.0,
            Weight::Medium => 510.0,
            Weight::Semibold => 590.0,
            Weight::Bold => 700.0,
        }
    }

    fn face(&self, w: Weight, size: f32) -> FontArc {
        let s = size.clamp(17.0, 96.0);
        let opsz = OPSZ.iter().copied().min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs())).unwrap_or(17.0);
        let key = (w, opsz as u16);
        let mut m = self.inst.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(key)
            .or_insert_with(|| {
                use ab_glyph::VariableFont;
                let mut f = ab_glyph::FontRef::try_from_slice(self.data).expect("validated");
                f.set_variation(b"wght", Self::wght(w));
                f.set_variation(b"opsz", opsz);
                FontArc::new(f)
            })
            .clone()
    }

    /// CoreText's automatic tracking for `size` pt (px, logical).
    fn tracking(&self, size: f32) -> f32 {
        let t = &self.trak;
        if t.is_empty() {
            return 0.0;
        }
        let em = if size <= t[0].0 {
            t[0].1
        } else if size >= t[t.len() - 1].0 {
            t[t.len() - 1].1
        } else {
            let i = t.iter().position(|e| e.0 >= size).unwrap_or(1).max(1);
            let (a, b) = (t[i - 1], t[i]);
            a.1 + (b.1 - a.1) * (size - a.0) / (b.0 - a.0)
        };
        em * size
    }
}

/// Horizontal track 0 of a `trak` table as (size, em offset).
fn parse_trak(d: &[u8]) -> Option<Vec<(f32, f32)>> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let i16_at = |o: usize| u16_at(o).map(|v| v as i16);
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let fixed = |o: usize| u32_at(o).map(|v| v as i32 as f32 / 65536.0);
    let ntab = u16_at(4)? as usize;
    let mut trak = None;
    let mut upem = 2048.0;
    for i in 0..ntab {
        let r = 12 + i * 16;
        let tag = d.get(r..r + 4)?;
        let off = u32_at(r + 8)? as usize;
        if tag == b"trak" {
            trak = Some(off);
        } else if tag == b"head" {
            upem = u16_at(off + 18)? as f32;
        }
    }
    let t = trak?;
    let horiz = u16_at(t + 6)? as usize;
    if horiz == 0 {
        return None;
    }
    let h = t + horiz;
    let (ntracks, nsizes) = (u16_at(h)? as usize, u16_at(h + 2)? as usize);
    let size_off = u32_at(h + 4)? as usize;
    let sizes: Vec<f32> = (0..nsizes).map(|k| fixed(t + size_off + 4 * k)).collect::<Option<_>>()?;
    for k in 0..ntracks {
        let e = h + 8 + k * 8;
        if fixed(e)? == 0.0 {
            let vo = u16_at(e + 6)? as usize;
            return (0..nsizes).map(|j| Some((sizes[j], i16_at(t + vo + 2 * j)? as f32 / upem))).collect();
        }
    }
    None
}

/// Font files tried (in this order) when SF Pro lacks a character.
const FALLBACK_FILES: &[&str] = &[
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "Symbola.ttf",
    "NotoSansCJK-Regular.ttc",
    "NotoSansCJKsc-Regular.otf",
    "NotoSansSC-Regular.otf",
    "NotoSansJP-Regular.otf",
    "wqy-microhei.ttc",
    "DroidSansFallbackFull.ttf",
    "DroidSansFallback.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansHebrew-Regular.ttf",
    "NotoSansDevanagari-Regular.ttf",
    "NotoSansThai-Regular.ttf",
    "LiberationSans-Regular.ttf",
];
/// Colour emoji fonts (CBDT/sbix PNG strikes), tried first for emoji code points.
const EMOJI_FILES: &[&str] = &[
    "NotoColorEmoji.ttf",
    "Noto-COLRv1.ttf",
    "TwemojiMozilla.ttf",
    "Twemoji.ttf",
    "JoyPixels.ttf",
    "EmojiOneColor.otf",
    "NotoEmoji-Regular.ttf",
];

/// Characters that should prefer the colour emoji font.
fn is_emoji(c: char) -> bool {
    let u = c as u32;
    (0x1F000..=0x1FAFF).contains(&u)
        || (0x2600..=0x27BF).contains(&u)
            && matches!(u, 0x2614 | 0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA | 0x26AB | 0x26BD | 0x26BE | 0x26C4 | 0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2..=0x26F5 | 0x26FA | 0x26FD | 0x2705 | 0x270A | 0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 | 0x27B0 | 0x27BF)
}

/// Zero-width characters that only modify neighbours (variation selectors, ZWJ).
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0xFE00..=0xFE0F | 0x200B..=0x200D | 0x2060 | 0xE0020..=0xE007F | 0x1F3FB..=0x1F3FF)
}

/// One character resolved to the font that draws it.
struct Resolved {
    font: FontArc,
    id: GlyphId,
    /// Index into the fallback list (None = SF Pro face).
    fb: Option<usize>,
    /// The glyph is a colour bitmap (emoji).
    color: bool,
}

impl Fonts {
    /// Load the UI font from `dir`: the Mac system font `SFNS.ttf` (variable, optical
    /// sizes) when present, else the SF Pro Display faces.
    pub fn load(dir: &Path) -> Self {
        let mut cands = vec![dir.join("SFNS.ttf")];
        // installed by dist/install.sh from the Mac assets (a development build's `dir` is the
        // repository's assets/fonts, which only has SF Pro Display)
        cands.extend(["/usr/local/share/aqua/fonts/SFNS.ttf", "/usr/share/aqua/fonts/SFNS.ttf"].map(PathBuf::from));
        if let Some(h) = std::env::var_os("HOME") {
            cands.push(PathBuf::from(h).join("linuxtosh-assets/assets/mac/fonts/SFNS.ttf"));
        }
        let var = if std::env::var_os("AQUA_NO_SYSTEM_FONT").is_some() {
            None
        } else {
            cands.iter().find_map(|p| VarFace::open(p))
        };
        let wanted = [
            (Weight::Light, "SF-Pro-Display-Light.otf"),
            (Weight::Regular, "SF-Pro-Display-Regular.otf"),
            (Weight::Medium, "SF-Pro-Display-Medium.otf"),
            (Weight::Semibold, "SF-Pro-Display-Semibold.otf"),
            (Weight::Bold, "SF-Pro-Display-Bold.otf"),
        ];
        let mut faces = Vec::new();
        for (w, f) in wanted {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                if let Ok(font) = FontArc::try_from_vec(bytes) {
                    faces.push((w, font));
                }
            }
        }
        if let Some(v) = &var {
            faces.insert(0, (Weight::Regular, v.face(Weight::Regular, 13.0)));
        }
        if faces.is_empty() {
            for p in [
                dir.join("DejaVuSans.ttf"),
                "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf".into(),
                "/usr/share/fonts/liberation/LiberationSans-Regular.ttf".into(),
                "/usr/share/fonts/dejavu/DejaVuSans.ttf".into(),
                "/usr/share/fonts/TTF/DejaVuSans.ttf".into(),
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf".into(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
            ] {
                if let Ok(b) = std::fs::read(&p) {
                    if let Ok(font) = FontArc::try_from_vec(b) {
                        faces.push((Weight::Regular, font));
                        break;
                    }
                }
            }
        }
        assert!(!faces.is_empty(), "aqua: no usable font found in {}", dir.display());
        let mut fb = Fallback::default();
        for f in FALLBACK_FILES.iter().chain(EMOJI_FILES) {
            for d in [dir.to_path_buf(), dir.join("fallback")] {
                let p = d.join(f);
                if p.is_file() && !fb.paths.contains(&p) {
                    fb.paths.push(p);
                    fb.loaded.push(None);
                }
            }
        }
        Self { faces, var, fallback: Mutex::new(fb) }
    }

    fn face(&self, w: Weight) -> FontArc {
        self.face_at(w, 13.0)
    }

    /// The face for weight `w` at `size` pt (optical size for the system font).
    fn face_at(&self, w: Weight, size: f32) -> FontArc {
        if let Some(v) = &self.var {
            return v.face(w, size);
        }
        self.faces
            .iter()
            .find(|(fw, _)| *fw == w)
            .or_else(|| self.faces.iter().find(|(fw, _)| *fw == Weight::Regular))
            .map(|(_, f)| f.clone())
            .unwrap_or_else(|| self.faces[0].1.clone())
    }

    /// The font (SF Pro, else a fallback) that has a glyph for `c`.
    fn resolve(&self, c: char, w: Weight, size: f32) -> Resolved {
        let face = self.face_at(w, size);
        let id = face.glyph_id(c);
        let emoji = is_emoji(c);
        if (id.0 != 0 && !emoji) || c.is_whitespace() || c.is_control() || is_invisible(c) {
            return Resolved { font: face.clone(), id, fb: None, color: false };
        }
        let mut fb = self.fallback.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = fb.find(c, emoji) {
            if let Some(Some(Some(f))) = fb.loaded.get(i).cloned() {
                let gid = f.glyph_id(c);
                let color = f.outline(gid).is_none() && f.glyph_raster_image2(gid, 64).is_some();
                return Resolved { font: f, id: gid, fb: Some(i), color };
            }
        }
        Resolved { font: face.clone(), id, fb: None, color: false }
    }

    fn px_scale(font: &FontArc, size: f32) -> PxScale {
        let upem = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(size * font.height_unscaled() / upem)
    }

    /// Scale making a fallback font's em match SF Pro's em at `size`.
    fn px_scale_for(&self, r: &Resolved, w: Weight, size: f32) -> PxScale {
        if r.fb.is_none() {
            return Self::px_scale(&r.font, size);
        }
        let em = |f: &FontArc| Self::px_scale(f, size).y * f.units_per_em().unwrap_or(1000.0) / f.height_unscaled();
        let target_em = em(&self.face(w));
        let upem = r.font.units_per_em().unwrap_or(1000.0);
        PxScale::from(target_em * r.font.height_unscaled() / upem)
    }

    /// Optical tracking used by SF Pro (negative tracking at display sizes).
    fn tracking(&self, size: f32) -> f32 {
        if let Some(v) = &self.var {
            return v.tracking(size);
        }
        if size >= 20.0 {
            -0.012 * size
        } else if size >= 13.0 {
            -0.004 * size
        } else {
            0.0
        }
    }

    fn advance(&self, r: &Resolved, w: Weight, size: f32) -> f32 {
        let sc = self.px_scale_for(r, w, size);
        let adv = r.font.as_scaled(sc).h_advance(r.id);
        if r.color {
            return adv.max(size * 1.15);
        }
        adv
    }

    pub fn measure(&self, s: &str, size: f32, w: Weight) -> f32 {
        let s = translate(s);
        let mut x = 0.0;
        let mut prev: Option<(Option<usize>, GlyphId)> = None;
        for c in s.chars() {
            if is_invisible(c) {
                continue;
            }
            let r = self.resolve(c, w, size);
            if let Some((pf, p)) = prev {
                if pf == r.fb && r.fb.is_none() {
                    x += r.font.as_scaled(Self::px_scale(&r.font, size)).kern(p, r.id);
                }
            }
            x += self.advance(&r, w, size) + self.tracking(size);
            prev = Some((r.fb, r.id));
        }
        x
    }

    /// Does SF Pro or an installed fallback font have a glyph for `c`?
    pub fn has_glyph(&self, c: char) -> bool {
        let r = self.resolve(c, Weight::Regular, 13.0);
        r.id.0 != 0
    }

    pub fn cap_height(&self, size: f32, _w: Weight) -> f32 {
        size * 0.70
    }
    pub fn ascent(&self, size: f32, w: Weight) -> f32 {
        let font = self.face_at(w, size);
        font.as_scaled(Self::px_scale(&font, size)).ascent()
    }

    /// `s` shortened with "…" to fit `max` px. Binary search over the prefix length: window
    /// titles can be thousands of characters long (a browser tab with a data: URL), and
    /// re-measuring after every dropped character froze the compositor for seconds.
    pub fn ellipsize(&self, s: &str, size: f32, w: Weight, max: f32) -> String {
        let s = translate(s);
        // No glyph is narrower than ~1 px, so longer prefixes can never fit.
        let cap = (max.max(0.0) as usize).saturating_add(2);
        let head: &str = match s.char_indices().nth(cap) {
            Some((i, _)) => &s[..i],
            None => s,
        };
        if head.len() == s.len() && self.measure(s, size, w) <= max {
            return s.to_string();
        }
        let chars: Vec<char> = head.chars().collect();
        let cand = |k: usize| chars[..k].iter().collect::<String>().trim_end().to_string() + "…";
        let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
        if hi == 0 || self.measure(&cand(1), size, w) > max {
            return "…".into();
        }
        lo = lo.max(1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.measure(&cand(mid), size, w) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        cand(lo)
    }

    /// Decoded colour glyph (PNG strike) of a fallback font.
    fn color_glyph(&self, fb: usize, font: &FontArc, id: GlyphId) -> Option<Arc<Pixmap>> {
        let mut st = self.fallback.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = st.color.get(&(fb, id.0)) {
            return c.clone();
        }
        let pm = font.glyph_raster_image2(id, 160).and_then(|img| {
            if !matches!(img.format, ab_glyph::GlyphImageFormat::Png) {
                return None;
            }
            let rgba = image::load_from_memory(img.data).ok()?.to_rgba8();
            let (w, h) = rgba.dimensions();
            let mut pm = Pixmap::new(w, h)?;
            for (dst, src) in pm.pixels_mut().iter_mut().zip(rgba.pixels()) {
                let [r, g, b, a] = src.0;
                let pr = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
                *dst = tiny_skia::PremultipliedColorU8::from_rgba(pr(r), pr(g), pr(b), a)
                    .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
            }
            Some(Arc::new(pm))
        });
        st.color.insert((fb, id.0), pm.clone());
        pm
    }

    /// Draw text in logical coords; returns advance width (logical).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        pm: &mut Pixmap,
        scale: f32,
        x: f32,
        baseline: f32,
        size: f32,
        w: Weight,
        color: Color,
        s: &str,
    ) -> f32 {
        let s = translate(s);
        let psize = size * scale;
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let c = color.premultiply().to_color_u8();
        let (cr, cg, cb, ca) = (c.red() as f32, c.green() as f32, c.blue() as f32, c.alpha() as f32);
        let mut pen = x * scale;
        let by = baseline * scale;
        let mut prev: Option<(Option<usize>, GlyphId)> = None;
        for ch in s.chars() {
            if is_invisible(ch) {
                continue;
            }
            let r = self.resolve(ch, w, size);
            let sc = self.px_scale_for(&r, w, psize);
            let sf = r.font.as_scaled(sc);
            if let Some((pf, p)) = prev {
                if pf == r.fb && r.fb.is_none() {
                    pen += sf.kern(p, r.id);
                }
            }
            let adv = self.advance(&r, w, psize);
            if r.color {
                if let Some(img) = r.fb.and_then(|fb| self.color_glyph(fb, &r.font, r.id)) {
                    let em = psize * 1.12;
                    let k = em / img.height().max(1) as f32;
                    let dw = img.width() as f32 * k;
                    let tx = pen + (adv - dw) / 2.0;
                    let ty = by - em * 0.86;
                    let paint = tiny_skia::PixmapPaint {
                        opacity: color.alpha(),
                        quality: tiny_skia::FilterQuality::Bicubic,
                        ..Default::default()
                    };
                    pm.draw_pixmap(
                        0,
                        0,
                        (*img).as_ref(),
                        &paint,
                        tiny_skia::Transform::from_row(k, 0.0, 0.0, k, tx, ty),
                        None,
                    );
                }
            } else {
                let g: Glyph = r.id.with_scale_and_position(sc, ab_glyph::point(pen, by));
                if let Some(og) = r.font.outline_glyph(g) {
                    let b = og.px_bounds();
                    let data = pm.data_mut();
                    og.draw(|gx, gy, cov| {
                        let px = b.min.x as i32 + gx as i32;
                        let py = b.min.y as i32 + gy as i32;
                        if px < 0 || py < 0 || px >= pw || py >= ph {
                            return;
                        }
                        let cov = cov.clamp(0.0, 1.0).powf(0.85);
                        let i = ((py * pw + px) * 4) as usize;
                        let a = ca * cov / 255.0;
                        let inv = 1.0 - a;
                        data[i] = (cr * cov + data[i] as f32 * inv).min(255.0) as u8;
                        data[i + 1] = (cg * cov + data[i + 1] as f32 * inv).min(255.0) as u8;
                        data[i + 2] = (cb * cov + data[i + 2] as f32 * inv).min(255.0) as u8;
                        data[i + 3] = (ca * cov + data[i + 3] as f32 * inv).min(255.0) as u8;
                    });
                }
            }
            pen += adv + self.tracking(size) * scale;
            prev = Some((r.fb, r.id));
        }
        pen / scale - x
    }
}

impl Fallback {
    fn load(&mut self, i: usize) -> Option<FontArc> {
        if let Some(Some(f)) = self.loaded.get(i) {
            return f.clone();
        }
        let p = self.paths.get(i)?.clone();
        let f = std::fs::read(&p).ok().and_then(|b| {
            let is_ttc = p.extension().map(|e| e.eq_ignore_ascii_case("ttc")).unwrap_or(false);
            if is_ttc {
                ab_glyph::FontVec::try_from_vec_and_index(b, 0).ok().map(FontArc::new)
            } else {
                FontArc::try_from_vec(b).ok()
            }
        });
        if let Some(slot) = self.loaded.get_mut(i) {
            *slot = Some(f.clone());
        }
        f
    }

    fn add_path(&mut self, p: PathBuf) -> usize {
        if let Some(i) = self.paths.iter().position(|x| *x == p) {
            return i;
        }
        self.paths.push(p);
        self.loaded.push(None);
        self.paths.len() - 1
    }

    /// Index of a fallback font covering `c` (loading fonts lazily).
    fn find(&mut self, c: char, emoji: bool) -> Option<usize> {
        if let Some(r) = self.by_char.get(&c) {
            return *r;
        }
        let has = |f: &FontArc| f.glyph_id(c).0 != 0;
        let mut found = None;
        if emoji {
            self.discover();
            let sys = self.system.clone().unwrap_or_default();
            for f in EMOJI_FILES {
                if let Some(p) = sys.get(*f).cloned() {
                    let i = self.add_path(p);
                    if self.load(i).map(|f| has(&f)).unwrap_or(false) {
                        found = Some(i);
                        break;
                    }
                }
            }
        }
        if found.is_none() {
            for i in 0..self.paths.len() {
                if self.load(i).map(|f| has(&f)).unwrap_or(false) {
                    found = Some(i);
                    break;
                }
            }
        }
        if found.is_none() {
            self.discover();
            let sys = self.system.clone().unwrap_or_default();
            for f in FALLBACK_FILES.iter().chain(EMOJI_FILES) {
                if let Some(p) = sys.get(*f).cloned() {
                    if self.paths.contains(&p) {
                        continue;
                    }
                    let i = self.add_path(p);
                    if self.load(i).map(|f| has(&f)).unwrap_or(false) {
                        found = Some(i);
                        break;
                    }
                }
            }
        }
        if found.is_none() {
            if let Some(p) = fc_font_for(c) {
                let i = self.add_path(p);
                if self.load(i).map(|f| has(&f)).unwrap_or(false) {
                    found = Some(i);
                }
            }
        }
        self.by_char.insert(c, found);
        found
    }

    /// Index the system font directories once (file name → path).
    fn discover(&mut self) {
        if self.system.is_some() {
            return;
        }
        let mut m = HashMap::new();
        let mut roots: Vec<PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
        if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.push(h.join(".local/share/fonts"));
            roots.push(h.join(".fonts"));
        }
        fn walk(d: &Path, depth: u32, m: &mut HashMap<String, PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if depth < 5 {
                        walk(&p, depth + 1, m);
                    }
                } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                    m.entry(n.to_string()).or_insert(p.clone());
                }
            }
        }
        for r in roots {
            walk(&r, 0, &mut m);
        }
        self.system = Some(m);
    }
}

/// A font file containing `c`, as reported by fontconfig (regular faces preferred).
fn fc_font_for(c: char) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-list")
        .arg(format!(":charset={:x}", c as u32))
        .arg("file")
        .arg("style")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(i32, PathBuf)> = None;
    for l in text.lines() {
        let (file, style) =
            l.split_once(':').map(|(f, s)| (f.trim(), s.to_lowercase())).unwrap_or((l.trim(), String::new()));
        let ext = file.rsplit('.').next().unwrap_or("").to_lowercase();
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
            continue;
        }
        let mut score = 0;
        if style.contains("regular") || style.contains("book") {
            score += 2;
        }
        if style.contains("bold")
            || style.contains("italic")
            || style.contains("oblique")
            || style.contains("condensed")
        {
            score -= 3;
        }
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, PathBuf::from(file)));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"))
    }

    #[test]
    fn ellipsize_fits_and_keeps_short_text() {
        let f = fonts();
        assert_eq!(f.ellipsize("Finder", 13.0, Weight::Regular, 500.0), "Finder");
        let long = "Очень длинный заголовок окна ".repeat(20);
        let e = f.ellipsize(&long, 13.0, Weight::Regular, 200.0);
        assert!(e.ends_with('…') && e.chars().count() > 5, "{e}");
        assert!(f.measure(&e, 13.0, Weight::Regular) <= 200.0);
        assert_eq!(f.ellipsize("abc", 13.0, Weight::Regular, 1.0), "…");
        assert_eq!(f.ellipsize("", 13.0, Weight::Regular, 0.0), "");
    }

    #[test]
    fn ellipsize_huge_titles_quickly() {
        let f = fonts();
        let title = "data:text/html;base64,".to_string() + &"QUFB".repeat(50_000);
        let t0 = std::time::Instant::now();
        let e = f.ellipsize(&title, 13.0, Weight::Semibold, 600.0);
        assert!(e.ends_with('…'));
        assert!(t0.elapsed() < std::time::Duration::from_millis(500), "{:?}", t0.elapsed());
    }
}
