use aqua_gfx::{canvas::marquee_offset, rgba, Canvas, Pixmap, Rect};

#[test]
fn marquee_stays_still_when_text_fits_or_during_initial_pause() {
    assert_eq!(marquee_offset(80.0, 100.0, 1000.0), None);
    assert_eq!(marquee_offset(100.0, 100.0, 1.0), None);
    assert_eq!(marquee_offset(200.0, 100.0, -1.0), Some(0.0));
    assert_eq!(marquee_offset(200.0, 100.0, 2.0), Some(0.0));
}

#[test]
fn marquee_scrolls_at_a_bounded_speed_and_repeats() {
    let travel = 200.0 + aqua_gfx::canvas::MARQUEE_GAP;
    let cycle = 2.2 + travel / 28.0;
    assert!((marquee_offset(200.0, 100.0, 3.2).unwrap() - 28.0).abs() < 0.001);
    assert!(marquee_offset(200.0, 100.0, cycle + 0.5).unwrap().abs() < 0.001);
    for n in 0..1000 {
        let offset = marquee_offset(200.0, 100.0, n as f32 / 10.0).unwrap();
        assert!(offset.is_finite() && (0.0..=travel).contains(&offset));
    }
}

#[test]
fn rounded_artwork_preserves_transparent_corners_at_both_scales() {
    let mut source = Pixmap::new(8, 8).unwrap();
    source.fill(rgba(255, 0, 0, 1.0));
    for scale in [1.0, 2.0] {
        let mut canvas = Canvas::new(32.0, 32.0, scale);
        canvas.draw_pixmap_rounded(&source, Rect::new(4.0, 4.0, 24.0, 24.0), 8.0);
        let pixel = |x: u32, y: u32| canvas.pm.pixel((x as f32 * scale) as u32, (y as f32 * scale) as u32).unwrap();
        assert_eq!(pixel(1, 1).alpha(), 0);
        assert_eq!(pixel(4, 4).alpha(), 0);
        assert_eq!(pixel(16, 16).alpha(), 255);
        assert!(pixel(16, 16).red() > 250);
    }
}
