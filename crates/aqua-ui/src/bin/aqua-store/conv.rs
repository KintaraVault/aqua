use aqua_store::model::{Details, Origin, Package, Ratings};
use aqua_store::sections;
use aqua_store::units;
use aqua_ui::{ntr, tr, trf};

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home(&'static str),
    Categories,
    Updates,
    Account,
    Feed { id: String, title: String },
    Category(String),
    Developer(String),
    Search(String),
    App(String),
}

pub const NAV: [&str; 8] = ["discover", "arcade", "create", "work", "play", "develop", "categories", "updates"];

impl Route {
    pub fn nav(i: usize) -> Route {
        match NAV.get(i).copied().unwrap_or("discover") {
            "categories" => Route::Categories,
            "updates" => Route::Updates,
            s => Route::Home(s),
        }
    }

    pub fn page(&self) -> &'static str {
        match self {
            Route::Home(s) => s,
            Route::Categories => "categories",
            Route::Updates => "updates",
            Route::Account => "account",
            Route::Feed { .. } | Route::Category(_) | Route::Developer(_) => "list",
            Route::Search(_) => "search",
            Route::App(_) => "app",
        }
    }

    pub fn nav_index(&self) -> i32 {
        match self {
            Route::Home(s) => NAV.iter().position(|n| n == s).map(|i| i as i32).unwrap_or(-1),
            Route::Categories | Route::Category(_) => 6,
            Route::Updates => 7,
            _ => -1,
        }
    }
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub fn flatpakref_name(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("Name=").map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

pub fn parse_start(args: &[String]) -> Route {
    if args.iter().any(|a| a == "--updates") {
        return Route::Updates;
    }
    if args.iter().any(|a| a == "--account") {
        return Route::Account;
    }
    if let Some(q) = arg_value(args, "--search") {
        return Route::Search(q.to_string());
    }
    if let Some(k) = arg_value(args, "--app") {
        return Route::App(k.to_string());
    }
    if let Some(c) = arg_value(args, "--category") {
        return Route::Category(c.to_string());
    }
    for a in args.iter().filter(|a| !a.starts_with("--")) {
        if let Some(id) = a.strip_prefix("appstream://").or_else(|| a.strip_prefix("appstream:")) {
            let id = id.trim_matches('/').trim_end_matches(".desktop");
            if !id.is_empty() {
                return Route::App(format!("flatpak:{id}"));
            }
        }
        if let Some(q) = a.strip_prefix("search:") {
            return Route::Search(q.to_string());
        }
        if a.ends_with(".flatpakref") {
            let text = std::fs::read_to_string(a).unwrap_or_default();
            if let Some(n) = flatpakref_name(&text) {
                return Route::App(format!("flatpak:{n}"));
            }
        }
        if aqua_store::model::parse_key(a).is_some() {
            return Route::App(a.clone());
        }
        if NAV.contains(&a.as_str()) {
            return Route::nav(NAV.iter().position(|n| n == a).unwrap_or(0));
        }
    }
    Route::Home("discover")
}

pub const ST_GET: i32 = 0;
pub const ST_OPEN: i32 = 1;
pub const ST_UPDATE: i32 = 2;
pub const ST_QUEUED: i32 = 3;
pub const ST_WORKING: i32 = 4;
pub const ST_INSTALLED: i32 = 5;
pub const ST_REMOVING: i32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct JobState {
    pub removing: bool,
    pub started: bool,
    pub fraction: f32,
    pub status: String,
}

pub fn state_of(p: &Package, openable: bool, job: Option<&JobState>) -> (i32, f32, String) {
    if let Some(j) = job {
        if j.removing {
            return (ST_REMOVING, -1.0, j.status.clone());
        }
        if !j.started {
            return (ST_QUEUED, -1.0, tr("Waiting…").to_string());
        }
        return (ST_WORKING, j.fraction, j.status.clone());
    }
    if !p.installed {
        return (ST_GET, 0.0, String::new());
    }
    if p.has_update() {
        return (ST_UPDATE, 0.0, String::new());
    }
    if openable {
        (ST_OPEN, 0.0, String::new())
    } else {
        (ST_INSTALLED, 0.0, String::new())
    }
}

pub fn letter(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

const TINTS: [u32; 10] =
    [0x0a84ff, 0x30b350, 0xff9f0a, 0xff453a, 0xbf5af2, 0x5e5ce6, 0x32ade6, 0xff375f, 0x64d2a0, 0x8e8e93];

pub fn tint(name: &str) -> u32 {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    TINTS[(h % TINTS.len() as u32) as usize]
}

pub fn parse_color(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

static NATIVE_LABEL: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

pub fn set_native_label(label: &str) {
    if let Ok(mut l) = NATIVE_LABEL.write() {
        *l = label.to_string();
    }
}

pub fn source_label(p: &Package) -> String {
    match p.origin() {
        Origin::Flatpak if p.repo.is_empty() || p.repo == "flathub" => "Flathub".into(),
        Origin::Flatpak => p.repo.clone(),
        Origin::Aur => "AUR".into(),
        o => NATIVE_LABEL
            .read()
            .ok()
            .filter(|l| !l.is_empty())
            .map(|l| l.clone())
            .unwrap_or_else(|| o.label().to_string()),
    }
}

pub fn relative(ts: i64, now: i64) -> String {
    let d = (now - ts).max(0) / 86400;
    match d {
        0 => tr("Today").to_string(),
        1 => tr("Yesterday").to_string(),
        2..=6 => ntr("{n} day ago", "{n} days ago", d),
        7..=30 => ntr("{n} week ago", "{n} weeks ago", d / 7),
        31..=364 => ntr("{n} month ago", "{n} months ago", d / 30),
        _ => ntr("{n} year ago", "{n} years ago", d / 365),
    }
}

pub fn date(ts: i64) -> String {
    let (y, m, d) = units::civil_from_days(ts.div_euclid(86400));
    format!("{d} {} {y}", tr(units::MONTHS[(m.clamp(1, 12) - 1) as usize]))
}

pub fn age_label(age: Option<u8>) -> String {
    match age {
        Some(a) => aqua_store::appstream::store_age(a).to_string(),
        None => "4+".into(),
    }
}

pub fn size_parts(b: u64) -> (String, String) {
    let (n, u) = units::format_size(b);
    (n, tr(u).to_string())
}

pub fn size_text(b: Option<u64>) -> String {
    match b {
        Some(b) if b > 0 => {
            let (n, u) = size_parts(b);
            format!("{n} {u}")
        }
        _ => String::new(),
    }
}

pub fn category_of(p: &Package) -> (String, String) {
    let title = sections::primary_category(&p.categories);
    let glyph = sections::CATEGORIES
        .iter()
        .find(|c| c.title == title || sections::category_title(c.feed_cat()) == title)
        .map(|c| c.glyph)
        .unwrap_or("categories");
    (tr(&title).to_string(), glyph.to_string())
}

pub fn category_id(p: &Package) -> Option<&'static str> {
    let title = sections::primary_category(&p.categories);
    sections::CATEGORIES
        .iter()
        .find(|c| c.feed_sub().is_none() && sections::category_title(c.feed_cat()) == title)
        .map(|c| c.id)
}

pub fn row_detail(p: &Package) -> String {
    let mut parts = vec![];
    let v = if p.installed && !p.installed_version.is_empty() { &p.installed_version } else { &p.version };
    if !v.is_empty() {
        parts.push(trf("Version {v}", &[("v", v)]));
    }
    parts.push(source_label(p));
    let s = size_text(p.size);
    if !s.is_empty() {
        parts.push(s);
    }
    parts.join("  ·  ")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub label: String,
    pub value: String,
    pub sub: String,
    pub link: String,
    pub glyph: String,
    pub stars: bool,
}

impl Cell {
    fn new(label: &str, value: String, sub: String) -> Cell {
        Cell { label: label.to_string(), value, sub, ..Default::default() }
    }
}

pub fn strip(d: &Details, ratings: Option<&Ratings>) -> Vec<Cell> {
    let p = &d.pkg;
    let mut v = vec![];
    if let Some(r) = ratings.filter(|r| r.total() > 0) {
        let mut c = Cell::new(
            &ntr("{n} RATING", "{n} RATINGS", r.total() as i64)
                .replace(&r.total().to_string(), &units::compact_count(r.total())),
            format!("{:.1}", r.average()),
            String::new(),
        );
        c.stars = true;
        c.link = "reviews".into();
        v.push(c);
    }
    v.push(Cell::new(tr("AGES"), age_label(d.age), tr("Years").to_string()));
    if !p.categories.is_empty() {
        let (title, glyph) = category_of(p);
        let mut c = Cell::new(tr("CATEGORY"), String::new(), title);
        c.glyph = glyph;
        if let Some(id) = category_id(p) {
            c.link = format!("category:{id}");
        }
        v.push(c);
    }
    let dev = if !p.developer.is_empty() { p.developer.clone() } else { d.maintainer.clone() };
    if !dev.is_empty() {
        let mut c = Cell::new(tr("DEVELOPER"), String::new(), dev.clone());
        c.glyph = "person".into();
        if p.origin() == Origin::Flatpak {
            c.link = format!("developer:{dev}");
        }
        v.push(c);
    }
    if !d.languages.is_empty() {
        let main =
            d.languages.iter().find(|l| l.starts_with("en")).or(d.languages.first()).cloned().unwrap_or_default();
        let code: String = main.split(['_', '-', '@']).next().unwrap_or("").to_uppercase();
        let more = d.languages.len().saturating_sub(1);
        let sub = if more > 0 { trf("+ {n} More", &[("n", &more)]) } else { String::new() };
        v.push(Cell::new(tr("LANGUAGE"), code, sub));
    }
    if let Some(n) = p.installs.filter(|n| *n > 0) {
        v.push(Cell::new(tr("DOWNLOADS"), units::compact_count(n), tr("Installs").to_string()));
    }
    let size = p.size.or(p.download_size).filter(|s| *s > 0);
    if let Some(s) = size {
        let (n, u) = size_parts(s);
        v.push(Cell::new(tr("SIZE"), n, u));
    }
    v
}

pub fn info(d: &Details, distro: &str) -> Vec<Cell> {
    let p = &d.pkg;
    let mut v = vec![];
    let dev = if !p.developer.is_empty() { p.developer.clone() } else { d.maintainer.clone() };
    if !dev.is_empty() {
        v.push(Cell::new(tr("Developer"), dev, String::new()));
    }
    if !d.packager.is_empty() {
        v.push(Cell::new(tr("Packager"), d.packager.clone(), String::new()));
    }
    let size = size_text(p.size);
    if !size.is_empty() {
        let dl = size_text(p.download_size);
        v.push(Cell::new(tr("Size"), size, if dl.is_empty() { dl } else { trf("Download {s}", &[("s", &dl)]) }));
    } else if p.download_size.is_some() {
        v.push(Cell::new(tr("Download Size"), size_text(p.download_size), String::new()));
    }
    if !p.categories.is_empty() {
        v.push(Cell::new(tr("Category"), category_of(p).0, String::new()));
    }
    let compat = match p.origin() {
        Origin::Flatpak => {
            if d.runtime.is_empty() {
                tr("Any Linux distribution").to_string()
            } else {
                d.runtime.split('/').next().unwrap_or(&d.runtime).to_string()
            }
        }
        _ => trf("Requires {d}", &[("d", &distro)]),
    };
    v.push(Cell::new(
        tr("Compatibility"),
        compat,
        if p.origin() == Origin::Flatpak { tr("Runs in a sandbox").to_string() } else { String::new() },
    ));
    if !d.languages.is_empty() {
        let n = d.languages.len();
        v.push(Cell::new(tr("Languages"), ntr("{n} language", "{n} languages", n as i64), String::new()));
    }
    v.push(Cell::new(tr("Age Rating"), age_label(d.age), d.content.first().cloned().unwrap_or_default()));
    if !p.license.is_empty() {
        let free = !p.license.to_lowercase().contains("proprietary") && !p.license.starts_with("LicenseRef");
        v.push(Cell::new(
            tr("License"),
            p.license.replace("LicenseRef-proprietary", tr("Proprietary")),
            if free { tr("Free software").to_string() } else { String::new() },
        ));
    }
    v.push(Cell::new(
        tr("Source"),
        p.source_label(),
        p.scope
            .map(|s| {
                if s == aqua_store::model::Scope::System { tr("All users") } else { tr("Current user") }.to_string()
            })
            .unwrap_or_default(),
    ));
    let ver = if !p.version.is_empty() {
        p.version.clone()
    } else {
        d.releases.first().map(|r| r.version.clone()).unwrap_or_default()
    };
    if !ver.is_empty() {
        let sub = if p.installed && !p.installed_version.is_empty() && p.installed_version != ver {
            trf("Installed: {v}", &[("v", &p.installed_version)])
        } else {
            String::new()
        };
        v.push(Cell::new(tr("Version"), ver, sub));
    }
    if let Some(t) = d.updated.or_else(|| d.releases.first().and_then(|r| r.timestamp)) {
        v.push(Cell::new(tr("Updated"), date(t), String::new()));
    }
    if p.votes > 0 {
        v.push(Cell::new(tr("Votes"), p.votes.to_string(), format!("{:.2}", p.popularity)));
    }
    if p.out_of_date {
        v.push(Cell::new(tr("Status"), tr("Flagged out of date").to_string(), String::new()));
    }
    v
}

pub fn links(d: &Details) -> Vec<Cell> {
    const KINDS: [(&str, &str, &str); 7] = [
        ("homepage", "Developer Website", "globe"),
        ("help", "Help", "info"),
        ("bugtracker", "Report a Problem", "flag"),
        ("donation", "Donate", "gift"),
        ("translate", "Translate", "bubbles"),
        ("vcs-browser", "Source Code", "link"),
        ("contact", "Contact", "link"),
    ];
    let mut v = vec![];
    for (k, label, g) in KINDS {
        let url = d
            .link(k)
            .map(str::to_string)
            .or_else(|| (k == "homepage" && !d.pkg.homepage.is_empty()).then(|| d.pkg.homepage.clone()));
        if let Some(u) = url.filter(|u| u.starts_with("http")) {
            v.push(Cell {
                label: k.into(),
                value: tr(label).to_string(),
                link: u,
                glyph: g.into(),
                ..Default::default()
            });
        }
    }
    v
}

pub fn perm_glyph(id: &str) -> &'static str {
    let id = id.to_lowercase();
    if id.contains("network") {
        "network"
    } else if id.contains("home")
        || id.contains("host")
        || id.contains("file")
        || id.contains("download")
        || id.contains("xdg")
    {
        "folder"
    } else if id.contains("device") || id.contains("usb") || id.contains("dri") {
        "usb"
    } else if id.contains("sound") || id.contains("pulse") || id.contains("audio") {
        "speaker"
    } else if id.contains("x11") || id.contains("wayland") || id.contains("display") {
        "display"
    } else if id.contains("notif") {
        "bell"
    } else if id.contains("bus") || id.contains("talk") {
        "bubbles"
    } else {
        "lockshield"
    }
}

pub fn notes_text(d: &Details) -> String {
    d.releases.first().map(|r| aqua_store::model::blocks_to_text(&r.notes)).unwrap_or_default()
}

pub fn shot_ratio(w: u32, h: u32) -> f32 {
    if w == 0 || h == 0 {
        return 1.6;
    }
    (w as f32 / h as f32).clamp(0.45, 2.2)
}

pub fn history_action(a: &str) -> String {
    match a {
        "install" => tr("Installed"),
        "remove" => tr("Removed"),
        "update" => tr("Updated"),
        "setup" => tr("Set Up"),
        other => other,
    }
    .to_string()
}

pub fn matches_filter(p: &Package, filter: &str, query: &str) -> bool {
    let ok = match filter {
        "flatpak" => p.origin() == Origin::Flatpak,
        "aur" => p.origin() == Origin::Aur,
        "native" => p.origin().is_native() && p.origin() != Origin::Aur,
        _ => true,
    };
    if !ok {
        return false;
    }
    let q = query.trim().to_lowercase();
    q.is_empty()
        || p.display_name().to_lowercase().contains(&q)
        || p.name.to_lowercase().contains(&q)
        || p.summary.to_lowercase().contains(&q)
}

pub fn web_url(p: &Package) -> String {
    match p.origin() {
        Origin::Flatpak if p.repo.is_empty() || p.repo == "flathub" => aqua_store::flathub::Flathub::web_url(&p.name),
        Origin::Aur => format!("https://aur.archlinux.org/packages/{}", p.name),
        Origin::Pacman => format!("https://archlinux.org/packages/?q={}", aqua_store::http::urlencode(&p.name)),
        Origin::Apt => format!("https://packages.debian.org/search?keywords={}", aqua_store::http::urlencode(&p.name)),
        Origin::Dnf => {
            format!("https://packages.fedoraproject.org/search?query={}", aqua_store::http::urlencode(&p.name))
        }
        _ => p.homepage.clone(),
    }
}

pub fn requirement_title(id: &str) -> String {
    match id {
        "polkit" => tr("Administrator Access Needed"),
        "flatpak" => tr("Flatpak Is Needed"),
        "aur" => tr("An AUR Helper Is Needed"),
        _ => tr("Additional Software Is Needed"),
    }
    .to_string()
}

pub fn tr_status(s: &str) -> String {
    let t = tr(s);
    if t != s {
        return t.to_string();
    }
    for p in ["Installing", "Removing", "Building", "Downloading", "Updating"] {
        if let Some(rest) = s.strip_prefix(p).and_then(|r| r.strip_prefix(' ')) {
            return trf(&format!("{p} {{n}}"), &[("n", &rest)]);
        }
    }
    s.to_string()
}
