//! CPU blur used for the software glass path, wallpaper preprocessing and shadows.
use tiny_skia::Pixmap;

fn box_pass_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let div = (2 * r + 1) as u32;
    for y in 0..h {
        let row = y * w * 4;
        let mut acc = [0u32; 4];
        for i in 0..=2 * r {
            let x = i.saturating_sub(r).min(w - 1);
            for c in 0..4 {
                acc[c] += src[row + x * 4 + c] as u32;
            }
        }
        for x in 0..w {
            for c in 0..4 {
                dst[row + x * 4 + c] = (acc[c] / div) as u8;
            }
            let out = x.saturating_sub(r);
            let inn = (x + r + 1).min(w - 1);
            for c in 0..4 {
                acc[c] = acc[c] + src[row + inn * 4 + c] as u32 - src[row + out * 4 + c] as u32;
            }
        }
    }
}

fn transpose(src: &[u8], dst: &mut [u8], w: usize, h: usize) {
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 4;
            let d = (x * h + y) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

/// Approximate gaussian blur with sigma ≈ radius/2 via three box passes.
pub fn blur(pm: &mut Pixmap, radius: f32) {
    if radius < 0.5 {
        return;
    }
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let sigma = radius / 2.0;
    let r = (((12.0 * sigma * sigma / 3.0 + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize;
    let mut a = pm.data().to_vec();
    let mut b = vec![0u8; a.len()];
    for _ in 0..3 {
        box_pass_h(&a, &mut b, w, h, r);
        std::mem::swap(&mut a, &mut b);
    }
    transpose(&a, &mut b, w, h);
    std::mem::swap(&mut a, &mut b);
    for _ in 0..3 {
        box_pass_h(&a, &mut b, h, w, r);
        std::mem::swap(&mut a, &mut b);
    }
    transpose(&a, &mut b, h, w);
    pm.data_mut().copy_from_slice(&b);
}

/// Fast blur for big radii: downsample, blur, upsample.
pub fn blur_fast(pm: &Pixmap, radius: f32) -> Pixmap {
    let f = if radius > 24.0 {
        4
    } else if radius > 8.0 {
        2
    } else {
        1
    };
    let small = crate::resize(pm, (pm.width() / f).max(1), (pm.height() / f).max(1));
    let mut small = small;
    blur(&mut small, radius / f as f32);
    crate::resize(&small, pm.width(), pm.height())
}

/// Saturation adjustment on premultiplied data.
pub fn saturate(pm: &mut Pixmap, s: f32) {
    for px in pm.data_mut().as_chunks_mut::<4>().0 {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let a = px[3] as f32;
        px[0] = (l + (r - l) * s).clamp(0.0, a) as u8;
        px[1] = (l + (g - l) * s).clamp(0.0, a) as u8;
        px[2] = (l + (b - l) * s).clamp(0.0, a) as u8;
    }
}
