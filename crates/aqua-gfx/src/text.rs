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
    /// Load SF Pro Display faces from `dir`.
    pub fn load(dir: &Path) -> Self {
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
        Self { faces, fallback: Mutex::new(fb) }
    }

    fn face(&self, w: Weight) -> &FontArc {
        self.faces
            .iter()
            .find(|(fw, _)| *fw == w)
            .or_else(|| self.faces.iter().find(|(fw, _)| *fw == Weight::Regular))
            .map(|(_, f)| f)
            .unwrap_or(&self.faces[0].1)
    }

    /// The font (SF Pro, else a fallback) that has a glyph for `c`.
    fn resolve(&self, c: char, w: Weight) -> Resolved {
        let face = self.face(w);
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
        let target_em = em(self.face(w));
        let upem = r.font.units_per_em().unwrap_or(1000.0);
        PxScale::from(target_em * r.font.height_unscaled() / upem)
    }

    /// Optical tracking used by SF Pro (negative tracking at display sizes).
    fn tracking(size: f32) -> f32 {
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
            let r = self.resolve(c, w);
            if let Some((pf, p)) = prev {
                if pf == r.fb && r.fb.is_none() {
                    x += r.font.as_scaled(Self::px_scale(&r.font, size)).kern(p, r.id);
                }
            }
            x += self.advance(&r, w, size) + Self::tracking(size);
            prev = Some((r.fb, r.id));
        }
        x
    }

    /// Does SF Pro or an installed fallback font have a glyph for `c`?
    pub fn has_glyph(&self, c: char) -> bool {
        let r = self.resolve(c, Weight::Regular);
        r.id.0 != 0
    }

    pub fn cap_height(&self, size: f32, _w: Weight) -> f32 {
        size * 0.70
    }
    pub fn ascent(&self, size: f32, w: Weight) -> f32 {
        let font = self.face(w);
        font.as_scaled(Self::px_scale(font, size)).ascent()
    }

    pub fn ellipsize(&self, s: &str, size: f32, w: Weight, max: f32) -> String {
        let s = translate(s);
        if self.measure(s, size, w) <= max {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let t: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
            if self.measure(&t, size, w) <= max {
                return t;
            }
        }
        "…".into()
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
            let r = self.resolve(ch, w);
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
            pen += adv + Self::tracking(size) * scale;
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
