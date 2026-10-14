use aqua_config::Config;

#[test]
fn desktop_settings_are_backward_compatible_with_an_existing_config() {
    let cfg = Config::from_toml("clock_24h = false\n").unwrap();
    assert!(cfg.desktop_icons);
    assert!(cfg.desktop_snap);
    assert!(cfg.wallpaper_tinting);
    assert!(!cfg.reduce_sidebar_transparency);
    assert!(!cfg.clock_24h);
}

#[test]
fn desktop_and_appearance_choices_survive_a_roundtrip() {
    let cfg = Config::from_toml(
        "desktop_icons = false\ndesktop_snap = false\nwallpaper_tinting = false\nreduce_sidebar_transparency = true\n",
    )
    .unwrap();
    let saved = toml::to_string_pretty(&cfg).unwrap();
    let restored = Config::from_toml(&saved).unwrap();
    assert!(!restored.desktop_icons);
    assert!(!restored.desktop_snap);
    assert!(!restored.wallpaper_tinting);
    assert!(restored.reduce_sidebar_transparency);
}
