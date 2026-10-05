//! Minimal freedesktop icon-theme lookup (PNG + SVG via resvg).
use aqua_gfx::Pixmap;
use std::path::{Path, PathBuf};

fn search_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(h) = dirs::data_dir() {
        v.push(h.join("icons"));
    }
    v.push("/usr/share/icons".into());
    v.push("/usr/local/share/icons".into());
    v
}

pub fn load_any(p: &Path, px: u32) -> Option<Pixmap> {
    match p.extension().and_then(|e| e.to_str()) {
        Some("svg") | Some("svgz") => load_svg(p, px),
        Some("png") | Some("jpg") | Some("jpeg") => aqua_gfx::load_image(p),
        _ => None,
    }
}

/// SVG options that never touch the file system: by default usvg reads every
/// `<image href="/any/path">` with fs::read — `/dev/zero` exhausted memory, a FIFO hung.
pub fn svg_options() -> resvg::usvg::Options<'static> {
    let mut o = resvg::usvg::Options::default();
    o.image_href_resolver.resolve_string = Box::new(|_, _| None);
    o
}

pub fn load_svg(p: &Path, px: u32) -> Option<Pixmap> {
    let data = aqua_gfx::read_regular(p, 16 << 20)?;
    let tree = resvg::usvg::Tree::from_data(&data, &svg_options()).ok()?;
    let size = tree.size();
    let s = px as f32 / size.width().max(size.height());
    let mut pm = Pixmap::new(px, px)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    Some(pm)
}

const THEMES: [&str; 4] = ["hicolor", "Adwaita", "breeze", "Papirus"];

/// Application, place and device icons (large sizes first).
pub fn lookup(name: &str, px: u32) -> Option<Pixmap> {
    lookup_in(
        name,
        px,
        &["scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48"],
        &["apps", "places", "devices", "mimetypes", "categories", "legacy"],
    )
}

/// Status / panel icons (tray items): small sizes first, extra `theme_path` from the item.
pub fn lookup_status(name: &str, theme_path: &str, px: u32) -> Option<Pixmap> {
    if name.is_empty() {
        return None;
    }
    if !theme_path.is_empty() {
        let base = Path::new(theme_path);
        for ext in ["svg", "png"] {
            let f = base.join(format!("{name}.{ext}"));
            if f.exists() {
                return load_any(&f, px);
            }
        }
        if let Ok(rd) = std::fs::read_dir(base) {
            for theme in rd.flatten() {
                for s in ["scalable", "48x48", "32x32", "24x24", "22x22", "16x16"] {
                    for cat in ["status", "apps", "panel"] {
                        for ext in ["svg", "png"] {
                            let f = theme.path().join(s).join(cat).join(format!("{name}.{ext}"));
                            if f.exists() {
                                return load_any(&f, px);
                            }
                        }
                    }
                }
            }
        }
    }
    lookup_in(
        name,
        px,
        &["scalable", "symbolic", "48x48", "32x32", "24x24", "22x22", "16x16", "64x64", "128x128", "256x256"],
        &["status", "panel", "apps", "devices", "actions", "legacy"],
    )
}

fn lookup_in(name: &str, px: u32, sizes: &[&str], cats: &[&str]) -> Option<Pixmap> {
    if name.is_empty() {
        return None;
    }
    let p = Path::new(name);
    if p.is_absolute() {
        return load_any(p, px);
    }
    for base in search_dirs() {
        for t in THEMES {
            for s in sizes {
                for cat in cats {
                    for ext in ["svg", "png"] {
                        let f = base.join(t).join(s).join(cat).join(format!("{name}.{ext}"));
                        if f.exists() {
                            if let Some(pm) = load_any(&f, px) {
                                return Some(pm);
                            }
                        }
                    }
                }
            }
        }
    }
    for ext in ["svg", "png"] {
        let f = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.{ext}"));
        if f.exists() {
            return load_any(&f, px);
        }
    }
    None
}

/// Render a monochrome line symbol (SVG path markup in a 24×24 box, symbol-like
/// 1.8 stroke) at `px` pixels in the given colour (used for menu item icons).
pub fn symbol(body: &str, px: u32, rgba: [u8; 4]) -> Option<Pixmap> {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" width=\"24\" height=\"24\" fill=\"none\" stroke=\"#{:02x}{:02x}{:02x}\" stroke-opacity=\"{:.3}\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{body}</svg>",
        rgba[0], rgba[1], rgba[2], rgba[3] as f32 / 255.0
    );
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &svg_options()).ok()?;
    let s = px as f32 / 24.0;
    let mut pm = Pixmap::new(px, px)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    Some(pm)
}

#[cfg(test)]
mod hostile_tests {
    #[test]
    fn svg_cannot_pull_in_files() {
        let d = std::env::temp_dir().join(format!("aqua-svg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let fifo = d.join("pipe.png");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        extern "C" {
            fn mkfifo(p: *const std::ffi::c_char, m: u32) -> i32;
        }
        assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='10' height='10'><image width='10' height='10' xlink:href='{}'/><image width='10' height='10' href='/dev/zero'/></svg>",
            fifo.display()
        );
        let f = d.join("evil.svg");
        std::fs::write(&f, svg).unwrap();
        let t = std::time::Instant::now();
        assert!(super::load_svg(&f, 32).is_some());
        assert!(super::load_svg(&fifo.with_extension("svg"), 32).is_none());
        assert!(t.elapsed().as_secs() < 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
