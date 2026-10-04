use crate::flatpak::{context_from_json, describe};
use crate::http::{urlencode, Http};
use crate::markup;
use crate::model::{Details, Icon, Link, Origin, Package, Release, Screenshot};
use crate::units::parse_date;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub const API: &str = "https://flathub.org/api/v2";
const TTL_LIST: Duration = Duration::from_secs(3 * 3600);
const TTL_APP: Duration = Duration::from_secs(12 * 3600);

pub struct Flathub {
    pub http: Arc<Http>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn hit_to_package(v: &Value) -> Option<Package> {
    let id = v.get("app_id").or_else(|| v.get("id")).and_then(|x| x.as_str())?;
    if !id.contains('.') {
        return None;
    }
    let mut p = Package::new(Origin::Flatpak, id);
    p.appstream_id = id.into();
    p.title = s(v, "name");
    p.summary = s(v, "summary");
    p.developer = s(v, "developer_name");
    p.license = s(v, "project_license");
    p.repo = "flathub".into();
    p.is_app = true;
    let icon = s(v, "icon");
    if !icon.is_empty() {
        p.icon = Icon::Url(icon);
    }
    p.verified = v.get("verification_verified").and_then(|x| x.as_bool()).unwrap_or(false)
        || v.get("verification_verified").and_then(|x| x.as_str()) == Some("true");
    p.installs = v.get("installs_last_month").and_then(|x| x.as_u64());
    let mut cats = vec![];
    match v.get("main_categories") {
        Some(Value::String(c)) => cats.push(c.clone()),
        Some(Value::Array(a)) => cats.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string))),
        _ => {}
    }
    if let Some(a) = v.get("sub_categories").and_then(|x| x.as_array()) {
        cats.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string)));
    }
    if let Some(a) = v.get("categories").and_then(|x| x.as_array()) {
        cats.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string)));
    }
    cats.dedup();
    p.categories = cats;
    p.keywords = v
        .get("keywords")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Some(p)
}

pub fn hits(v: &Value) -> Vec<Package> {
    v.get("hits").and_then(|h| h.as_array()).map(|a| a.iter().filter_map(hit_to_package).collect()).unwrap_or_default()
}

pub fn age_from_details(v: &Value) -> Option<u8> {
    let d = v.get("content_rating_details")?;
    let loc = d.get("en_US").or_else(|| d.as_object().and_then(|o| o.values().next()))?;
    loc.get("minimumAge").and_then(|x| x.as_u64()).map(|a| a.min(99) as u8)
}

pub fn content_descriptions(v: &Value) -> Vec<String> {
    let Some(d) = v.get("content_rating_details") else { return vec![] };
    let Some(loc) = d.get("en_US").or_else(|| d.as_object().and_then(|o| o.values().next())) else { return vec![] };
    loc.get("categories")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter(|c| c.get("level").and_then(|l| l.as_str()).is_some_and(|l| l != "none" && l != "unknown"))
                .map(|c| s(c, "description"))
                .filter(|d| !d.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn pick_shot(sizes: &[Value], target: u32) -> (String, u32, u32) {
    let mut best: Option<(i64, String, u32, u32)> = None;
    for sz in sizes {
        let w: u32 = sz
            .get("width")
            .and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or(x.as_u64().map(|n| n as u32)))
            .unwrap_or(0);
        let h: u32 = sz
            .get("height")
            .and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or(x.as_u64().map(|n| n as u32)))
            .unwrap_or(0);
        let src = s(sz, "src");
        if src.is_empty() {
            continue;
        }
        let score = if w >= target { (w - target) as i64 } else { (target - w) as i64 * 3 };
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, src, w, h));
        }
    }
    best.map(|b| (b.1, b.2, b.3)).unwrap_or_default()
}

pub fn parse_appstream(v: &Value) -> Option<Details> {
    let mut pkg = hit_to_package(v)?;
    pkg.developer = s(v, "developer_name");
    let mut d = Details { description: markup::parse(&s(v, "description")), ..Default::default() };
    if let Some(a) = v.get("screenshots").and_then(|x| x.as_array()) {
        for shot in a {
            let Some(sizes) = shot.get("sizes").and_then(|x| x.as_array()) else { continue };
            let (thumb, w, h) = pick_shot(sizes, 752);
            let (full, _, _) = pick_shot(sizes, 1600);
            if thumb.is_empty() {
                continue;
            }
            let caption =
                shot.get("caption").map(|c| c.as_str().map(str::to_string).unwrap_or_default()).unwrap_or_default();
            d.screenshots.push(Screenshot { thumb, full, caption, width: w, height: h });
        }
    }
    if let Some(a) = v.get("releases").and_then(|x| x.as_array()) {
        for r in a {
            let ts = r.get("timestamp").and_then(|t| t.as_i64().or_else(|| t.as_str().and_then(parse_date)));
            let ts = ts.or_else(|| r.get("date").and_then(|t| t.as_str()).and_then(parse_date));
            d.releases.push(Release {
                version: s(r, "version"),
                timestamp: ts,
                notes: markup::parse(&s(r, "description")),
            });
        }
    }
    if let Some(r) = d.releases.first() {
        pkg.version = r.version.clone();
        d.updated = r.timestamp;
    }
    if let Some(u) = v.get("urls").and_then(|x| x.as_object()) {
        for k in
            ["homepage", "bugtracker", "help", "donation", "contact", "vcs_browser", "translate", "faq", "contribute"]
        {
            if let Some(url) = u.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                d.links.push(Link { kind: k.into(), url: url.into() });
            }
        }
        pkg.homepage = u.get("homepage").and_then(|x| x.as_str()).unwrap_or("").into();
    }
    d.age = age_from_details(v);
    d.content = content_descriptions(v);
    if let Some(b) = v.get("branding").and_then(|x| x.as_array()) {
        for br in b {
            match br.get("scheme_preference").and_then(|x| x.as_str()) {
                Some("dark") => d.brand_dark = s(br, "value"),
                _ => d.brand_light = s(br, "value"),
            }
        }
    }
    if let Some(m) = v.get("metadata") {
        pkg.verified |= m.get("flathub::verification::verified").and_then(|x| x.as_bool()).unwrap_or(false);
    }
    d.runtime = v.get("bundle").map(|b| s(b, "runtime")).unwrap_or_default();
    if let Some(l) = v.get("launchable") {
        pkg.desktop_id = s(l, "value");
    }
    if let Some(t) = v.get("translation").and_then(|x| x.as_object()) {
        d.languages = t.keys().cloned().collect();
    }
    d.pkg = pkg;
    Some(d)
}

pub fn apply_summary(d: &mut Details, v: &Value) {
    d.pkg.download_size = v.get("download_size").and_then(|x| x.as_u64()).or(d.pkg.download_size);
    if let Some(n) = v.get("installed_size").and_then(|x| x.as_u64()) {
        d.pkg.size = Some(n);
    }
    if let Some(meta) = v.get("metadata") {
        if let Some(p) = meta.get("permissions") {
            d.permissions = describe(&context_from_json(p));
        }
        let rt = s(meta, "runtimeName");
        if !rt.is_empty() {
            d.runtime = rt;
        }
    }
    if let Some(t) = v.get("timestamp").and_then(|x| x.as_i64()) {
        d.updated = d.updated.or(Some(t));
    }
}

impl Flathub {
    pub fn new(http: Arc<Http>) -> Flathub {
        Flathub { http }
    }

    fn list(&self, url: &str) -> Result<Vec<Package>, String> {
        self.http.get_json(url, TTL_LIST).map(|v| hits(&v))
    }

    pub fn collection(&self, name: &str, page: u32, per_page: u32) -> Result<Vec<Package>, String> {
        self.list(&format!("{API}/collection/{name}?page={page}&per_page={per_page}"))
    }

    pub fn category(&self, cat: &str, sub: Option<&str>, page: u32, per_page: u32) -> Result<Vec<Package>, String> {
        match sub {
            Some(sc) => self.list(&format!(
                "{API}/collection/category/{}/subcategories?subcategory={}&page={page}&per_page={per_page}",
                urlencode(cat),
                urlencode(sc)
            )),
            None => self.list(&format!("{API}/collection/category/{}?page={page}&per_page={per_page}", urlencode(cat))),
        }
    }

    pub fn developer(&self, name: &str) -> Result<Vec<Package>, String> {
        self.list(&format!("{API}/collection/developer/{}?page=1&per_page=30", urlencode(name)))
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, String> {
        let body = json!({ "query": query, "filters": [] });
        self.http
            .post_json_cached(&format!("{API}/search?locale=en"), &body, Duration::from_secs(1800))
            .map(|v| hits(&v))
    }

    pub fn details(&self, id: &str) -> Result<Details, String> {
        let v = self.http.get_json(&format!("{API}/appstream/{}", urlencode(id)), TTL_APP)?;
        let mut d = parse_appstream(&v).ok_or("not found")?;
        if let Ok(sum) = self.http.get_json(&format!("{API}/summary/{}", urlencode(id)), TTL_APP) {
            apply_summary(&mut d, &sum);
        }
        if let Ok(st) = self.http.get_json(&format!("{API}/stats/{}", urlencode(id)), TTL_APP) {
            d.pkg.installs = st.get("installs_total").and_then(|x| x.as_u64()).or(d.pkg.installs);
        }
        Ok(d)
    }

    pub fn app_of_the_day(&self, day: &str) -> Option<String> {
        let v = self.http.get_json(&format!("{API}/app-picks/app-of-the-day/{day}"), TTL_APP).ok()?;
        v.get("app_id").and_then(|x| x.as_str()).map(str::to_string)
    }

    pub fn apps_of_the_week(&self, day: &str) -> Vec<String> {
        let Ok(v) = self.http.get_json(&format!("{API}/app-picks/apps-of-the-week/{day}"), TTL_APP) else {
            return vec![];
        };
        let mut a: Vec<(u64, String)> = v
            .get("apps")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter().map(|x| (x.get("position").and_then(|p| p.as_u64()).unwrap_or(99), s(x, "app_id"))).collect()
            })
            .unwrap_or_default();
        a.sort();
        a.into_iter().map(|x| x.1).filter(|x| !x.is_empty()).collect()
    }

    pub fn web_url(id: &str) -> String {
        format!("https://flathub.org/apps/{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = r##"{"id":"org.gimp.GIMP","name":"GNU Image Manipulation Program","summary":"High-end image creation","description":"<p>GIMP is free.</p><ul><li>Layers</li></ul>","developer_name":"The GIMP team","icon":"https://dl.flathub.org/x/128x128/org.gimp.GIMP.png","screenshots":[{"caption":"Main window","sizes":[{"width":"1920","height":"1080","src":"https://a/orig.png"},{"width":"752","height":"423","src":"https://a/752.png"},{"width":"1248","height":"702","src":"https://a/1248.png"}]}],"releases":[{"timestamp":"1788998400","version":"3.2.6","description":"<p>Bug fixes</p>"},{"timestamp":"1780000000","version":"3.2.4"}],"urls":{"homepage":"https://www.gimp.org/","bugtracker":"https://b","contact":null},"categories":["Graphics","2DGraphics"],"project_license":"GPL-3.0+","content_rating_details":{"en_US":{"categories":[{"description":"No violence","id":"violence","level":"none"},{"description":"Users can chat","id":"social","level":"mild"}],"minimumAge":13}},"branding":[{"value":"#f4e4cb","type":"primary","scheme_preference":"light"},{"value":"#293a56","type":"primary","scheme_preference":"dark"}],"metadata":{"flathub::verification::verified":true},"bundle":{"value":"app/org.gimp.GIMP/x86_64/stable","type":"flatpak","runtime":"org.gnome.Platform/x86_64/51"},"launchable":{"value":"org.gimp.GIMP.desktop","type":"desktop-id"}}"##;

    #[test]
    fn appstream_details() {
        let v: Value = serde_json::from_str(APP).unwrap();
        let d = parse_appstream(&v).unwrap();
        assert_eq!(d.pkg.name, "org.gimp.GIMP");
        assert_eq!(d.pkg.version, "3.2.6");
        assert!(d.pkg.verified);
        assert_eq!(d.description.len(), 2);
        assert_eq!(d.screenshots[0].thumb, "https://a/752.png");
        assert_eq!(d.screenshots[0].full, "https://a/orig.png");
        assert_eq!(d.screenshots[0].caption, "Main window");
        assert_eq!(d.releases.len(), 2);
        assert_eq!(d.releases[0].timestamp, Some(1788998400));
        assert_eq!(d.link("homepage"), Some("https://www.gimp.org/"));
        assert_eq!(d.link("contact"), None);
        assert_eq!(d.age, Some(13));
        assert_eq!(d.content, vec!["Users can chat"]);
        assert_eq!(d.brand_dark, "#293a56");
        assert_eq!(d.pkg.desktop_id, "org.gimp.GIMP.desktop");
        assert_eq!(d.pkg.icon, Icon::Url("https://dl.flathub.org/x/128x128/org.gimp.GIMP.png".into()));
    }

    #[test]
    fn summary_and_hits() {
        let v: Value = serde_json::from_str(APP).unwrap();
        let mut d = parse_appstream(&v).unwrap();
        let sum: Value = serde_json::from_str(r#"{"download_size":101495709,"installed_size":268301312,"metadata":{"runtimeName":"GNOME 51","permissions":{"shared":["network"],"filesystems":["home"]}}}"#).unwrap();
        apply_summary(&mut d, &sum);
        assert_eq!(d.pkg.size, Some(268301312));
        assert_eq!(d.runtime, "GNOME 51");
        assert_eq!(d.permissions.len(), 2);
        let h: Value = serde_json::from_str(r#"{"hits":[{"app_id":"org.a.B","name":"B","summary":"s","main_categories":"game","sub_categories":["Emulator"],"verification_verified":true,"installs_last_month":42},{"name":"bad"}]}"#).unwrap();
        let p = hits(&h);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].categories, vec!["game", "Emulator"]);
        assert_eq!(p[0].installs, Some(42));
        assert!(p[0].verified);
    }
}
