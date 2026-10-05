//! Unit tests for the pure helpers of System Settings.
use super::*;

fn edid_base() -> Vec<u8> {
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    let m: u16 = ((b'D' - b'A' + 1) as u16) << 10 | ((b'E' - b'A' + 1) as u16) << 5 | (b'L' - b'A' + 1) as u16;
    e[8..10].copy_from_slice(&m.to_be_bytes());
    for i in 0..8 {
        e[38 + i * 2] = 1;
        e[39 + i * 2] = 1;
    }
    e
}

#[test]
fn edid_monitor_name() {
    let mut e = edid_base();
    assert_eq!(edid_name(&e), Some(trf("{vendor} display", &[("vendor", &"DEL")])));
    let d = 54 + 18;
    e[d + 3] = 0xFC;
    e[d + 5..d + 5 + 8].copy_from_slice(b"U2720Q\n ");
    assert_eq!(edid_name(&e).as_deref(), Some("U2720Q"));
    assert_eq!(edid_name(&e[..100]), None);
}

#[test]
fn edid_established_and_detailed_modes() {
    let mut e = edid_base();
    e[35] = 0x01;
    e[36] = 0x08 | 0x10;
    let d = &mut e[54..72];
    let (ha, hb, va, vb) = (1920u32, 280u32, 1080u32, 45u32);
    let clk = (148_500_000u32 / 10_000) as u16;
    d[0..2].copy_from_slice(&clk.to_le_bytes());
    d[2] = (ha & 0xff) as u8;
    d[3] = (hb & 0xff) as u8;
    d[4] = (((ha >> 8) << 4) | (hb >> 8)) as u8;
    d[5] = (va & 0xff) as u8;
    d[6] = (vb & 0xff) as u8;
    d[7] = (((va >> 8) << 4) | (vb >> 8)) as u8;
    let modes = edid_modes(&e);
    assert!(modes.contains(&(800, 600, 60.0)), "{modes:?}");
    assert!(modes.contains(&(1024, 768, 60.0)), "{modes:?}");
    assert!(!modes.iter().any(|m| m.2 == 87.0), "interlaced 1024x768@87 is skipped: {modes:?}");
    assert!(modes.contains(&(1920, 1080, 60.0)), "{modes:?}");
    assert!(edid_modes(&[0u8; 128]).is_empty());
}

#[test]
fn mode_helpers() {
    let modes = parse_modes(Some("1920x1080@60.00,1920x1080@144,1280x720@60,bad,1x1@x"));
    assert_eq!(modes.len(), 3);
    assert_eq!(rates_for(&modes, "1920x1080"), vec![144.0, 60.0]);
    let (res, ri, rates, rr) = mode_lists(&modes, "1920x1080", 143.9);
    assert_eq!(res, vec!["1920x1080", "1280x720"]);
    assert_eq!((ri, rr), (0, 0));
    assert_eq!(rates.len(), 2);
    assert_eq!(mode_lists(&modes, "800x600", 60.0).1, -1);
    assert_eq!(fmt_hz(60.0), "60");
    assert_eq!(fmt_hz(59.94), "59.94");
    assert!(parse_modes(None).is_empty());
}

#[test]
fn complete_modes_adds_current() {
    let mut m = vec![("1280x720".to_string(), 60.0)];
    complete_modes("TEST-1", "1920x1080", 75.0, &mut m);
    assert_eq!(m[0], ("1920x1080".to_string(), 75.0));
    let mut m = vec![("1920x1080".to_string(), 60.0)];
    complete_modes("TEST-1", "1920x1080", 120.0, &mut m);
    assert!(m.contains(&("1920x1080".to_string(), 120.0)));
}

#[test]
fn slider_mappings() {
    assert_eq!(idle_idx(0), 0);
    assert_eq!(IDLE[idle_idx(300) as usize], 300);
    assert_eq!(IDLE[idle_idx(310) as usize], 300);
    assert_eq!(IDLE[idle_idx(100_000) as usize], 3600);
    assert_eq!(lerp_inv(5.0, 0.0, 10.0), 0.5);
    assert_eq!(lerp_inv(-5.0, 0.0, 10.0), 0.0);
    assert_eq!(lerp(0.25, 0.0, 8.0), 2.0);
    assert_eq!(lerp(2.0, 0.0, 8.0), 8.0);
}

#[test]
fn shortcut_key_names() {
    assert_eq!(mods_prefix(true, false, true, true), "ctrl+shift+super+");
    assert_eq!(cyr_to_lat('ф'), Some('a'));
    assert_eq!(cyr_to_lat('ё'), Some('`'));
    assert_eq!(key_name('Ф').as_deref(), Some("a"));
    assert_eq!(key_name('!').as_deref(), Some("1"));
    assert_eq!(key_name('?').as_deref(), Some("slash"));
    assert_eq!(key_name(' ').as_deref(), Some("space"));
    assert_eq!(key_name('€'), None);
}

#[test]
fn shortcut_recording() {
    assert!(matches!(record_key("", false, false, false, true), Rec::Pending(p) if p == "super+"));
    assert!(matches!(record_key("q", false, false, false, true), Rec::Chord(c) if c == "super+q"));
    assert!(matches!(record_key("й", true, false, true, false), Rec::Chord(c) if c == "ctrl+shift+q"));
    assert!(matches!(record_key("q", false, false, false, false), Rec::Invalid(_)));
    let esc = char::from(slint::platform::Key::Escape).to_string();
    assert!(matches!(record_key(&esc, false, false, false, false), Rec::Cancel));
    let f5 = char::from(slint::platform::Key::F5).to_string();
    assert!(matches!(record_key(&f5, false, false, false, false), Rec::Chord(c) if c == "F5"));
    let bs = char::from(slint::platform::Key::Backspace).to_string();
    assert!(matches!(record_key(&bs, false, false, false, false), Rec::Clear));
}

#[test]
fn glass_sliders_round_trip() {
    let d = aqua_config::GlassStyle::default();
    for (get, set) in GLASS_KNOBS.iter() {
        let v0 = get(&d);
        assert!(v0 > 0.0 && v0 < 1.0, "the standard glass sits inside every slider's range");
        for v in [0.0, 0.3, 1.0] {
            let mut g = d;
            set(&mut g, v);
            assert!((get(&g) - v).abs() < 1e-4);
        }
    }
}
