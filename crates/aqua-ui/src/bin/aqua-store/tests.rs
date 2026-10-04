use crate::conv::*;
use aqua_store::model::{Details, Origin, Package, Ratings, Release};
use aqua_ui::{ntr, tr, trf};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn start_routes() {
    assert_eq!(parse_start(&[]), Route::Home("discover"));
    assert_eq!(parse_start(&s(&["--updates"])), Route::Updates);
    assert_eq!(parse_start(&s(&["--search", "gimp"])), Route::Search("gimp".into()));
    assert_eq!(parse_start(&s(&["appstream://org.gimp.GIMP"])), Route::App("flatpak:org.gimp.GIMP".into()));
    assert_eq!(parse_start(&s(&["appstream:org.gnome.Maps.desktop"])), Route::App("flatpak:org.gnome.Maps".into()));
    assert_eq!(parse_start(&s(&["pacman:firefox"])), Route::App("pacman:firefox".into()));
    assert_eq!(parse_start(&s(&["develop"])), Route::Home("develop"));
    assert_eq!(parse_start(&s(&["categories"])), Route::Categories);
}

#[test]
fn flatpakref() {
    let t = "[Flatpak Ref]\nTitle=GIMP\nName=org.gimp.GIMP\nBranch=stable\n";
    assert_eq!(flatpakref_name(t).as_deref(), Some("org.gimp.GIMP"));
    assert_eq!(flatpakref_name("[Flatpak Ref]\n"), None);
}

#[test]
fn routes_map_to_pages() {
    assert_eq!(Route::nav(6), Route::Categories);
    assert_eq!(Route::nav(7), Route::Updates);
    assert_eq!(Route::nav(1), Route::Home("arcade"));
    assert_eq!(Route::Category("games".into()).nav_index(), 6);
    assert_eq!(Route::App("x".into()).page(), "app");
    assert_eq!(Route::Developer("GNOME".into()).page(), "list");
}

#[test]
fn button_states() {
    let mut p = Package::new(Origin::Flatpak, "org.x.App");
    assert_eq!(state_of(&p, false, None).0, ST_GET);
    p.installed = true;
    assert_eq!(state_of(&p, true, None).0, ST_OPEN);
    assert_eq!(state_of(&p, false, None).0, ST_INSTALLED);
    p.update_version = "2".into();
    assert_eq!(state_of(&p, true, None).0, ST_UPDATE);
    let j = JobState { removing: false, started: false, fraction: -1.0, status: String::new() };
    assert_eq!(state_of(&p, true, Some(&j)).0, ST_QUEUED);
    let j = JobState { started: true, fraction: 0.4, ..j };
    let (st, f, _) = state_of(&p, true, Some(&j));
    assert_eq!((st, f), (ST_WORKING, 0.4));
    let j = JobState { removing: true, ..j };
    assert_eq!(state_of(&p, true, Some(&j)).0, ST_REMOVING);
}

#[test]
fn placeholders() {
    assert_eq!(letter("gimp"), "G");
    assert_eq!(letter("  42 apps"), "4");
    assert_eq!(letter("—"), "?");
    assert_eq!(tint("Firefox"), tint("Firefox"));
    assert_eq!(parse_color("#ff8800"), Some(0xff8800));
    assert_eq!(parse_color("red"), None);
}

#[test]
fn source_labels() {
    let mut p = Package::new(Origin::Flatpak, "a");
    assert_eq!(source_label(&p), "Flathub");
    p.repo = "fedora".into();
    assert_eq!(source_label(&p), "fedora");
    assert_eq!(source_label(&Package::new(Origin::Aur, "yay")), "AUR");
}

#[test]
fn relative_dates() {
    let now = 1_700_000_000;
    assert_eq!(relative(now, now), tr("Today"));
    assert_eq!(relative(now - 86400, now), tr("Yesterday"));
    assert_eq!(relative(now - 3 * 86400, now), ntr("{n} day ago", "{n} days ago", 3));
    assert_eq!(relative(now - 14 * 86400, now), ntr("{n} week ago", "{n} weeks ago", 2));
    assert_eq!(relative(now - 120 * 86400, now), ntr("{n} month ago", "{n} months ago", 4));
    assert_eq!(relative(now - 800 * 86400, now), ntr("{n} year ago", "{n} years ago", 2));
    assert_eq!(date(0), format!("1 {} 1970", tr(aqua_store::units::MONTHS[0])));
}

#[test]
fn info_strip() {
    let mut d = Details { pkg: Package::new(Origin::Flatpak, "org.x.App"), ..Default::default() };
    d.pkg.developer = "X Team".into();
    d.pkg.categories = vec!["Graphics".into()];
    d.pkg.size = Some(767_300_000);
    d.age = Some(12);
    d.languages = vec!["de".into(), "en_GB".into(), "fr".into()];
    let r = Ratings { stars: [0, 0, 1, 2, 7] };
    let c = strip(&d, Some(&r));
    assert_eq!(c[0].label, ntr("{n} RATING", "{n} RATINGS", 10));
    assert_eq!(c[0].value, "4.6");
    assert!(c[0].stars);
    let lang = c.iter().find(|x| x.label == tr("LANGUAGE")).unwrap();
    assert_eq!(lang.value, "EN");
    assert_eq!(lang.sub, trf("+ {n} More", &[("n", &2)]));
    let dev = c.iter().find(|x| x.label == tr("DEVELOPER")).unwrap();
    assert_eq!(dev.link, "developer:X Team");
    let cat = c.iter().find(|x| x.label == tr("CATEGORY")).unwrap();
    assert_eq!(cat.link, "category:graphics");
    assert!(c.iter().any(|x| x.label == tr("SIZE")));
    assert!(strip(&d, None).iter().all(|x| !x.stars));
}

#[test]
fn info_and_links() {
    let mut d = Details { pkg: Package::new(Origin::Pacman, "htop"), ..Default::default() };
    d.pkg.version = "3.3".into();
    d.pkg.installed = true;
    d.pkg.installed_version = "3.2".into();
    d.pkg.license = "GPL-2.0".into();
    d.releases = vec![Release { version: "3.3".into(), timestamp: Some(86400 * 365), notes: vec![] }];
    d.links = vec![aqua_store::model::Link { kind: "bugtracker".into(), url: "https://bugs.example".into() }];
    d.pkg.homepage = "https://htop.dev".into();
    let i = info(&d, "Arch Linux");
    let ver = i.iter().find(|c| c.label == tr("Version")).unwrap();
    assert_eq!(ver.sub, trf("Installed: {v}", &[("v", &"3.2")]));
    assert_eq!(
        i.iter().find(|c| c.label == tr("Compatibility")).unwrap().value,
        trf("Requires {d}", &[("d", &"Arch Linux")])
    );
    let l = links(&d);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].link, "https://htop.dev");
    assert_eq!(l[1].value, tr("Report a Problem"));
}

#[test]
fn filters_and_urls() {
    let mut p = Package::new(Origin::Aur, "paru-bin");
    p.title = "Paru".into();
    assert!(matches_filter(&p, "all", ""));
    assert!(matches_filter(&p, "aur", "par"));
    assert!(!matches_filter(&p, "native", ""));
    assert!(!matches_filter(&p, "flatpak", ""));
    assert!(!matches_filter(&p, "all", "zzz"));
    assert_eq!(web_url(&p), "https://aur.archlinux.org/packages/paru-bin");
    assert_eq!(web_url(&Package::new(Origin::Flatpak, "org.x.A")), "https://flathub.org/apps/org.x.A");
}

#[test]
fn misc() {
    assert_eq!(perm_glyph("network"), "network");
    assert_eq!(perm_glyph("filesystem-home"), "folder");
    assert_eq!(perm_glyph("sockets-pulseaudio"), "speaker");
    assert!((shot_ratio(1600, 900) - 1.777).abs() < 0.01);
    assert_eq!(shot_ratio(0, 0), 1.6);
    assert_eq!(shot_ratio(100, 1000), 0.45);
    assert_eq!(age_label(None), "4+");
    assert_eq!(history_action("remove"), tr("Removed"));
    assert_eq!(size_text(None), "");
}

#[test]
fn update_schedule() {
    assert!(crate::notify::due(0, 100));
    assert!(!crate::notify::due(1000, 1000 + 3600));
    assert!(crate::notify::due(1000, 1000 + 7 * 3600));
    assert!(crate::notify::due(5000, 10));
    assert!(crate::notify::autostart_entry(false).contains("Hidden=true"));
    assert!(crate::notify::autostart_entry(true).contains("Exec=aqua-store --check-updates"));
}

#[test]
fn home_shelves_offline() {
    let dir = std::env::temp_dir().join(format!("aqua-store-ui-test-{}", std::process::id()));
    let http = std::sync::Arc::new(aqua_store::http::Http::new(dir));
    let prefs = aqua_store::prefs::Prefs { use_flathub: false, ..Default::default() };
    let env = aqua_store::system::Env::default();
    let run = std::sync::Arc::new(aqua_store::runner::fake::FakeRunner::default());
    let store = aqua_store::store::Store::with_env(run, http, prefs, env);
    let h = crate::app::build_home(&store, "discover");
    assert!(h.heroes.is_empty());
    assert!(!h.error.is_empty() || !h.shelves.is_empty());
}
