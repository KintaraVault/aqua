use crate::appstream::Catalog;
use crate::aur::Aur;
use crate::flathub::Flathub;
use crate::flatpak::{Flatpak, TOGGLES};
use crate::http::Http;
use crate::installed::{self, Index};
use crate::manager::Manager;
use crate::model::{key_of, Details, Origin, Package, Update};
use crate::prefs::Prefs;
use crate::reviews::Reviews;
use crate::runner::{Cmd, SharedRunner, SystemRunner};
use crate::sections::Feed;
use crate::setup::{self, Requirement};
use crate::system::Env;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

pub struct Store {
    pub run: SharedRunner,
    pub env: RwLock<Env>,
    pub http: Arc<Http>,
    pub flathub: Flathub,
    pub reviews: Reviews,
    pub flatpak: Flatpak,
    pub native: Option<Box<dyn Manager>>,
    pub aur: Option<Aur>,
    catalog: RwLock<Arc<Catalog>>,
    pub prefs: Mutex<Prefs>,
    index: Mutex<Option<Arc<Index>>>,
    updates: Mutex<Option<Arc<Vec<Update>>>>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub apps: Vec<Package>,
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug, Default)]
pub struct Featured {
    pub label: String,
    pub details: Details,
}

pub fn native_manager(o: Origin, run: SharedRunner) -> Box<dyn Manager> {
    match o {
        Origin::Pacman | Origin::Aur => Box::new(crate::pacman::Pacman { run }),
        Origin::Dnf => Box::new(crate::dnf::Dnf { run }),
        Origin::Apt => Box::new(crate::apt::Apt { run }),
        Origin::Flatpak => unreachable!(),
    }
}

fn dedupe(v: &mut Vec<Package>) {
    let mut seen = HashSet::new();
    v.retain(|p| seen.insert(p.key()));
}

impl Store {
    pub fn system() -> Store {
        Store::new(Arc::new(SystemRunner::default()), Arc::new(Http::default()), Prefs::load())
    }

    pub fn new(run: SharedRunner, http: Arc<Http>, prefs: Prefs) -> Store {
        let env = Env::detect(&*run);
        Store::with_env(run, http, prefs, env)
    }

    pub fn with_env(run: SharedRunner, http: Arc<Http>, prefs: Prefs, env: Env) -> Store {
        let native = env.native.map(|o| native_manager(o, run.clone()));
        let aur = env.has_aur().then(|| Aur { run: run.clone(), http: http.clone() });
        let distro = if env.distro_name.is_empty() { "Linux".to_string() } else { env.distro_name.clone() };
        Store {
            flathub: Flathub::new(http.clone()),
            reviews: Reviews::new(http.clone(), &distro),
            flatpak: Flatpak::new(run.clone(), prefs.flatpak_scope),
            native,
            aur,
            catalog: RwLock::new(Arc::new(Catalog::default())),
            prefs: Mutex::new(prefs),
            index: Mutex::new(None),
            updates: Mutex::new(None),
            env: RwLock::new(env),
            http,
            run,
        }
    }

    pub fn env(&self) -> Env {
        self.env.read().unwrap().clone()
    }

    pub fn reload_env(&self) {
        let e = Env::detect(&*self.run);
        *self.env.write().unwrap() = e;
    }

    pub fn prefs(&self) -> Prefs {
        self.prefs.lock().unwrap().clone()
    }

    pub fn set_prefs(&self, p: Prefs) {
        *self.flatpak.scope.lock().unwrap() = p.flatpak_scope;
        p.save();
        *self.prefs.lock().unwrap() = p;
    }

    pub fn load_catalog(&self) {
        let lang = crate::reviews::current_locale();
        let base = lang.split('_').next().unwrap_or("").to_string();
        let c = Catalog::load(&base);
        *self.catalog.write().unwrap() = Arc::new(c);
    }

    pub fn set_catalog(&self, c: Catalog) {
        *self.catalog.write().unwrap() = Arc::new(c);
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.read().unwrap().clone()
    }

    pub fn manager(&self, o: Origin) -> Option<&dyn Manager> {
        match o {
            Origin::Flatpak => Some(&self.flatpak),
            Origin::Aur => self.aur.as_ref().map(|a| a as &dyn Manager),
            _ => self.native.as_deref().filter(|m| m.origin() == o),
        }
    }

    pub fn invalidate(&self) {
        *self.index.lock().unwrap() = None;
        *self.updates.lock().unwrap() = None;
    }

    pub fn invalidate_index(&self) {
        *self.index.lock().unwrap() = None;
    }

    pub fn index(&self) -> Arc<Index> {
        if let Some(i) = self.index.lock().unwrap().clone() {
            return i;
        }
        let env = self.env();
        let (flatpaks, natives, foreign, desktops) = std::thread::scope(|s| {
            let f = s.spawn(|| if env.flatpak { self.flatpak.installed() } else { vec![] });
            let n = s.spawn(|| self.native.as_ref().map(|m| m.installed()).unwrap_or_default());
            let a = s.spawn(|| self.aur.as_ref().map(|a| a.installed()).unwrap_or_default());
            let d = s.spawn(installed::desktop_entries);
            (
                f.join().unwrap_or_default(),
                n.join().unwrap_or_default(),
                a.join().unwrap_or_default(),
                d.join().unwrap_or_default(),
            )
        });
        let foreign: HashSet<String> = foreign.into_iter().map(|p| p.name).collect();
        let paths: Vec<PathBuf> = desktops
            .iter()
            .filter(|d| crate::flatpak::app_of_export(&d.path).is_none())
            .filter(|d| d.path.is_file())
            .map(|d| d.path.clone())
            .collect();
        let owners = self.native.as_ref().map(|m| m.owners(&paths)).unwrap_or_default();
        let catalog = self.catalog();
        let mut idx = installed::build(
            &desktops,
            &flatpaks,
            self.native.as_ref().map(|m| (m.origin(), &natives[..])),
            &foreign,
            &owners,
        );
        for p in &mut idx.apps {
            let comp = match p.origin() {
                Origin::Flatpak => catalog.flatpak_by_id(&p.name),
                _ => catalog.by_package(&p.name),
            };
            if let Some(c) = comp {
                if p.summary.is_empty() {
                    p.summary = c.summary.clone();
                }
                if p.appstream_id.is_empty() {
                    p.appstream_id = c.id.clone();
                }
                if p.developer.is_empty() {
                    p.developer = c.developer.clone();
                }
            }
        }
        let idx = Arc::new(idx);
        *self.index.lock().unwrap() = Some(idx.clone());
        idx
    }

    pub fn cached_updates(&self) -> Option<Arc<Vec<Update>>> {
        self.updates.lock().unwrap().clone()
    }

    pub fn updates(&self) -> Arc<Vec<Update>> {
        if let Some(u) = self.cached_updates() {
            return u;
        }
        let prefs = self.prefs();
        let env = self.env();
        let (f, n, a) = std::thread::scope(|s| {
            let f = s.spawn(|| if env.flatpak { self.flatpak.updates() } else { vec![] });
            let n = s.spawn(|| {
                if prefs.include_system {
                    self.native.as_ref().map(|m| m.updates()).unwrap_or_default()
                } else {
                    vec![]
                }
            });
            let a = s.spawn(|| {
                if prefs.use_aur {
                    self.aur.as_ref().map(|m| m.updates()).unwrap_or_default()
                } else {
                    vec![]
                }
            });
            (f.join().unwrap_or_default(), n.join().unwrap_or_default(), a.join().unwrap_or_default())
        });
        let idx = self.index();
        let apps: HashSet<String> = idx.apps.iter().map(|p| p.key()).collect();
        let mut all: Vec<Update> = f.into_iter().chain(n).chain(a).collect();
        for u in &mut all {
            if apps.contains(&u.key()) {
                u.is_app = true;
            }
        }
        let all = Arc::new(all);
        *self.updates.lock().unwrap() = Some(all.clone());
        all
    }

    pub fn mark(&self, v: &mut [Package]) {
        let idx = self.index();
        let ups: HashMap<String, String> =
            self.cached_updates().map(|u| u.iter().map(|x| (x.key(), x.to.clone())).collect()).unwrap_or_default();
        for p in v.iter_mut() {
            let k = p.key();
            if let Some(ver) = idx.version(&k) {
                p.installed = true;
                p.installed_version = ver.to_string();
                if let Some(app) = idx.app(&k) {
                    if p.desktop_id.is_empty() {
                        p.desktop_id = app.desktop_id.clone();
                    }
                    p.scope = p.scope.or(app.scope);
                }
            } else {
                p.installed = false;
                p.installed_version.clear();
            }
            p.update_version = ups.get(&k).cloned().unwrap_or_default();
        }
    }

    fn local_feed(&self, feed: &Feed) -> Vec<Package> {
        let cat = self.catalog();
        let cats = feed.local_categories();
        let native = self.env().native;
        let mut v: Vec<Package> = vec![];
        let pick = |flatpak: bool| -> Vec<Package> {
            let list: Vec<&crate::appstream::Component> = if cats.is_empty() {
                if flatpak {
                    cat.flatpak.iter().collect()
                } else {
                    cat.native.iter().collect()
                }
            } else {
                cat.in_category(&cats, flatpak)
            };
            list.into_iter().filter_map(|c| c.package(if flatpak { None } else { native })).collect()
        };
        if self.prefs().use_flathub {
            v.extend(pick(true));
        }
        if self.prefs().use_native {
            v.extend(pick(false));
        }
        v.sort_by_key(|p| (p.icon.is_none(), p.summary.is_empty(), p.display_name().to_lowercase()));
        dedupe(&mut v);
        v
    }

    pub fn feed(&self, feed: &Feed, page: u32, per_page: u32) -> Result<Vec<Package>, String> {
        let prefs = self.prefs();
        let remote = if prefs.use_flathub {
            match feed {
                Feed::Collection(c) => self.flathub.collection(c, page, per_page),
                Feed::Category(c, s) => self.flathub.category(c, *s, page, per_page),
            }
        } else {
            Err("Flathub is turned off".into())
        };
        let mut v = match remote {
            Ok(v) if !v.is_empty() => v,
            other => {
                let local = self.local_feed(feed);
                let start = ((page.max(1) - 1) * per_page) as usize;
                let slice: Vec<Package> = local.into_iter().skip(start).take(per_page as usize).collect();
                if slice.is_empty() {
                    return other.and(Ok(vec![]));
                }
                slice
            }
        };
        self.fill_icons(&mut v);
        self.mark(&mut v);
        Ok(v)
    }

    fn fill_icons(&self, v: &mut [Package]) {
        let cat = self.catalog();
        for p in v.iter_mut() {
            if p.icon.is_none() {
                let c =
                    if p.origin() == Origin::Flatpak { cat.flatpak_by_id(&p.name) } else { cat.by_package(&p.name) };
                if let Some(c) = c {
                    p.icon = c.icon.clone();
                }
            }
        }
    }

    pub fn developer(&self, name: &str) -> Vec<Package> {
        let mut v = self.flathub.developer(name).unwrap_or_default();
        self.mark(&mut v);
        v
    }

    pub fn search(&self, query: &str) -> SearchResults {
        let q = query.trim();
        if q.is_empty() {
            return SearchResults::default();
        }
        let prefs = self.prefs();
        let env = self.env();
        let cat = self.catalog();
        let (flathub, native_pk, aur) = std::thread::scope(|s| {
            let f = s.spawn(|| {
                if !prefs.use_flathub {
                    return vec![];
                }
                match self.flathub.search(q) {
                    Ok(v) if !v.is_empty() => v,
                    _ => cat.search(q, true).into_iter().filter_map(|c| c.package(None)).collect(),
                }
            });
            let n = s.spawn(|| {
                if prefs.use_native {
                    self.native.as_ref().map(|m| m.search(q)).unwrap_or_default()
                } else {
                    vec![]
                }
            });
            let a = s.spawn(|| {
                if prefs.use_aur {
                    self.aur.as_ref().map(|m| m.search(q)).unwrap_or_default()
                } else {
                    vec![]
                }
            });
            (f.join().unwrap_or_default(), n.join().unwrap_or_default(), a.join().unwrap_or_default())
        });
        let mut apps = flathub;
        let mut native_apps: Vec<Package> = vec![];
        if prefs.use_native {
            for c in cat.search(q, false) {
                if let Some(p) = c.package(env.native) {
                    native_apps.push(p);
                }
            }
        }
        let mut packages = vec![];
        for mut p in native_pk {
            if let Some(c) = cat.by_package(&p.name) {
                if let Some(mut ap) = c.package(env.native) {
                    ap.merge_from(&p);
                    if !native_apps.iter().any(|x| x.name == ap.name) {
                        native_apps.push(ap);
                    }
                    continue;
                }
            }
            p.is_app = false;
            packages.push(p);
        }
        let preferred_native = prefs.preferred == "native";
        let flathub_ids: HashSet<String> = apps.iter().map(|p| crate::appstream::norm_id(&p.appstream_id)).collect();
        if preferred_native {
            let native_ids: HashSet<String> =
                native_apps.iter().map(|p| crate::appstream::norm_id(&p.appstream_id)).collect();
            apps.retain(|p| !native_ids.contains(&crate::appstream::norm_id(&p.appstream_id)));
            native_apps.extend(apps);
            apps = native_apps;
        } else {
            apps.extend(
                native_apps.into_iter().filter(|p| !flathub_ids.contains(&crate::appstream::norm_id(&p.appstream_id))),
            );
        }
        let ql = q.to_lowercase();
        apps.sort_by_key(|p| {
            let t = p.display_name().to_lowercase();
            if t == ql {
                0
            } else if t.starts_with(&ql) {
                1
            } else if t.contains(&ql) {
                2
            } else {
                3
            }
        });
        packages.extend(aur);
        dedupe(&mut apps);
        dedupe(&mut packages);
        self.fill_icons(&mut apps);
        self.mark(&mut apps);
        self.mark(&mut packages);
        SearchResults { apps, packages }
    }

    pub fn details(&self, pkg: &Package) -> Result<Details, String> {
        let cat = self.catalog();
        let env = self.env();
        let mut d = match pkg.origin() {
            Origin::Flatpak => {
                let remote = if self.prefs().use_flathub && (pkg.repo.is_empty() || pkg.repo == "flathub") {
                    self.flathub.details(&pkg.name).ok()
                } else {
                    None
                };
                let local = || cat.flatpak_by_id(&pkg.name).and_then(|c| c.details(None));
                let mut d = remote.or_else(local).unwrap_or_else(|| Details { pkg: pkg.clone(), ..Default::default() });
                if env.flatpak {
                    if let Some(info) = self.flatpak.info(&pkg.name) {
                        d.permissions = info.permissions;
                        d.pkg.size = info.pkg.size.or(d.pkg.size);
                        d.pkg.scope = info.pkg.scope;
                        d.pkg.repo = info.pkg.repo;
                    }
                }
                d
            }
            o => {
                let m = self.manager(o).ok_or("This source is not available on this system.")?;
                let mut d = m.info(&pkg.name).unwrap_or_else(|| Details { pkg: pkg.clone(), ..Default::default() });
                let comp = cat
                    .by_package(&pkg.name)
                    .or_else(|| (!pkg.appstream_id.is_empty()).then(|| cat.native_by_id(&pkg.appstream_id)).flatten());
                if let Some(c) = comp.filter(|_| o != Origin::Aur) {
                    if let Some(cd) = c.details(env.native) {
                        if cd.description.len() > d.description.len() {
                            d.description = cd.description;
                        }
                        d.screenshots = cd.screenshots;
                        if d.releases.is_empty() {
                            d.releases = cd.releases;
                        }
                        d.age = d.age.or(cd.age);
                        for l in cd.links {
                            if d.link(&l.kind).is_none() {
                                d.links.push(l);
                            }
                        }
                        let summary = d.pkg.summary.clone();
                        d.pkg.title = cd.pkg.title.clone();
                        d.pkg.merge_from(&cd.pkg);
                        d.pkg.icon = cd.pkg.icon;
                        if !cd.pkg.summary.is_empty() {
                            d.pkg.summary = cd.pkg.summary;
                        } else {
                            d.pkg.summary = summary;
                        }
                    }
                }
                d
            }
        };
        d.pkg.merge_from(pkg);
        if d.pkg.title.is_empty() || d.pkg.title == d.pkg.name {
            d.pkg.title = pkg.display_name().to_string();
        }
        if d.pkg.icon.is_none() {
            if let Some(app) = self.index().app(&pkg.key()) {
                d.pkg.icon = app.icon.clone();
            }
        }
        self.mark(std::slice::from_mut(&mut d.pkg));
        Ok(d)
    }

    pub fn alternatives(&self, pkg: &Package) -> Vec<Package> {
        let cat = self.catalog();
        let env = self.env();
        let prefs = self.prefs();
        let mut out = vec![pkg.clone()];
        let id = if pkg.appstream_id.is_empty() {
            pkg.desktop_id.trim_end_matches(".desktop").to_string()
        } else {
            pkg.appstream_id.clone()
        };
        let id = id.trim_end_matches(".desktop").to_string();
        if pkg.origin() != Origin::Flatpak && prefs.use_flathub && id.contains('.') {
            if let Some(c) = cat.flatpak_by_id(&id) {
                out.extend(c.package(None));
            } else if let Ok(d) = self.flathub.details(&id) {
                out.push(d.pkg);
            }
        }
        if !pkg.origin().is_native() && prefs.use_native {
            if let Some(c) = (!id.is_empty()).then(|| cat.native_by_id(&id)).flatten() {
                out.extend(c.package(env.native));
            } else if let Some(m) = &self.native {
                let mut names = vec![];
                let t = pkg.display_name().to_lowercase().replace(' ', "-");
                if !t.is_empty() && t.len() < 40 {
                    names.push(t);
                }
                if let Some(last) = id.rsplit('.').next() {
                    names.push(last.to_lowercase());
                }
                names.dedup();
                for n in names {
                    if let Some(d) = m.info(&n) {
                        let mut p = d.pkg;
                        p.title = pkg.display_name().to_string();
                        p.icon = pkg.icon.clone();
                        out.push(p);
                        break;
                    }
                }
            }
        }
        if pkg.origin() != Origin::Aur && prefs.use_aur {
            if let Some(a) = &self.aur {
                let mut names: Vec<String> = vec![pkg.name.to_lowercase()];
                if let Some(last) = id.rsplit('.').next() {
                    names.push(last.to_lowercase());
                }
                names.push(pkg.display_name().to_lowercase().replace(' ', "-"));
                names.dedup();
                let has_native = out.iter().any(|p| p.origin().is_native());
                if !has_native {
                    for v in a.rpc_info(&names) {
                        if let Some(mut p) = crate::aur::from_rpc(&v) {
                            p.title = pkg.display_name().to_string();
                            p.icon = pkg.icon.clone();
                            out.push(p);
                            break;
                        }
                    }
                }
            }
        }
        dedupe(&mut out);
        self.mark(&mut out);
        out
    }

    pub fn requirements(&self) -> Vec<Requirement> {
        let env = self.env();
        let remotes = if env.flatpak { self.flatpak.remotes() } else { vec![] };
        let is_fh = |r: &crate::flatpak::Remote| r.name == crate::flatpak::FLATHUB || r.url.contains("flathub.org");
        let facts = setup::Facts {
            flathub_user: remotes.iter().any(|r| is_fh(r) && r.scope == crate::model::Scope::User),
            flathub_system: remotes.iter().any(|r| is_fh(r) && r.scope == crate::model::Scope::System),
            native_catalog: !self.catalog().native.is_empty() || !crate::appstream::native_sources().is_empty(),
            scope: self.prefs().flatpak_scope,
        };
        let mut v = setup::requirements(&env, &facts);
        if !env.flatpak {
            v.retain(|r| r.id != "flathub" || !env.can_elevate());
            if let Some(i) = v.iter().position(|r| r.id == "flatpak") {
                if env.native.is_some() {
                    v[i].steps.push(crate::flatpak::Flatpak::add_flathub(facts.scope));
                }
            }
        }
        v
    }

    pub fn install_steps(&self, pkg: &Package) -> Result<Vec<Cmd>, String> {
        let env = self.env();
        match pkg.origin() {
            Origin::Flatpak => {
                if !env.flatpak {
                    return Err("flatpak".into());
                }
                let scope = pkg.scope.unwrap_or(self.prefs().flatpak_scope);
                let mut steps = vec![];
                if (pkg.repo.is_empty() || pkg.repo == "flathub") && !self.flatpak.has_flathub(scope) {
                    steps.push(Flatpak::add_flathub(scope));
                }
                let mut p = pkg.clone();
                p.scope = Some(scope);
                steps.extend(self.flatpak.install(&p));
                Ok(steps)
            }
            Origin::Aur => {
                if !env.can_elevate() {
                    return Err("polkit".into());
                }
                if env.aur_helper.is_none() {
                    return Err("aur".into());
                }
                Ok(self.aur.as_ref().ok_or("aur")?.install(pkg))
            }
            o => {
                if !env.can_elevate() {
                    return Err("polkit".into());
                }
                Ok(self.manager(o).ok_or_else(|| o.id().to_string())?.install(pkg))
            }
        }
    }

    pub fn remove_steps(&self, pkg: &Package, purge: bool) -> Result<Vec<Cmd>, String> {
        let env = self.env();
        if pkg.origin() != Origin::Flatpak && !env.can_elevate() {
            return Err("polkit".into());
        }
        let mut p = pkg.clone();
        if p.origin() == Origin::Flatpak && p.scope.is_none() {
            p.scope = self.index().app(&p.key()).and_then(|a| a.scope);
        }
        Ok(self.manager(p.origin()).ok_or("unavailable")?.remove(&p, purge))
    }

    pub fn update_steps(&self, ups: &[Update]) -> Result<Vec<Cmd>, String> {
        let env = self.env();
        let mut by: Vec<(Origin, Vec<String>)> = vec![];
        for u in ups {
            match by.iter_mut().find(|x| x.0 == u.origin()) {
                Some(x) => x.1.push(u.name.clone()),
                None => by.push((u.origin(), vec![u.name.clone()])),
            }
        }
        by.sort_by_key(|x| match x.0 {
            Origin::Flatpak => 2,
            Origin::Aur => 1,
            _ => 0,
        });
        let mut steps = vec![];
        for (o, names) in by {
            if o != Origin::Flatpak && !env.can_elevate() {
                return Err("polkit".into());
            }
            let m = self.manager(o).ok_or("unavailable")?;
            steps.extend(if m.upgrade_is_whole_system() { m.update(&[]) } else { m.update(&names) });
        }
        Ok(steps)
    }

    pub fn refresh_steps(&self) -> Vec<Cmd> {
        let mut v = vec![];
        if self.env().flatpak {
            v.extend(self.flatpak.refresh());
        }
        if let Some(m) = &self.native {
            if m.origin() == Origin::Apt || m.origin() == Origin::Dnf {
                v.extend(m.refresh());
            }
        }
        v
    }

    pub fn launch(&self, pkg: &Package) -> bool {
        let apps = aqua_apps::scan();
        let mut ids: Vec<String> = vec![];
        for s in [&pkg.desktop_id, &pkg.appstream_id, &pkg.name] {
            if !s.is_empty() {
                ids.push(s.trim_end_matches(".desktop").to_string());
            }
        }
        if let Some(app) = self.index().app(&pkg.key()) {
            ids.insert(0, app.desktop_id.trim_end_matches(".desktop").to_string());
        }
        for id in &ids {
            if let Some(a) = apps.iter().find(|a| a.id.trim_end_matches(".desktop") == id) {
                return aqua_apps::launch(&a.launch_command());
            }
        }
        match pkg.origin() {
            Origin::Flatpak => aqua_apps::launch(&format!("flatpak run {}", aqua_apps::shell_quote(&pkg.name))),
            _ => {
                if aqua_apps::find_in_path(&pkg.name).is_some() {
                    aqua_apps::launch(&aqua_apps::terminal_wrap(&pkg.name))
                } else {
                    false
                }
            }
        }
    }

    pub fn can_open(&self, pkg: &Package) -> bool {
        if !pkg.installed {
            return false;
        }
        if pkg.origin() == Origin::Flatpak || self.index().app(&pkg.key()).is_some() || pkg.is_app {
            return true;
        }
        aqua_apps::find_in_path(&pkg.name).is_some()
    }

    pub fn permissions(&self, pkg: &Package) -> Vec<(String, String, bool)> {
        if pkg.origin() != Origin::Flatpak || !pkg.installed {
            return vec![];
        }
        let scope = pkg.scope.or_else(|| self.index().app(&pkg.key()).and_then(|a| a.scope));
        let (base, ov) = self.flatpak.permissions(&pkg.name, scope);
        let eff = base.overlay(&ov);
        TOGGLES.iter().map(|(id, label)| (id.to_string(), label.to_string(), eff.on(id))).collect()
    }

    pub fn featured(&self) -> Vec<Featured> {
        if !self.prefs().use_flathub {
            return vec![];
        }
        let day = crate::units::today();
        let mut ids: Vec<(String, String)> = vec![];
        if let Some(a) = self.flathub.app_of_the_day(&day) {
            ids.push(("App of the Day".into(), a));
        }
        for a in self.flathub.apps_of_the_week(&day) {
            if !ids.iter().any(|x| x.1 == a) {
                ids.push(("App of the Week".into(), a));
            }
        }
        let mut out: Vec<Featured> = std::thread::scope(|s| {
            let hs: Vec<_> = ids
                .iter()
                .map(|(label, id)| {
                    let label = label.clone();
                    s.spawn(move || self.flathub.details(id).ok().map(|details| Featured { label, details }))
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
        });
        for f in &mut out {
            self.mark(std::slice::from_mut(&mut f.details.pkg));
        }
        out
    }

    pub fn key_package(&self, key: &str) -> Option<Package> {
        let (o, n) = crate::model::parse_key(key)?;
        if let Some(a) = self.index().app(key) {
            return Some(a.clone());
        }
        let mut p = Package::new(o, n);
        if o == Origin::Flatpak {
            p.appstream_id = n.into();
            p.repo = "flathub".into();
        }
        Some(p)
    }

    pub fn native_key(&self, name: &str) -> Option<String> {
        self.env().native.map(|o| key_of(o, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;

    fn store(run: Arc<FakeRunner>, native: Origin) -> Store {
        let env = Env {
            native: Some(native),
            flatpak: true,
            pkexec: true,
            aur_helper: Some("paru".into()),
            ..Default::default()
        };
        let http = Arc::new(Http::new(std::env::temp_dir().join(format!("aqua-store-test-{}", std::process::id()))));
        let prefs = Prefs { use_flathub: false, ..Default::default() };
        Store::with_env(run, http, prefs, env)
    }

    #[test]
    fn update_steps_group_and_order() {
        let run = Arc::new(FakeRunner::with(&["pacman", "flatpak", "paru"]));
        let s = store(run, Origin::Pacman);
        let mk = |o, n: &str| Update { origin: Some(o), name: n.into(), ..Default::default() };
        let steps = s
            .update_steps(&[
                mk(Origin::Flatpak, "a.b.C"),
                mk(Origin::Pacman, "linux"),
                mk(Origin::Pacman, "vim"),
                mk(Origin::Aur, "yay-bin"),
            ])
            .unwrap();
        let lines: Vec<String> = steps.iter().map(|c| c.argv.join(" ")).collect();
        assert_eq!(lines[0], "pacman -Syu --noconfirm");
        assert!(lines[1].starts_with("paru -S --noconfirm --needed --sudo pkexec"));
        assert!(lines[1].ends_with("yay-bin"));
        assert_eq!(lines[2], "flatpak update -y --noninteractive a.b.C");
    }

    #[test]
    fn install_requires_tools() {
        let run = Arc::new(FakeRunner::with(&["dnf", "rpm"]));
        let mut s = store(run.clone(), Origin::Dnf);
        s.env.write().unwrap().flatpak = false;
        assert_eq!(s.install_steps(&Package::new(Origin::Flatpak, "a.b.C")), Err("flatpak".into()));
        s.env.write().unwrap().pkexec = false;
        assert_eq!(s.install_steps(&Package::new(Origin::Dnf, "vim")), Err("polkit".into()));
        s.env.write().unwrap().pkexec = true;
        assert_eq!(
            s.install_steps(&Package::new(Origin::Dnf, "vim")).unwrap()[0].argv,
            vec!["dnf", "install", "-y", "vim"]
        );
        s.env.write().unwrap().flatpak = true;
        run.on("flatpak remotes", 0, "flathub\thttps://dl.flathub.org/repo/\tuser\n");
        let st = s.install_steps(&Package::new(Origin::Flatpak, "a.b.C")).unwrap();
        assert_eq!(st.len(), 1);
        s.prefs.lock().unwrap().flatpak_scope = crate::model::Scope::System;
        let st = s.install_steps(&Package::new(Origin::Flatpak, "a.b.C")).unwrap();
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].argv[1], "remote-add");
        assert!(st[1].argv.contains(&"--system".to_string()));
        s.aur = None;
    }

    #[test]
    fn search_splits_apps_and_packages() {
        let run = Arc::new(FakeRunner::with(&["pacman", "flatpak"]));
        run.on(
            "pacman -Ss",
            0,
            "extra/gimp 3.0.4-1\n    GNU Image Manipulation Program\nextra/gimp-help 1-1\n    Help\n",
        );
        run.on("pacman -Q", 0, "gimp-help 1-1\n");
        run.on("pacman -Qm", 0, "");
        let s = store(run, Origin::Pacman);
        let xml = r#"<components origin="o"><component type="desktop-application"><id>org.gimp.GIMP</id><pkgname>gimp</pkgname><name>GIMP</name><summary>Edit</summary></component></components>"#;
        s.set_catalog(Catalog::from_components(
            crate::appstream::parse_xml(xml, std::path::Path::new("/x"), "", ""),
            vec![],
        ));
        s.prefs.lock().unwrap().use_aur = false;
        let r = s.search("gimp");
        assert_eq!(r.apps.iter().map(|p| p.key()).collect::<Vec<_>>(), vec!["pacman:gimp"]);
        assert_eq!(r.apps[0].title, "GIMP");
        assert_eq!(r.packages.iter().map(|p| p.key()).collect::<Vec<_>>(), vec!["pacman:gimp-help"]);
        assert!(r.packages[0].installed);
    }
}
