//! `.icns` reader (PNG / JPEG-2000-free subset): picks the largest PNG member.
//! Used for icons extracted from application bundles.
use aqua_gfx::Pixmap;
use std::path::Path;

/// Largest PNG rendition in an `.icns` file.
pub fn load_best(path: &Path) -> Option<Pixmap> {
    let data = aqua_gfx::read_regular(path, 64 << 20)?;
    parse(&data)
}

pub fn parse(data: &[u8]) -> Option<Pixmap> {
    if data.len() < 8 || &data[0..4] != b"icns" {
        return None;
    }
    let mut best: Option<Pixmap> = None;
    let mut off = 8;
    while off + 8 <= data.len() {
        let len = u32::from_be_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]) as usize;
        if len < 8 || off + len > data.len() {
            break;
        }
        let body = &data[off + 8..off + len];
        if body.starts_with(&[0x89, b'P', b'N', b'G']) {
            if let Some(pm) = aqua_gfx::load_image_bytes(body) {
                if best.as_ref().map(|b| pm.width() > b.width()).unwrap_or(true) {
                    best = Some(pm);
                }
            }
        }
        off += len;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32) -> Vec<u8> {
        let mut pm = Pixmap::new(w, w).unwrap();
        pm.fill(aqua_gfx::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        pm.encode_png().unwrap()
    }

    fn icns(members: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = vec![];
        for (ty, data) in members {
            body.extend_from_slice(*ty);
            body.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
            body.extend_from_slice(data);
        }
        let mut out = b"icns".to_vec();
        out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn picks_largest_png() {
        let data = icns(&[(b"ic07", png(16)), (b"ic09", png(64)), (b"is32", vec![1, 2, 3]), (b"ic08", png(32))]);
        assert_eq!(parse(&data).map(|p| p.width()), Some(64));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"").is_none());
        assert!(parse(b"notanicnsfile").is_none());
        assert!(parse(&icns(&[(b"is32", vec![0; 10])])).is_none());
        let mut truncated = icns(&[(b"ic07", png(16))]);
        truncated.truncate(20);
        assert!(parse(&truncated).is_none());
    }
}
