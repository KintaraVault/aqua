use crate::markup;
use crate::model::{Details, Icon, Link, Origin, Package, Release, Screenshot};
use crate::units::parse_date;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Component {
    pub id: String,
    pub pkgname: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub icon: Icon,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub urls: Vec<(String, String)>,
    pub developer: String,
    pub license: String,
    pub desktop_id: String,
    pub screenshots: Vec<Screenshot>,
    pub releases: Vec<Release>,
    pub age: Option<u8>,
    pub flatpak_remote: String,
}

impl Component {
    pub fn is_flatpak(&self) -> bool {
        !self.flatpak_remote.is_empty()
    }

    pub fn package(&self, native: Option<Origin>) -> Option<Package> {
        let (origin, name) = if self.is_flatpak() {
            (Origin::Flatpak, self.id.trim_end_matches(".desktop").to_string())
        } else {
            (native?, self.pkgname.clone())
        };
        if name.is_empty() {
            return None;
        }
        let mut p = Package::new(origin, &name);
        p.appstream_id = self.id.clone();
        p.title = if self.name.is_empty() { name.clone() } else { self.name.clone() };
        p.summary = self.summary.clone();
        p.icon = self.icon.clone();
        p.categories = self.categories.clone();
        p.keywords = self.keywords.clone();
        p.developer = self.developer.clone();
        p.license = self.license.clone();
        p.desktop_id = self.desktop_id.clone();
        p.homepage = self.urls.iter().find(|u| u.0 == "homepage").map(|u| u.1.clone()).unwrap_or_default();
        p.repo = if self.is_flatpak() { self.flatpak_remote.clone() } else { String::new() };
        p.version = self.releases.first().map(|r| r.version.clone()).unwrap_or_default();
        p.is_app = true;
        Some(p)
    }

    pub fn details(&self, native: Option<Origin>) -> Option<Details> {
        let pkg = self.package(native)?;
        Some(Details {
            pkg,
            description: markup::parse(&self.description),
            screenshots: self.screenshots.clone(),
            releases: self.releases.clone(),
            links: self.urls.iter().map(|(k, u)| Link { kind: k.clone(), url: u.clone() }).collect(),
            age: self.age,
            updated: self.releases.first().and_then(|r| r.timestamp),
            ..Default::default()
        })
    }
}

const OARS: &[(&str, [u8; 4])] = &[
    ("violence-cartoon", [0, 3, 4, 6]),
    ("violence-fantasy", [0, 3, 7, 8]),
    ("violence-realistic", [0, 4, 9, 14]),
    ("violence-bloodshed", [0, 9, 11, 18]),
    ("violence-sexual", [0, 18, 18, 18]),
    ("violence-desecration", [0, 3, 7, 13]),
    ("violence-slavery", [0, 3, 13, 15]),
    ("violence-worship", [0, 3, 13, 15]),
    ("drugs-alcohol", [0, 11, 13, 16]),
    ("drugs-narcotics", [0, 12, 14, 17]),
    ("drugs-tobacco", [0, 10, 13, 13]),
    ("sex-nudity", [0, 12, 14, 14]),
    ("sex-themes", [0, 13, 14, 15]),
    ("sex-homosexuality", [0, 13, 14, 15]),
    ("sex-prostitution", [0, 12, 14, 18]),
    ("sex-adultery", [0, 8, 10, 18]),
    ("sex-appearance", [0, 10, 10, 15]),
    ("language-profanity", [0, 8, 11, 14]),
    ("language-humor", [0, 3, 8, 14]),
    ("language-discrimination", [0, 9, 10, 11]),
    ("social-chat", [0, 4, 10, 13]),
    ("social-info", [0, 0, 13, 13]),
    ("social-audio", [0, 15, 15, 15]),
    ("social-location", [0, 13, 13, 13]),
    ("social-contacts", [0, 12, 12, 12]),
    ("money-purchasing", [0, 12, 14, 18]),
    ("money-gambling", [0, 7, 14, 18]),
];

pub fn oars_age(attrs: &[(String, String)]) -> u8 {
    attrs
        .iter()
        .filter_map(|(id, level)| {
            let row = OARS.iter().find(|r| r.0 == id)?;
            let i = match level.as_str() {
                "mild" => 1,
                "moderate" => 2,
                "intense" => 3,
                _ => 0,
            };
            Some(row.1[i])
        })
        .max()
        .unwrap_or(0)
}

pub fn store_age(min_age: u8) -> &'static str {
    match min_age {
        0..=4 => "4+",
        5..=9 => "9+",
        10..=13 => "13+",
        14..=16 => "16+",
        _ => "18+",
    }
}

fn lang_ok(n: &roxmltree::Node, lang: &str) -> u8 {
    match n.attribute(("http://www.w3.org/XML/1998/namespace", "lang")) {
        None => 1,
        Some(l) if !lang.is_empty() && (l == lang || l.split(['_', '-']).next() == Some(lang)) => 2,
        Some(_) => 0,
    }
}

fn pick_text(node: roxmltree::Node, tag: &str, lang: &str) -> String {
    let mut best = (0u8, String::new());
    for c in node.children().filter(|c| c.has_tag_name(tag)) {
        let s = lang_ok(&c, lang);
        if s > best.0 {
            best = (s, c.text().unwrap_or("").trim().to_string());
        }
    }
    best.1
}

fn markup_of(node: roxmltree::Node) -> String {
    fn walk(n: roxmltree::Node, out: &mut String) {
        for c in n.children() {
            if c.is_text() {
                out.push_str(&c.text().unwrap_or("").replace('&', "&amp;").replace('<', "&lt;"));
            } else if c.is_element() {
                let t = c.tag_name().name();
                let keep = matches!(t, "p" | "ul" | "ol" | "li");
                if keep {
                    out.push('<');
                    out.push_str(t);
                    out.push('>');
                }
                walk(c, out);
                if keep {
                    out.push_str("</");
                    out.push_str(t);
                    out.push('>');
                }
            }
        }
    }
    let mut s = String::new();
    walk(node, &mut s);
    s
}

fn pick_description(node: roxmltree::Node, lang: &str) -> String {
    let mut best = (0u8, String::new());
    for c in node.children().filter(|c| c.has_tag_name("description")) {
        let s = lang_ok(&c, lang);
        if s > best.0 {
            let kids: Vec<roxmltree::Node> = c.children().filter(|k| k.is_element()).collect();
            let localized_children =
                kids.iter().any(|k| k.attribute(("http://www.w3.org/XML/1998/namespace", "lang")).is_some());
            let text = if localized_children {
                let mut o = String::new();
                for k in kids.iter().filter(|k| lang_ok(k, lang) > 0) {
                    let t = k.tag_name().name();
                    o.push_str(&format!("<{t}>{}</{t}>", markup_of(*k)));
                }
                o
            } else {
                markup_of(c)
            };
            best = (s, text);
        }
    }
    best.1
}

pub fn parse_xml(xml: &str, icon_base: &Path, flatpak_remote: &str, lang: &str) -> Vec<Component> {
    let Ok(doc) = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    ) else {
        return vec![];
    };
    let root = doc.root_element();
    let origin = root.attribute("origin").unwrap_or("");
    let mut out = vec![];
    for c in root.children().filter(|n| n.has_tag_name("component")) {
        let ty = c.attribute("type").unwrap_or("");
        if !matches!(ty, "desktop-application" | "desktop" | "console-application" | "web-application") {
            continue;
        }
        let mut comp = Component {
            id: pick_text(c, "id", ""),
            pkgname: pick_text(c, "pkgname", ""),
            name: pick_text(c, "name", lang),
            summary: pick_text(c, "summary", lang),
            description: pick_description(c, lang),
            license: pick_text(c, "project_license", ""),
            flatpak_remote: flatpak_remote.into(),
            ..Default::default()
        };
        if comp.id.is_empty() {
            continue;
        }
        comp.developer = pick_text(c, "developer_name", lang);
        if comp.developer.is_empty() {
            if let Some(d) = c.children().find(|n| n.has_tag_name("developer")) {
                comp.developer = pick_text(d, "name", lang);
            }
        }
        let mut best_icon: (u32, Icon) = (0, Icon::None);
        for i in c.children().filter(|n| n.has_tag_name("icon")) {
            let w: u32 = i.attribute("width").and_then(|x| x.parse().ok()).unwrap_or(48);
            let t = i.text().unwrap_or("").trim();
            if t.is_empty() {
                continue;
            }
            let (score, icon) = match i.attribute("type").unwrap_or("") {
                "cached" => {
                    let dir = if flatpak_remote.is_empty() {
                        icon_base.join(origin).join(format!("{w}x{w}"))
                    } else {
                        icon_base.join(format!("{w}x{w}"))
                    };
                    let p = dir.join(t);
                    if !p.exists() {
                        continue;
                    }
                    (1000 + w.min(256), Icon::Path(p))
                }
                "local" => (900 + w.min(256), Icon::Path(PathBuf::from(t))),
                "remote" => (500 + w.min(256), Icon::Url(t.into())),
                "stock" => (100, Icon::Named(t.into())),
                _ => continue,
            };
            if score > best_icon.0 {
                best_icon = (score, icon);
            }
        }
        comp.icon = best_icon.1;
        if let Some(cats) = c.children().find(|n| n.has_tag_name("categories")) {
            comp.categories = cats
                .children()
                .filter(|n| n.has_tag_name("category"))
                .filter_map(|n| n.text())
                .map(|s| s.trim().to_string())
                .collect();
        }
        if let Some(k) = c.children().find(|n| n.has_tag_name("keywords") && lang_ok(n, "") > 0) {
            comp.keywords = k
                .children()
                .filter(|n| n.has_tag_name("keyword") && lang_ok(n, "") > 0)
                .filter_map(|n| n.text())
                .map(|s| s.trim().to_string())
                .collect();
        }
        for u in c.children().filter(|n| n.has_tag_name("url")) {
            if let (Some(t), Some(v)) = (u.attribute("type"), u.text()) {
                comp.urls.push((if t == "vcs-browser" { "vcs_browser".into() } else { t.into() }, v.trim().into()));
            }
        }
        if let Some(l) =
            c.children().find(|n| n.has_tag_name("launchable") && n.attribute("type") == Some("desktop-id"))
        {
            comp.desktop_id = l.text().unwrap_or("").trim().into();
        }
        if let Some(ss) = c.children().find(|n| n.has_tag_name("screenshots")) {
            for s in ss.children().filter(|n| n.has_tag_name("screenshot")) {
                let mut shot = Screenshot { caption: pick_text(s, "caption", lang), ..Default::default() };
                let mut best_thumb = u32::MAX;
                for img in s.children().filter(|n| n.has_tag_name("image")) {
                    let url = img.text().unwrap_or("").trim().to_string();
                    let w: u32 = img.attribute("width").and_then(|x| x.parse().ok()).unwrap_or(0);
                    let h: u32 = img.attribute("height").and_then(|x| x.parse().ok()).unwrap_or(0);
                    if img.attribute("type") == Some("thumbnail") {
                        let d = w.abs_diff(752);
                        if d < best_thumb {
                            best_thumb = d;
                            shot.thumb = url;
                            shot.width = w;
                            shot.height = h;
                        }
                    } else {
                        shot.full = url;
                        if shot.width == 0 {
                            shot.width = w;
                            shot.height = h;
                        }
                    }
                }
                if shot.thumb.is_empty() {
                    shot.thumb = shot.full.clone();
                }
                if shot.full.is_empty() {
                    shot.full = shot.thumb.clone();
                }
                if !shot.thumb.is_empty() {
                    comp.screenshots.push(shot);
                }
            }
        }
        if let Some(rs) = c.children().find(|n| n.has_tag_name("releases")) {
            for r in rs.children().filter(|n| n.has_tag_name("release")) {
                comp.releases.push(Release {
                    version: r.attribute("version").unwrap_or("").into(),
                    timestamp: r
                        .attribute("timestamp")
                        .and_then(parse_date)
                        .or_else(|| r.attribute("date").and_then(parse_date)),
                    notes: markup::parse(&pick_description(r, lang)),
                });
            }
        }
        if let Some(cr) = c.children().find(|n| n.has_tag_name("content_rating")) {
            let attrs: Vec<(String, String)> = cr
                .children()
                .filter(|n| n.has_tag_name("content_attribute"))
                .filter_map(|n| Some((n.attribute("id")?.to_string(), n.text()?.trim().to_string())))
                .collect();
            comp.age = Some(oars_age(&attrs));
        }
        out.push(comp);
    }
    out
}

fn ystr(v: &Value, lang: &str) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Object(m) => m
            .get(lang)
            .or_else(|| m.get("C"))
            .or_else(|| m.get("en"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

pub fn parse_dep11(text: &str, icon_base: &Path, lang: &str) -> Vec<Component> {
    let docs = crate::yaml::documents(text);
    let mut origin = String::new();
    let mut media = String::new();
    let mut out = vec![];
    for d in docs {
        if d.get("File").is_some() {
            origin = d.get("Origin").and_then(|x| x.as_str()).unwrap_or("").into();
            media = d.get("MediaBaseUrl").and_then(|x| x.as_str()).unwrap_or("").into();
            continue;
        }
        let ty = d.get("Type").and_then(|x| x.as_str()).unwrap_or("");
        if !matches!(ty, "desktop-application" | "console-application" | "web-application") {
            continue;
        }
        let full = |u: &str| {
            if u.starts_with("http") || media.is_empty() {
                u.to_string()
            } else {
                format!("{}/{}", media.trim_end_matches('/'), u)
            }
        };
        let mut c = Component {
            id: d.get("ID").and_then(|x| x.as_str()).unwrap_or("").into(),
            pkgname: d.get("Package").and_then(|x| x.as_str()).unwrap_or("").into(),
            name: d.get("Name").map(|v| ystr(v, lang)).unwrap_or_default(),
            summary: d.get("Summary").map(|v| ystr(v, lang)).unwrap_or_default(),
            description: d.get("Description").map(|v| ystr(v, lang)).unwrap_or_default(),
            license: d.get("ProjectLicense").and_then(|x| x.as_str()).unwrap_or("").into(),
            ..Default::default()
        };
        if c.id.is_empty() {
            continue;
        }
        c.developer = d.get("DeveloperName").map(|v| ystr(v, lang)).unwrap_or_default();
        if c.developer.is_empty() {
            c.developer = d.get("Developer").and_then(|x| x.get("name")).map(|v| ystr(v, lang)).unwrap_or_default();
        }
        c.categories = d
            .get("Categories")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        c.keywords = d
            .get("Keywords")
            .and_then(|k| k.get(lang).or_else(|| k.get("C")))
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if let Some(icon) = d.get("Icon") {
            let mut best = (0u32, Icon::None);
            if let Some(a) = icon.get("cached").and_then(|x| x.as_array()) {
                for i in a {
                    let w = i.get("width").and_then(|x| x.as_u64()).unwrap_or(64) as u32;
                    let n = i.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    let p = icon_base.join(&origin).join(format!("{w}x{w}")).join(n);
                    if p.exists() && 1000 + w > best.0 {
                        best = (1000 + w, Icon::Path(p));
                    }
                }
            }
            if let Some(a) = icon.get("remote").and_then(|x| x.as_array()) {
                for i in a {
                    let w = i.get("width").and_then(|x| x.as_u64()).unwrap_or(64) as u32;
                    if let Some(u) = i.get("url").and_then(|x| x.as_str()) {
                        if 500 + w > best.0 {
                            best = (500 + w, Icon::Url(full(u)));
                        }
                    }
                }
            }
            if best.0 == 0 {
                if let Some(s) = icon.get("stock").and_then(|x| x.as_str()) {
                    best = (1, Icon::Named(s.into()));
                }
            }
            c.icon = best.1;
        }
        if let Some(u) = d.get("Url").and_then(|x| x.as_object()) {
            for (k, v) in u {
                if let Some(s) = v.as_str() {
                    c.urls.push((k.replace('-', "_"), s.into()));
                }
            }
        }
        if let Some(l) = d.get("Launchable").and_then(|l| l.get("desktop-id")).and_then(|x| x.as_array()) {
            c.desktop_id = l.first().and_then(|x| x.as_str()).unwrap_or("").into();
        }
        if let Some(a) = d.get("Screenshots").and_then(|x| x.as_array()) {
            for s in a {
                let src = s.get("source-image");
                let full_url = src.and_then(|x| x.get("url")).and_then(|x| x.as_str()).map(full).unwrap_or_default();
                let thumb = s
                    .get("thumbnails")
                    .and_then(|x| x.as_array())
                    .and_then(|t| {
                        t.iter()
                            .min_by_key(|x| (x.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as i64 - 752).abs())
                    })
                    .and_then(|x| x.get("url"))
                    .and_then(|x| x.as_str())
                    .map(full)
                    .unwrap_or_else(|| full_url.clone());
                if thumb.is_empty() {
                    continue;
                }
                c.screenshots.push(Screenshot {
                    thumb,
                    full: full_url,
                    caption: s.get("caption").map(|v| ystr(v, lang)).unwrap_or_default(),
                    width: src.and_then(|x| x.get("width")).and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    height: src.and_then(|x| x.get("height")).and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                });
            }
        }
        if let Some(a) = d.get("Releases").and_then(|x| x.as_array()) {
            for r in a {
                c.releases.push(Release {
                    version: r
                        .get("version")
                        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                        .unwrap_or_default(),
                    timestamp: r.get("unix-timestamp").and_then(|x| x.as_i64()),
                    notes: markup::parse(&r.get("description").map(|v| ystr(v, lang)).unwrap_or_default()),
                });
            }
        }
        out.push(c);
    }
    out
}

pub fn read_maybe_gz(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut s = String::new();
        flate2::read::MultiGzDecoder::new(&bytes[..]).read_to_string(&mut s).ok()?;
        Some(s)
    } else {
        String::from_utf8(bytes).ok()
    }
}

fn list_dir(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

pub fn native_sources() -> Vec<(PathBuf, PathBuf)> {
    let mut out = vec![];
    for (xml, icons) in [
        ("/usr/share/swcatalog/xml", "/usr/share/swcatalog/icons"),
        ("/usr/share/app-info/xmls", "/usr/share/app-info/icons"),
        ("/var/lib/swcatalog/yaml", "/var/lib/swcatalog/icons"),
        ("/var/lib/app-info/yaml", "/var/lib/app-info/icons"),
        ("/var/cache/swcatalog/xml", "/var/cache/swcatalog/icons"),
    ] {
        for f in list_dir(Path::new(xml)) {
            let n = f.to_string_lossy();
            if n.contains("flatpak") {
                continue;
            }
            if n.ends_with(".xml") || n.ends_with(".xml.gz") || n.ends_with(".yml") || n.ends_with(".yml.gz") {
                out.push((f, PathBuf::from(icons)));
            }
        }
    }
    out
}

pub fn flatpak_sources() -> Vec<(PathBuf, PathBuf, String)> {
    let arch = std::env::consts::ARCH;
    let mut roots = vec![PathBuf::from("/var/lib/flatpak/appstream")];
    if let Some(d) = dirs::data_dir() {
        roots.push(d.join("flatpak/appstream"));
    }
    let mut out = vec![];
    for r in roots {
        for remote in list_dir(&r) {
            let active = remote.join(arch).join("active");
            let name = remote.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for f in ["appstream.xml.gz", "appstream.xml"] {
                let p = active.join(f);
                if p.exists() {
                    out.push((p, active.join("icons"), name.clone()));
                    break;
                }
            }
        }
    }
    out
}

#[derive(Default)]
pub struct Catalog {
    pub native: Vec<Component>,
    pub flatpak: Vec<Component>,
    by_pkg: HashMap<String, usize>,
    by_id: HashMap<String, usize>,
    flatpak_by_id: HashMap<String, usize>,
}

pub fn norm_id(id: &str) -> String {
    id.trim_end_matches(".desktop").to_lowercase()
}

impl Catalog {
    pub fn from_components(native: Vec<Component>, flatpak: Vec<Component>) -> Catalog {
        let mut c = Catalog { native, flatpak, ..Default::default() };
        for (i, n) in c.native.iter().enumerate() {
            if !n.pkgname.is_empty() {
                c.by_pkg.entry(n.pkgname.clone()).or_insert(i);
            }
            c.by_id.entry(norm_id(&n.id)).or_insert(i);
        }
        for (i, n) in c.flatpak.iter().enumerate() {
            c.flatpak_by_id.entry(norm_id(&n.id)).or_insert(i);
        }
        c
    }

    pub fn load(lang: &str) -> Catalog {
        let mut native = vec![];
        for (f, icons) in native_sources() {
            let Some(text) = read_maybe_gz(&f) else { continue };
            let n = f.to_string_lossy();
            if n.ends_with(".yml") || n.ends_with(".yml.gz") {
                native.extend(parse_dep11(&text, &icons, lang));
            } else {
                native.extend(parse_xml(&text, &icons, "", lang));
            }
        }
        let mut flatpak = vec![];
        for (f, icons, remote) in flatpak_sources() {
            if let Some(text) = read_maybe_gz(&f) {
                flatpak.extend(parse_xml(&text, &icons, &remote, lang));
            }
        }
        Catalog::from_components(native, flatpak)
    }

    pub fn by_package(&self, pkg: &str) -> Option<&Component> {
        self.by_pkg.get(pkg).map(|i| &self.native[*i])
    }

    pub fn native_by_id(&self, id: &str) -> Option<&Component> {
        self.by_id.get(&norm_id(id)).map(|i| &self.native[*i])
    }

    pub fn flatpak_by_id(&self, id: &str) -> Option<&Component> {
        self.flatpak_by_id.get(&norm_id(id)).map(|i| &self.flatpak[*i])
    }

    pub fn search<'a>(&'a self, q: &str, flatpak: bool) -> Vec<&'a Component> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let list = if flatpak { &self.flatpak } else { &self.native };
        let mut hits: Vec<(i32, &Component)> = list
            .iter()
            .filter_map(|c| {
                let name = c.name.to_lowercase();
                let s = if name == q {
                    100
                } else if name.starts_with(&q) {
                    60
                } else if name.contains(&q) {
                    40
                } else if c.keywords.iter().any(|k| k.to_lowercase().contains(&q)) || c.pkgname.contains(&q) {
                    25
                } else if c.summary.to_lowercase().contains(&q) {
                    10
                } else {
                    return None;
                };
                Some((s, c))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        hits.into_iter().map(|h| h.1).take(60).collect()
    }

    pub fn in_category<'a>(&'a self, cats: &[&str], flatpak: bool) -> Vec<&'a Component> {
        let list = if flatpak { &self.flatpak } else { &self.native };
        list.iter().filter(|c| c.categories.iter().any(|x| cats.iter().any(|k| k.eq_ignore_ascii_case(x)))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<components version="0.14" origin="archlinux-arch-extra">
  <component type="desktop-application">
    <id>org.gimp.GIMP</id>
    <pkgname>gimp</pkgname>
    <name>GNU Image Manipulation Program</name>
    <name xml:lang="ru">Редактор GIMP</name>
    <summary>Create images and edit photographs</summary>
    <description><p>GIMP is <em>great</em>.</p><ul><li>Layers</li></ul></description>
    <description xml:lang="de"><p>GIMP ist toll.</p></description>
    <icon type="stock">gimp</icon>
    <icon type="remote" width="128" height="128">https://x/gimp.png</icon>
    <categories><category>Graphics</category><category>2DGraphics</category></categories>
    <keywords><keyword>paint</keyword><keyword xml:lang="de">malen</keyword></keywords>
    <url type="homepage">https://www.gimp.org/</url>
    <url type="vcs-browser">https://gitlab.gnome.org/GNOME/gimp</url>
    <developer><name>The GIMP team</name></developer>
    <project_license>GPL-3.0+</project_license>
    <launchable type="desktop-id">gimp.desktop</launchable>
    <screenshots><screenshot type="default"><caption>Editing</caption><image type="source" width="1920" height="1080">https://x/s.png</image><image type="thumbnail" width="752" height="423">https://x/t.png</image></screenshot></screenshots>
    <releases><release version="3.0.4" timestamp="1745000000"><description><p>Fixes</p></description></release></releases>
    <content_rating type="oars-1.1"><content_attribute id="violence-cartoon">mild</content_attribute><content_attribute id="social-chat">intense</content_attribute></content_rating>
  </component>
  <component type="addon"><id>org.gimp.Plugin</id></component>
</components>"#;

    #[test]
    fn xml_catalog() {
        let v = parse_xml(XML, Path::new("/nonexistent"), "", "");
        assert_eq!(v.len(), 1);
        let c = &v[0];
        assert_eq!(c.pkgname, "gimp");
        assert_eq!(c.name, "GNU Image Manipulation Program");
        assert_eq!(c.icon, Icon::Url("https://x/gimp.png".into()));
        assert_eq!(c.keywords, vec!["paint"]);
        assert_eq!(c.developer, "The GIMP team");
        assert_eq!(c.screenshots[0].thumb, "https://x/t.png");
        assert_eq!(c.screenshots[0].full, "https://x/s.png");
        assert_eq!(c.age, Some(13));
        assert_eq!(c.urls[1].0, "vcs_browser");
        let d = c.details(Some(Origin::Pacman)).unwrap();
        assert_eq!(d.pkg.key(), "pacman:gimp");
        assert_eq!(d.description.len(), 2);
        assert_eq!(d.releases[0].notes.len(), 1);
        let ru = parse_xml(XML, Path::new("/nonexistent"), "", "ru");
        assert_eq!(ru[0].name, "Редактор GIMP");
        let de = parse_xml(XML, Path::new("/nonexistent"), "", "de");
        assert_eq!(markup::parse(&de[0].description), vec![crate::model::Block::Para("GIMP ist toll.".into())]);
    }

    #[test]
    fn flatpak_catalog_and_lookup() {
        let v = parse_xml(XML, Path::new("/nonexistent"), "flathub", "");
        let p = v[0].package(None).unwrap();
        assert_eq!(p.key(), "flatpak:org.gimp.GIMP");
        let cat = Catalog::from_components(parse_xml(XML, Path::new("/x"), "", ""), v);
        assert!(cat.by_package("gimp").is_some());
        assert!(cat.native_by_id("org.gimp.gimp.desktop").is_some());
        assert!(cat.flatpak_by_id("org.gimp.GIMP").is_some());
        assert_eq!(cat.search("gimp", false).len(), 1);
        assert_eq!(cat.search("paint", true).len(), 1);
        assert_eq!(cat.in_category(&["graphics"], false).len(), 1);
    }

    #[test]
    fn ages() {
        assert_eq!(store_age(0), "4+");
        assert_eq!(store_age(9), "9+");
        assert_eq!(store_age(13), "13+");
        assert_eq!(store_age(16), "16+");
        assert_eq!(store_age(18), "18+");
        assert_eq!(oars_age(&[]), 0);
    }

    #[test]
    fn dep11_catalog() {
        let y = "---\nFile: DEP-11\nOrigin: debian-main\nMediaBaseUrl: https://m\n---\nType: desktop-application\nID: org.gnome.Maps\nPackage: gnome-maps\nName:\n  C: Maps\nSummary:\n  C: Find places\nCategories:\n- Utility\nIcon:\n  remote:\n  - url: org/gnome/maps.png\n    width: 128\nScreenshots:\n- source-image:\n    url: a/b.png\n    width: 1000\n    height: 600\nReleases:\n- version: 46.0\n  unix-timestamp: 1700000000\n";
        let v = parse_dep11(y, Path::new("/x"), "");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].icon, Icon::Url("https://m/org/gnome/maps.png".into()));
        assert_eq!(v[0].screenshots[0].thumb, "https://m/a/b.png");
        assert_eq!(v[0].releases[0].timestamp, Some(1700000000));
        assert_eq!(v[0].package(Some(Origin::Apt)).unwrap().name, "gnome-maps");
    }
}
