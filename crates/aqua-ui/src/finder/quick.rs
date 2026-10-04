//! Quick actions on images: rotate, convert and combine into a PDF.
use super::menus::{mi, on, sep};
use super::*;
use image::{DynamicImage, ImageFormat};
use std::io::Write;

fn format_of(p: &Path) -> Option<ImageFormat> {
    match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

/// Whether the quick actions can work on this file.
pub fn editable_image(p: &Path) -> bool {
    format_of(p).is_some()
}

fn err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn save(img: &DynamicImage, p: &Path, fmt: ImageFormat) -> std::io::Result<()> {
    if fmt == ImageFormat::Jpeg {
        let mut out = std::io::BufWriter::new(std::fs::File::create(p)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&img.to_rgb8()).map_err(err)?;
        return out.flush();
    }
    img.save_with_format(p, fmt).map_err(err)
}

/// Rotate an image in place by a quarter turn.
pub fn rotate(p: &Path, left: bool) -> std::io::Result<()> {
    let fmt = format_of(p).ok_or_else(|| err("unsupported image"))?;
    let img = image::open(p).map_err(err)?;
    let img = if left { img.rotate270() } else { img.rotate90() };
    let tmp = p.with_extension(format!("{}.part", p.extension().and_then(|e| e.to_str()).unwrap_or("")));
    save(&img, &tmp, fmt)?;
    std::fs::rename(&tmp, p)
}

/// Write a copy of `p` in another format next to it; returns the new file.
pub fn convert(p: &Path, png: bool) -> std::io::Result<PathBuf> {
    let img = image::open(p).map_err(err)?;
    let dir = p.parent().unwrap_or(Path::new("/"));
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let out = fs::unique(dir, &format!("{stem}.{}", if png { "png" } else { "jpg" }), "");
    save(&img, &out, if png { ImageFormat::Png } else { ImageFormat::Jpeg })?;
    Ok(out)
}

/// One page per image, each page the size of its picture.
pub fn make_pdf(images: &[PathBuf], out: &Path) -> std::io::Result<()> {
    let mut pages = vec![];
    for p in images {
        let img = image::open(p).map_err(err)?.to_rgb8();
        let (w, h) = img.dimensions();
        let mut jpeg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&img).map_err(err)?;
        pages.push((w, h, jpeg));
    }
    if pages.is_empty() {
        return Err(err("no images"));
    }
    let mut buf: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = vec![];
    let n = pages.len();
    // Objects: 1 catalog, 2 pages, then per page: page, contents, image.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    let mut obj = |buf: &mut Vec<u8>, body: &[u8]| {
        offsets.push(buf.len());
        buf.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        buf.extend_from_slice(body);
        buf.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut buf, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut buf, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    for (i, (w, h, jpeg)) in pages.iter().enumerate() {
        let (pw, ph) = (*w as f32 * 0.75, *h as f32 * 0.75);
        let base = 3 + i * 3;
        obj(
            &mut buf,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                base + 2,
                base + 1
            )
            .as_bytes(),
        );
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        obj(&mut buf, format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).as_bytes());
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            jpeg.len()
        )
        .into_bytes();
        body.extend_from_slice(jpeg);
        body.extend_from_slice(b"\nendstream");
        obj(&mut buf, &body);
    }
    let xref = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        buf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes(),
    );
    std::fs::write(out, buf)
}

impl App {
    /// Selected files the image quick actions apply to.
    pub(super) fn quick_targets(&self) -> Vec<PathBuf> {
        let paths = if self.menu_paths.is_empty() { self.sel_paths() } else { self.menu_paths.clone() };
        paths.into_iter().filter(|p| editable_image(p)).collect()
    }

    pub(super) fn quick_items(&self) -> Vec<FMenuItem> {
        let n = self.quick_targets().len();
        vec![
            on(mi("Rotate Left", "quick:rotl", ""), n > 0),
            on(mi("Rotate Right", "quick:rotr", ""), n > 0),
            on(mi("Create PDF", "quick:pdf", ""), n > 0),
            sep(),
            on(mi("Convert to JPEG", "quick:jpeg", ""), n > 0),
            on(mi("Convert to PNG", "quick:png", ""), n > 0),
        ]
    }

    pub(super) fn quick_action(&mut self, what: &str) {
        let targets = self.quick_targets();
        if targets.is_empty() {
            return;
        }
        let mut made = vec![];
        let mut failed = None;
        match what {
            "rotl" | "rotr" => {
                for p in &targets {
                    if let Err(e) = rotate(p, what == "rotl") {
                        failed = Some((p.clone(), e));
                    }
                    self.thumbs.remove(p);
                }
            }
            "jpeg" | "png" => {
                for p in &targets {
                    match convert(p, what == "png") {
                        Ok(n) => made.push(n),
                        Err(e) => failed = Some((p.clone(), e)),
                    }
                }
            }
            "pdf" => {
                let dir = targets[0].parent().unwrap_or(Path::new("/")).to_path_buf();
                let stem = if targets.len() == 1 {
                    targets[0].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    crate::tr("Untitled").to_string()
                };
                let out = fs::unique(&dir, &format!("{stem}.pdf"), "");
                match make_pdf(&targets, &out) {
                    Ok(()) => made.push(out),
                    Err(e) => failed = Some((targets[0].clone(), e)),
                }
            }
            _ => return,
        }
        if let Some((p, e)) = failed {
            self.message(
                &crate::trf("The operation couldn't be completed for “{name}”.", &[("name", &name_of(&p))]),
                &e.to_string(),
            );
        }
        self.reload_keep(if made.is_empty() { targets } else { made });
        if self.ui().global::<F>().get_ql_open() {
            self.ql_update();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqua-quick-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn picture(p: &Path, w: u32, h: u32) {
        let img =
            image::RgbImage::from_fn(
                w,
                h,
                |x, _| if x == 0 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) },
            );
        img.save(p).unwrap();
    }

    #[test]
    fn rotate_turns_the_picture() {
        let d = tmp("rot");
        let p = d.join("a.png");
        picture(&p, 4, 2);
        rotate(&p, false).unwrap();
        let img = image::open(&p).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (2, 4));
        assert_eq!(img.get_pixel(1, 0), &image::Rgb([255, 0, 0]));
        rotate(&p, true).unwrap();
        let img = image::open(&p).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (4, 2));
        assert_eq!(img.get_pixel(0, 0), &image::Rgb([255, 0, 0]));
        assert!(rotate(&d.join("x.gif"), true).is_err());
    }

    #[test]
    fn convert_and_pdf() {
        let d = tmp("conv");
        let p = d.join("a.png");
        picture(&p, 8, 6);
        let j = convert(&p, false).unwrap();
        assert_eq!(j, d.join("a.jpg"));
        assert_eq!(image::open(&j).unwrap().to_rgb8().dimensions(), (8, 6));
        let again = convert(&j, true).unwrap();
        assert_eq!(again, d.join("a 2.png"));
        let pdf = d.join("out.pdf");
        make_pdf(&[p, j], &pdf).unwrap();
        let bytes = std::fs::read(&pdf).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert_eq!(text.matches("/DCTDecode").count(), 2);
        assert!(text.trim_end().ends_with("%%EOF"));
        let xref: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(bytes[xref..].starts_with(b"xref"));
        assert!(make_pdf(&[], &pdf).is_err());
    }
}
