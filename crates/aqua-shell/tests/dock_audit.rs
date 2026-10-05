//! Dock audit against the apps installed on this machine (run with `--ignored`):
//! every app, opened with the app ids real toolkits use, must land on exactly one Dock
//! icon that belongs to it (and shows its icon), and distinct apps must not merge.
use aqua_shell::{dock, Shell, WindowInfo};
use std::path::PathBuf;

fn shell() -> Shell {
    let dir = std::env::temp_dir().join(format!("aqua-dock-audit-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::env::set_var("AQUA_CONFIG", dir.join("config/config.toml"));
    std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
    let cfg = aqua_config::Config {
        fetch_icons: false,
        icon_cache: Some(dir.join("icons")),
        font_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")),
        ..Default::default()
    };
    let wp = aqua_wallpaper::generate(320, 200, aqua_wallpaper::Palette::Tahoe);
    Shell::new(cfg, 1280.0, 800.0, 1.0, &wp)
}

fn win(id: u64, app: &str) -> WindowInfo {
    WindowInfo { id, app_id: app.into(), title: app.into(), focused: false, minimized: false }
}

#[test]
#[ignore]
fn installed_apps_map_to_their_own_dock_icon() {
    let mut sh = shell();
    let apps = sh.apps.clone();
    let mut problems = vec![];
    let mut checked = 0;
    for app in &apps {
        let stem = app.id.trim_end_matches(".desktop").to_string();
        let mut ids = vec![stem.clone()];
        if let Some(c) = &app.wm_class {
            ids.push(c.clone());
        }
        for id in ids {
            sh.set_windows(vec![win(1, &id)]);
            let items = dock::visible_items(&sh);
            let hits: Vec<_> = items.iter().filter(|i| i.matches(&id)).collect();
            checked += 1;
            match hits.as_slice() {
                [] => problems.push(format!("{} ({id}): no Dock icon", app.name)),
                [it] => {
                    let owner = aqua_apps::match_app_id(&apps, &id).map(|a| a.id.clone());
                    if owner.as_deref() != Some(app.id.as_str()) {
                        problems.push(format!("{} ({id}): window attributed to {:?}", app.name, owner));
                    } else if !it.pinned && it.icon.icon != app.icon {
                        problems.push(format!("{} ({id}): icon {:?} instead of {:?}", app.name, it.icon.icon, app.icon));
                    } else if it.pinned && !it.matches(&stem) && !app.wm_class.as_deref().is_some_and(|c| it.matches(c)) {
                        problems.push(format!("{} ({id}): grabbed by pinned {:?}", app.name, it.app));
                    }
                }
                many => problems.push(format!(
                    "{} ({id}): {} icons {:?}",
                    app.name,
                    many.len(),
                    many.iter().map(|i| i.app.clone()).collect::<Vec<_>>()
                )),
            }
        }
    }
    // pairs of distinct apps running together must give two icons
    let mut merged = vec![];
    for (i, a) in apps.iter().enumerate() {
        for b in apps.iter().skip(i + 1) {
            let (ia, ib) = (a.id.trim_end_matches(".desktop"), b.id.trim_end_matches(".desktop"));
            sh.set_windows(vec![win(1, ia), win(2, ib)]);
            let items = dock::visible_items(&sh);
            let both = items.iter().filter(|it| it.matches(ia) && it.matches(ib)).count();
            if both > 0 {
                merged.push(format!("{} + {}", a.name, b.name));
            }
        }
    }
    println!("checked {checked} app ids of {} apps", apps.len());
    for p in &problems {
        println!("PROBLEM {p}");
    }
    for m in &merged {
        println!("MERGED {m}");
    }
    assert!(problems.is_empty() && merged.is_empty(), "{} problems, {} merges", problems.len(), merged.len());
}
