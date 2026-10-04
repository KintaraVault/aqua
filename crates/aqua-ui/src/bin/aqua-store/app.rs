use crate::conv::{self, JobState, Route};
use crate::images::{Loader, Req};
use aqua_store::history;
use aqua_store::jobs::Jobs;
use aqua_store::model::{Details, Origin, Package, Ratings, Review, Update};
use aqua_store::sections::{self, Feed};
use aqua_store::setup::Requirement;
use aqua_store::store::Store;
use aqua_ui::{tr, trf, SApp, SCat, SHero, SHist, SShelf, SUpd, StoreWindow, AS};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

pub fn install(app: App) -> Rc<RefCell<App>> {
    let rc = Rc::new(RefCell::new(app));
    APP.with(|a| *a.borrow_mut() = Some(rc.clone()));
    rc
}

pub fn with(f: impl FnOnce(&mut App) + 'static) {
    let Some(app) = APP.with(|a| a.borrow().clone()) else { return };
    let busy = app.try_borrow_mut().is_err();
    if busy {
        slint::Timer::single_shot(Duration::ZERO, move || with(f));
        return;
    }
    f(&mut app.borrow_mut());
}

pub fn bg<T: Send + 'static>(
    store: &Arc<Store>,
    work: impl FnOnce(&Store) -> T + Send + 'static,
    done: impl FnOnce(&mut App, T) + Send + 'static,
) {
    let s = store.clone();
    let _ = std::thread::Builder::new().name("store-bg".into()).spawn(move || {
        let t = work(&s);
        let _ = slint::invoke_from_event_loop(move || with(move |a| done(a, t)));
    });
}

pub fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

pub fn color(rgb: u32) -> slint::Color {
    slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

#[derive(Clone, Debug)]
pub enum Pending {
    Install(String),
    Remove(String, bool),
    Reinstall(String),
    Update(Vec<String>),
}

#[derive(Clone, Default)]
pub struct Home {
    pub heroes: Vec<(String, Details)>,
    pub shelves: Vec<(String, String, Vec<Package>)>,
    pub error: String,
}

pub struct App {
    pub ui: slint::Weak<StoreWindow>,
    pub store: Arc<Store>,
    pub jobs: Jobs,
    pub loader: Loader,
    pub pkgs: HashMap<String, Package>,
    pub icons: HashMap<String, slint::Image>,
    requested: HashSet<String>,
    models: Vec<Weak<VecModel<SApp>>>,
    pub heroes: Rc<VecModel<SHero>>,
    pub hero_keys: Vec<String>,
    pub upd_model: Rc<VecModel<SUpd>>,
    pub sys_model: Rc<VecModel<SUpd>>,
    pub recent_model: Rc<VecModel<SUpd>>,
    pub installed_model: Rc<VecModel<SApp>>,
    pub job_state: HashMap<String, JobState>,
    pub groups: HashMap<String, Vec<String>>,
    pub route: Route,
    pub stack: Vec<Route>,
    /// Pages left with Back (Forward returns to them); cleared by a new navigation.
    pub fwd: Vec<Route>,
    pub gen: u64,
    pub detail: Option<Details>,
    pub ratings: Option<Ratings>,
    pub reviews: Vec<Review>,
    pub skey: String,
    pub alts: Vec<Package>,
    pub list_feed: Option<Feed>,
    pub list_page: u32,
    pub list_more: bool,
    pub list_loading: bool,
    pub list_model: Rc<VecModel<SApp>>,
    pub updates: Vec<Update>,
    pub update_notes: HashMap<String, (String, String)>,
    pub pending: Option<Pending>,
    pub confirm: Option<Pending>,
    pub menu_key: String,
    pub reqs: Vec<Requirement>,
    pub req_state: HashMap<String, i32>,
    pub need: Option<Requirement>,
    pub toast_timer: slint::Timer,
    pub suggest_timer: slint::Timer,
    pub suggest_gen: u64,
    pub home_cache: HashMap<String, Home>,
    pub installed: Vec<Package>,
    pub filters: Vec<&'static str>,
    pub voted: HashSet<i64>,
    pub shots: Vec<aqua_store::model::Screenshot>,
    pub shot_model: Rc<VecModel<aqua_ui::SShot>>,
    pub history: Vec<history::Entry>,
}

pub const PER_PAGE: u32 = 30;

impl App {
    pub fn new(ui: &StoreWindow, store: Arc<Store>, jobs: Jobs, loader: Loader) -> App {
        App {
            ui: ui.as_weak(),
            store,
            jobs,
            loader,
            pkgs: HashMap::new(),
            icons: HashMap::new(),
            requested: HashSet::new(),
            models: vec![],
            heroes: Rc::new(VecModel::default()),
            hero_keys: vec![],
            upd_model: Rc::new(VecModel::default()),
            sys_model: Rc::new(VecModel::default()),
            recent_model: Rc::new(VecModel::default()),
            installed_model: Rc::new(VecModel::default()),
            job_state: HashMap::new(),
            groups: HashMap::new(),
            route: Route::Home("discover"),
            stack: vec![],
            fwd: vec![],
            gen: 0,
            detail: None,
            ratings: None,
            reviews: vec![],
            skey: String::new(),
            alts: vec![],
            list_feed: None,
            list_page: 1,
            list_more: false,
            list_loading: false,
            list_model: Rc::new(VecModel::default()),
            updates: vec![],
            update_notes: HashMap::new(),
            pending: None,
            confirm: None,
            menu_key: String::new(),
            reqs: vec![],
            req_state: HashMap::new(),
            need: None,
            toast_timer: slint::Timer::default(),
            suggest_timer: slint::Timer::default(),
            suggest_gen: 0,
            home_cache: HashMap::new(),
            installed: vec![],
            filters: vec![],
            voted: HashSet::new(),
            shots: vec![],
            shot_model: Rc::new(VecModel::default()),
            history: vec![],
        }
    }

    pub fn ui(&self) -> Option<StoreWindow> {
        self.ui.upgrade()
    }

    pub fn g<R>(&self, f: impl FnOnce(&AS) -> R) -> Option<R> {
        self.ui().map(|u| f(&u.global::<AS>()))
    }

    pub fn toast(&mut self, text: &str) {
        self.g(|g| g.set_toast(ss(text)));
        let w = self.ui.clone();
        self.toast_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(2600), move || {
            if let Some(u) = w.upgrade() {
                u.global::<AS>().set_toast(SharedString::new());
            }
        });
    }

    pub fn openable(p: &Package) -> bool {
        p.installed && (p.is_app || p.origin() == Origin::Flatpak || !p.desktop_id.is_empty())
    }

    pub fn job_for(&self, key: &str) -> Option<&JobState> {
        self.job_state.get(key).or_else(|| {
            self.groups.iter().find(|(_, m)| m.iter().any(|k| k == key)).and_then(|(g, _)| self.job_state.get(g))
        })
    }

    pub fn sapp(&self, p: &Package) -> SApp {
        let key = p.key();
        let name = p.display_name().to_string();
        let (state, progress, status) = conv::state_of(p, Self::openable(p), self.job_for(&key));
        let icon = self.icons.get(&key).cloned();
        SApp {
            key: ss(&key),
            name: ss(&name),
            summary: ss(&p.summary),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            letter: ss(conv::letter(&name)),
            tint: color(conv::tint(&name)),
            source: ss(conv::source_label(p)),
            state,
            progress,
            status: ss(status),
            verified: p.verified,
            category: ss(conv::category_of(p).0),
            detail: SharedString::new(),
            badge: SharedString::new(),
        }
    }

    pub fn remember(&mut self, p: &Package) {
        let k = p.key();
        match self.pkgs.get_mut(&k) {
            Some(old) => {
                let mut n = p.clone();
                n.merge_from(old);
                if n.icon.is_none() {
                    n.icon = old.icon.clone();
                }
                *old = n;
            }
            None => {
                self.pkgs.insert(k, p.clone());
            }
        }
    }

    pub fn want_icon(&mut self, p: &Package) {
        let k = p.key();
        if p.icon.is_none() || self.icons.contains_key(&k) || !self.requested.insert(k.clone()) {
            return;
        }
        let id = [&p.desktop_id, &p.appstream_id, &p.name]
            .iter()
            .find(|s| !s.is_empty())
            .map(|s| s.trim_end_matches(".desktop").to_string())
            .unwrap_or_default();
        self.loader.push(Req::Icon { key: k, id, name: p.display_name().to_string(), icon: p.icon.clone() });
    }

    pub fn model(&mut self, v: &[Package]) -> ModelRc<SApp> {
        for p in v {
            self.remember(p);
            self.want_icon(p);
        }
        let rows: Vec<SApp> = v.iter().map(|p| self.sapp(self.pkgs.get(&p.key()).unwrap_or(p))).collect();
        let m = Rc::new(VecModel::from(rows));
        self.models.retain(|w| w.strong_count() > 0);
        self.models.push(Rc::downgrade(&m));
        ModelRc::from(m)
    }

    pub fn track(&mut self, m: &Rc<VecModel<SApp>>) {
        self.models.retain(|w| w.strong_count() > 0);
        if !self.models.iter().any(|w| w.as_ptr() == Rc::as_ptr(m)) {
            self.models.push(Rc::downgrade(m));
        }
    }

    pub fn refresh_key(&mut self, key: &str) {
        let Some(p) = self.pkgs.get(key).cloned() else { return };
        let fresh = self.sapp(&p);
        let patch = |row: &mut SApp| {
            let detail = row.detail.clone();
            let badge = row.badge.clone();
            let summary = row.summary.clone();
            *row = fresh.clone();
            row.detail = detail;
            row.badge = badge;
            if !summary.is_empty() {
                row.summary = summary;
            }
        };
        self.models.retain(|w| w.strong_count() > 0);
        for w in &self.models {
            if let Some(m) = w.upgrade() {
                for i in 0..m.row_count() {
                    if let Some(mut r) = m.row_data(i) {
                        if r.key == key {
                            patch(&mut r);
                            m.set_row_data(i, r);
                        }
                    }
                }
            }
        }
        for i in 0..self.heroes.row_count() {
            if let Some(mut h) = self.heroes.row_data(i) {
                if h.app.key == key {
                    patch(&mut h.app);
                    self.heroes.set_row_data(i, h);
                }
            }
        }
        for m in [&self.upd_model, &self.sys_model, &self.recent_model] {
            for i in 0..m.row_count() {
                if let Some(mut u) = m.row_data(i) {
                    if u.app.key == key {
                        patch(&mut u.app);
                        m.set_row_data(i, u);
                    }
                }
            }
        }
        if let Some(u) = self.ui() {
            let g = u.global::<AS>();
            let mut d = g.get_detail();
            if d.app.key == key {
                patch(&mut d.app);
                d.installed = p.installed;
                g.set_detail(d);
            }
        }
    }

    pub fn refresh_all(&mut self) {
        let keys: Vec<String> = self.pkgs.keys().cloned().collect();
        for k in keys {
            self.refresh_key(&k);
        }
    }

    pub fn icon_ready(&mut self, key: String, img: slint::Image) {
        self.icons.insert(key.clone(), img);
        self.refresh_key(&key);
    }

    pub fn page_title(r: &Route) -> String {
        match r {
            Route::Home(id) => match *id {
                "discover" => tr("Discover").to_string(),
                s => sections::section(s).map(|x| tr(x.title).to_string()).unwrap_or_default(),
            },
            Route::Categories => tr("Categories").to_string(),
            Route::Updates => tr("Updates").to_string(),
            Route::Account => tr("Account").to_string(),
            Route::Feed { title, .. } => title.clone(),
            Route::Category(id) => {
                sections::category(id).map(|c| tr(c.title).to_string()).unwrap_or_else(|| id.clone())
            }
            Route::Developer(n) => n.clone(),
            Route::Search(q) => trf("Results for “{q}”", &[("q", q)]),
            Route::App(_) => String::new(),
        }
    }

    pub fn navigate(&mut self, i: usize) {
        self.stack.clear();
        self.fwd.clear();
        let r = Route::nav(i);
        self.show(r, false);
    }

    pub fn open(&mut self, r: Route) {
        if r == self.route {
            return;
        }
        self.fwd.clear();
        self.show(r, true);
    }

    pub fn back(&mut self) {
        if let Some(r) = self.stack.pop() {
            self.fwd.push(self.route.clone());
            self.show(r, false);
        }
    }

    pub fn forward(&mut self) {
        if let Some(r) = self.fwd.pop() {
            let fwd = std::mem::take(&mut self.fwd);
            self.show(r, true);
            self.fwd = fwd;
            let f = !self.fwd.is_empty();
            self.g(|g| g.set_can_forward(f));
        }
    }

    pub fn show(&mut self, r: Route, push: bool) {
        if push {
            let old = self.route.clone();
            self.stack.push(old);
            if self.stack.len() > 40 {
                self.stack.remove(0);
            }
        }
        self.route = r.clone();
        self.gen += 1;
        let title = Self::page_title(&r);
        let back = !self.stack.is_empty();
        let fwd = !self.fwd.is_empty();
        self.g(|g| {
            g.set_menu_open(false);
            g.set_page(ss(r.page()));
            g.set_nav(r.nav_index());
            g.set_can_back(back);
            g.set_can_forward(fwd);
            g.set_title(ss(&title));
            g.set_subtitle(SharedString::new());
            g.set_error(SharedString::new());
            g.set_loading(true);
            g.set_scroll_y(0.0);
        });
        if !matches!(r, Route::App(_)) {
            self.detail = None;
        }
        match r {
            Route::Home(id) => self.load_home(id),
            Route::Categories => self.load_categories(),
            Route::Updates => self.load_updates(false),
            Route::Account => self.load_account(),
            Route::Feed { id, .. } => match sections::feed_from_id(&id) {
                Some(f) => self.load_list(f),
                None => self.g(|g| g.set_loading(false)).unwrap_or(()),
            },
            Route::Category(id) => match sections::category(&id) {
                Some(c) => self.load_list(c.feed.clone()),
                None => match sections::feed_from_id(&id) {
                    Some(f) => self.load_list(f),
                    None => self.g(|g| g.set_loading(false)).unwrap_or(()),
                },
            },
            Route::Developer(name) => self.load_developer(name),
            Route::Search(q) => self.load_search(q),
            Route::App(key) => self.load_app(&key),
        }
    }

    pub fn reload(&mut self) {
        let r = self.route.clone();
        if let Route::Home(id) = &r {
            self.home_cache.remove(*id);
        }
        if r == Route::Categories {
            self.home_cache.remove("categories");
        }
        self.show(r, false);
    }

    fn set_shelves(&mut self, shelves: &[(String, String, Vec<Package>)]) {
        let rows: Vec<SShelf> = shelves
            .iter()
            .map(|(title, id, apps)| SShelf {
                id: ss(id),
                title: ss(title),
                subtitle: SharedString::new(),
                apps: self.model(apps),
            })
            .collect();
        self.g(|g| g.set_shelves(ModelRc::new(VecModel::from(rows))));
    }

    fn apply_home(&mut self, h: &Home) {
        self.hero_keys.clear();
        let mut rows = vec![];
        for (label, d) in &h.heroes {
            self.remember(&d.pkg);
            self.want_icon(&d.pkg);
            let p = self.pkgs.get(&d.pkg.key()).cloned().unwrap_or_else(|| d.pkg.clone());
            let mut app = self.sapp(&p);
            app.summary = app.category.clone();
            let dark = self.ui().map(|u| u.global::<aqua_ui::Theme>().get_dark()).unwrap_or(false);
            let brand =
                conv::parse_color(if dark && !d.brand_dark.is_empty() { &d.brand_dark } else { &d.brand_light })
                    .unwrap_or_else(|| conv::tint(p.display_name()));
            let key = p.key();
            let subtitle = {
                let t = aqua_store::model::blocks_to_text(&d.description);
                let first = t.split(['\n']).next().unwrap_or("").to_string();
                let mut s: String = first.chars().take(110).collect();
                if first.chars().count() > 110 {
                    s.push('…');
                }
                s
            };
            let hk = format!("hero:{key}");
            let art = self.icons.get(&hk).cloned();
            if art.is_none() && self.requested.insert(hk.clone()) {
                if let Some(s) = d.screenshots.first() {
                    let url = if s.width >= 900 || s.full.is_empty() { s.thumb.clone() } else { s.full.clone() };
                    let url = if url.is_empty() { s.full.clone() } else { url };
                    self.loader.push_front(Req::Art { key: hk.clone(), url });
                }
            }
            rows.push(SHero {
                label: ss(label.to_uppercase()),
                title: ss(if p.summary.is_empty() { p.display_name() } else { &p.summary }),
                subtitle: ss(subtitle),
                has_art: art.is_some(),
                art: art.unwrap_or_default(),
                brand: color(brand),
                app,
            });
            self.hero_keys.push(hk);
        }
        self.heroes.set_vec(rows);
        self.g(|g| g.set_heroes(ModelRc::from(self.heroes.clone())));
        self.set_shelves(&h.shelves);
        let err = h.error.clone();
        self.g(|g| {
            g.set_loading(false);
            g.set_error(ss(err));
        });
    }

    pub fn art_ready(&mut self, key: &str, img: slint::Image) {
        self.icons.insert(key.to_string(), img.clone());
        if let Some(i) = self.hero_keys.iter().position(|k| k == key) {
            if let Some(mut h) = self.heroes.row_data(i) {
                h.art = img;
                h.has_art = true;
                self.heroes.set_row_data(i, h);
            }
        }
    }

    fn load_home(&mut self, id: &'static str) {
        self.heroes.set_vec(vec![]);
        self.g(|g| {
            g.set_heroes(ModelRc::default());
            g.set_shelves(ModelRc::default());
        });
        if let Some(h) = self.home_cache.get(id).cloned() {
            self.apply_home(&h);
            return;
        }
        let gen = self.gen;
        bg(
            &self.store,
            move |s| build_home(s, id),
            move |a, h| {
                if h.error.is_empty() {
                    a.home_cache.insert(id.to_string(), h.clone());
                }
                if a.gen == gen {
                    a.apply_home(&h);
                }
            },
        );
    }

    fn load_categories(&mut self) {
        let cats: Vec<SCat> = sections::CATEGORIES
            .iter()
            .map(|c| SCat { id: ss(c.id), title: ss(tr(c.title)), glyph: ss(c.glyph), tint: color(c.tint) })
            .collect();
        let mut cats = cats;
        cats.sort_by_key(|c| c.title.to_lowercase());
        self.g(|g| {
            g.set_cats(ModelRc::new(VecModel::from(cats)));
            g.set_shelves(ModelRc::default());
        });
        if let Some(h) = self.home_cache.get("categories").cloned() {
            self.set_shelves(&h.shelves);
            self.g(|g| g.set_loading(false));
            return;
        }
        let gen = self.gen;
        bg(
            &self.store,
            |s| {
                let pick: [(&str, Feed); 2] = [
                    ("Editors’ Choice: Verified Apps", Feed::Collection("verified")),
                    ("New Apps We Love", Feed::Collection("recently-added")),
                ];
                let mut out = vec![];
                for (t, f) in pick {
                    if let Ok(v) = s.feed(&f, 1, 12) {
                        if !v.is_empty() {
                            out.push((tr(t).to_string(), f.id(), v.into_iter().take(8).collect::<Vec<_>>()));
                        }
                    }
                }
                Home { shelves: out, ..Default::default() }
            },
            move |a, h| {
                a.home_cache.insert("categories".to_string(), h.clone());
                if a.gen == gen {
                    a.set_shelves(&h.shelves);
                    a.g(|g| g.set_loading(false));
                }
            },
        );
    }

    fn load_list(&mut self, feed: Feed) {
        self.list_feed = Some(feed.clone());
        self.list_page = 1;
        self.list_more = false;
        self.list_loading = false;
        self.list_model = Rc::new(VecModel::default());
        let m = self.list_model.clone();
        self.track(&m);
        self.g(|g| {
            g.set_list(ModelRc::from(m));
            g.set_list_more(false);
            g.set_list_loading(false);
        });
        self.fetch_page();
    }

    pub fn fetch_page(&mut self) {
        let Some(feed) = self.list_feed.clone() else { return };
        let page = self.list_page;
        let gen = self.gen;
        self.list_loading = true;
        if page > 1 {
            self.g(|g| g.set_list_loading(true));
        }
        bg(
            &self.store,
            move |s| s.feed(&feed, page, PER_PAGE),
            move |a, res| {
                if a.gen != gen {
                    return;
                }
                a.list_loading = false;
                match res {
                    Ok(v) => {
                        a.list_more = v.len() as u32 >= PER_PAGE;
                        let have: HashSet<String> = (0..a.list_model.row_count())
                            .filter_map(|i| a.list_model.row_data(i).map(|r| r.key.to_string()))
                            .collect();
                        for p in v.iter().filter(|p| !have.contains(&p.key())) {
                            a.remember(p);
                            a.want_icon(p);
                            let row = a.sapp(a.pkgs.get(&p.key()).unwrap_or(p));
                            a.list_model.push(row);
                        }
                        let more = a.list_more;
                        a.g(|g| {
                            g.set_list_more(more);
                            g.set_error(SharedString::new());
                        });
                    }
                    Err(e) => {
                        a.list_more = false;
                        a.g(|g| g.set_error(ss(e)));
                    }
                }
                a.g(|g| {
                    g.set_loading(false);
                    g.set_list_loading(false);
                });
            },
        );
    }

    pub fn load_more(&mut self) {
        if self.list_more && !self.list_loading && self.route.page() == "list" {
            self.list_page += 1;
            self.fetch_page();
        }
    }

    fn load_developer(&mut self, name: String) {
        self.list_feed = None;
        self.list_more = false;
        self.list_model = Rc::new(VecModel::default());
        let m = self.list_model.clone();
        self.track(&m);
        self.g(|g| {
            g.set_list(ModelRc::from(m));
            g.set_subtitle(ss(tr("Developer")));
            g.set_list_more(false);
        });
        let gen = self.gen;
        bg(
            &self.store,
            move |s| s.developer(&name),
            move |a, v| {
                if a.gen != gen {
                    return;
                }
                for p in &v {
                    a.remember(p);
                    a.want_icon(p);
                    let row = a.sapp(a.pkgs.get(&p.key()).unwrap_or(p));
                    a.list_model.push(row);
                }
                a.g(|g| g.set_loading(false));
            },
        );
    }

    fn load_search(&mut self, q: String) {
        self.g(|g| {
            g.set_searching(true);
            g.set_results(ModelRc::default());
            g.set_packages(ModelRc::default());
            g.set_suggestions(ModelRc::default());
        });
        let gen = self.gen;
        bg(
            &self.store,
            move |s| s.search(&q),
            move |a, r| {
                if a.gen != gen {
                    return;
                }
                let apps: Vec<Package> = r.apps.into_iter().take(80).collect();
                let pkgs: Vec<Package> = r.packages.into_iter().take(80).collect();
                let am = a.model(&apps);
                let pm = a.model(&pkgs);
                a.g(|g| {
                    g.set_results(am);
                    g.set_packages(pm);
                    g.set_searching(false);
                    g.set_loading(false);
                });
            },
        );
    }

    pub fn suggest(&mut self, text: String) {
        self.suggest_gen += 1;
        let gen = self.suggest_gen;
        if text.trim().chars().count() < 2 {
            self.g(|g| g.set_suggestions(ModelRc::default()));
            return;
        }
        let store = self.store.clone();
        self.suggest_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(260), move || {
            let q = text.clone();
            bg(
                &store,
                move |s| {
                    let mut v = if s.prefs().use_flathub { s.flathub.search(&q).unwrap_or_default() } else { vec![] };
                    if v.len() < 6 {
                        let cat = s.catalog();
                        let native = s.env().native;
                        for c in cat.search(&q, false).into_iter().take(6) {
                            if let Some(p) = c.package(native) {
                                v.push(p);
                            }
                        }
                    }
                    v.truncate(6);
                    s.mark(&mut v);
                    v
                },
                move |a, v| {
                    if a.suggest_gen == gen {
                        let m = a.model(&v);
                        a.g(|g| g.set_suggestions(m));
                    }
                },
            );
        });
    }

    pub fn load_updates(&mut self, force: bool) {
        self.refresh_recent();
        if !force {
            if let Some(u) = self.store.cached_updates() {
                self.apply_updates(u.to_vec());
                self.g(|g| g.set_loading(false));
                return;
            }
        }
        self.g(|g| g.set_checking(true));
        bg(
            &self.store,
            move |s| {
                if force {
                    s.invalidate();
                }
                let u = s.updates().to_vec();
                let idx = s.index();
                (u, idx.apps.clone())
            },
            |a, (u, apps)| {
                for p in &apps {
                    a.remember(p);
                }
                let mut prefs = a.store.prefs();
                prefs.last_check = aqua_store::units::now();
                a.store.set_prefs(prefs);
                a.apply_updates(u);
                a.remark();
                a.g(|g| {
                    g.set_checking(false);
                    g.set_loading(false);
                });
            },
        );
    }

    pub fn last_check_text(&self) -> String {
        let t = self.store.prefs().last_check;
        if t <= 0 {
            return String::new();
        }
        let now = aqua_store::units::now();
        if now - t < 120 {
            tr("Last checked just now").to_string()
        } else if now - t < 3600 {
            trf("Last checked {n} min ago", &[("n", &((now - t) / 60))])
        } else {
            trf("Last checked {d}", &[("d", &conv::relative(t, now).to_lowercase())])
        }
    }

    pub fn apply_updates(&mut self, ups: Vec<Update>) {
        self.updates = ups.clone();
        let hidden: HashSet<String> = self.store.prefs().hidden.into_iter().collect();
        let mut apps = vec![];
        let mut sys = vec![];
        let mut want_notes = vec![];
        for u in ups.iter().filter(|u| !hidden.contains(&u.key())) {
            let key = u.key();
            let mut p = self.pkgs.get(&key).cloned().unwrap_or_else(|| {
                let mut p = Package::new(u.origin(), &u.name);
                p.installed = true;
                p.installed_version = u.from.clone();
                p.repo = u.repo.clone();
                p.scope = u.scope;
                p
            });
            p.installed = true;
            p.update_version = u.to.clone();
            if p.installed_version.is_empty() {
                p.installed_version = u.from.clone();
            }
            p.is_app = p.is_app || u.is_app;
            self.remember(&p);
            self.want_icon(&p);
            let (notes, date) = self.update_notes.get(&key).cloned().unwrap_or_default();
            let row = SUpd {
                app: self.sapp(&p),
                from: ss(&u.from),
                to: ss(&u.to),
                notes: ss(notes),
                size: ss(conv::size_text(u.download_size)),
                date: ss(date),
                system: !u.is_app,
            };
            if u.is_app {
                if u.origin() == Origin::Flatpak && !self.update_notes.contains_key(&key) {
                    want_notes.push(u.name.clone());
                }
                apps.push(row);
            } else {
                sys.push(row);
            }
        }
        let badge = apps.len() as i32 + if sys.is_empty() { 0 } else { 1 };
        self.upd_model.set_vec(apps);
        self.sys_model.set_vec(sys);
        let last = self.last_check_text();
        self.g(|g| {
            g.set_updates(ModelRc::from(self.upd_model.clone()));
            g.set_system_updates(ModelRc::from(self.sys_model.clone()));
            g.set_badge(badge);
            g.set_last_check(ss(last));
        });
        if !want_notes.is_empty() {
            bg(
                &self.store,
                move |s| {
                    std::thread::scope(|sc| {
                        let hs: Vec<_> = want_notes
                            .iter()
                            .map(|id| sc.spawn(move || s.flathub.details(id).ok().map(|d| (id.clone(), d))))
                            .collect();
                        hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect::<Vec<_>>()
                    })
                },
                |a, v: Vec<(String, Details)>| {
                    let now = aqua_store::units::now();
                    for (id, d) in v {
                        let key = aqua_store::model::key_of(Origin::Flatpak, &id);
                        let date = d
                            .releases
                            .first()
                            .and_then(|r| r.timestamp)
                            .map(|t| conv::relative(t, now))
                            .unwrap_or_default();
                        a.update_notes.insert(key.clone(), (conv::notes_text(&d), date));
                        let mut p = d.pkg.clone();
                        if let Some(old) = a.pkgs.get(&key) {
                            p.installed = old.installed;
                            p.installed_version = old.installed_version.clone();
                            p.update_version = old.update_version.clone();
                            p.is_app = true;
                        }
                        a.remember(&p);
                        a.want_icon(&p);
                    }
                    let ups = a.updates.clone();
                    a.apply_updates(ups);
                },
            );
        }
    }

    pub fn refresh_recent(&mut self) {
        self.history = history::load();
        let now = aqua_store::units::now();
        let mut seen = HashSet::new();
        let mut rows = vec![];
        for e in self.history.iter().rev() {
            if e.action != "update" && e.action != "install" {
                continue;
            }
            if now - e.time > 30 * 86400 || !seen.insert(e.key.clone()) {
                continue;
            }
            let p = self.pkgs.get(&e.key).cloned().unwrap_or_else(|| {
                let (o, n) = aqua_store::model::parse_key(&e.key).unwrap_or((Origin::Flatpak, &e.name));
                let mut p = Package::new(o, n);
                p.title = e.name.clone();
                p.installed = true;
                p
            });
            rows.push(SUpd {
                app: self.sapp(&p),
                from: SharedString::new(),
                to: ss(trf("Version {v}", &[("v", &e.version)])),
                notes: SharedString::new(),
                size: SharedString::new(),
                date: ss(conv::relative(e.time, now)),
                system: false,
            });
            if rows.len() >= 8 {
                break;
            }
        }
        self.recent_model.set_vec(rows);
        self.g(|g| g.set_recent(ModelRc::from(self.recent_model.clone())));
    }

    pub fn remark(&mut self) {
        let v: Vec<Package> = self.pkgs.values().cloned().collect();
        bg(
            &self.store,
            move |s| {
                let mut v = v;
                s.mark(&mut v);
                v
            },
            |a, v| {
                for p in v {
                    let k = p.key();
                    if let Some(old) = a.pkgs.get_mut(&k) {
                        old.installed = p.installed;
                        old.installed_version = p.installed_version;
                        old.update_version = p.update_version;
                        if old.desktop_id.is_empty() {
                            old.desktop_id = p.desktop_id;
                        }
                        old.scope = old.scope.or(p.scope);
                    }
                }
                a.refresh_all();
                if a.route == Route::Account {
                    a.apply_installed();
                }
            },
        );
    }

    pub fn after_change(&mut self) {
        bg(
            &self.store,
            |s| {
                s.invalidate();
                let idx = s.index();
                let u = s.updates().to_vec();
                (idx.apps.clone(), u)
            },
            |a, (apps, u)| {
                a.installed = apps.clone();
                for p in &apps {
                    a.remember(p);
                }
                a.apply_updates(u);
                a.remark();
                a.refresh_recent();
                if let Route::App(k) = a.route.clone() {
                    a.refresh_detail_extras(&k);
                }
            },
        );
    }

    fn load_account(&mut self) {
        let (_, full) = aqua_ui::user_names();
        let env = self.store.env();
        let mut f: Vec<(&'static str, String)> = vec![("all", tr("All").to_string())];
        if env.flatpak {
            f.push(("flatpak", "Flatpak".into()));
        }
        if env.native.is_some() {
            f.push(("native", env.native_label()));
        }
        if env.has_aur() {
            f.push(("aur", "AUR".into()));
        }
        self.filters = f.iter().map(|x| x.0).collect();
        let labels: Vec<SharedString> = f.iter().map(|x| ss(&x.1)).collect();
        self.g(|g| {
            g.set_user_name(ss(&full));
            g.set_filters(ModelRc::new(VecModel::from(labels)));
            if g.get_filter() as usize >= f.len() {
                g.set_filter(0);
            }
        });
        if !self.installed.is_empty() {
            self.apply_installed();
        }
        bg(
            &self.store,
            |s| {
                let mut v = s.index().apps.clone();
                s.updates();
                s.mark(&mut v);
                v
            },
            |a, v| {
                a.installed = v;
                for p in a.installed.clone() {
                    a.remember(&p);
                }
                a.apply_installed();
                a.refresh_all();
            },
        );
    }

    pub fn apply_installed(&mut self) {
        let (filter, query) = self.g(|g| (g.get_filter(), g.get_query().to_string())).unwrap_or((0, String::new()));
        let fid = self.filters.get(filter.max(0) as usize).copied().unwrap_or("all");
        let mut v: Vec<Package> =
            self.installed.iter().filter(|p| conv::matches_filter(p, fid, &query)).cloned().collect();
        v.sort_by_key(|p| p.display_name().to_lowercase());
        let mut rows = vec![];
        for p in &v {
            self.want_icon(p);
            let mut r = self.sapp(self.pkgs.get(&p.key()).unwrap_or(p));
            r.detail = ss(conv::row_detail(self.pkgs.get(&p.key()).unwrap_or(p)));
            r.badge = ss(if p.has_update() { tr("Update") } else { "" });
            rows.push(r);
        }
        self.installed_model.set_vec(rows);
        let m = self.installed_model.clone();
        self.track(&m);
        let count = self.installed.len() as i32;
        self.g(|g| {
            g.set_installed(ModelRc::from(m));
            g.set_installed_count(count);
            g.set_loading(false);
        });
    }

    pub fn open_history(&mut self) {
        self.history = history::load();
        self.g(|g| {
            g.set_history_query(SharedString::new());
            g.set_sheet(ss("history"));
        });
        self.apply_history("");
    }

    pub fn apply_history(&mut self, q: &str) {
        let rows: Vec<SHist> = history::filter(&self.history, q, "", 0)
            .into_iter()
            .rev()
            .take(400)
            .map(|e| SHist {
                key: ss(&e.key),
                name: ss(&e.name),
                action: ss(conv::history_action(&e.action)),
                version: ss(&e.version),
                source: ss(&e.source),
                date: ss(conv::date(e.time)),
            })
            .collect();
        self.g(|g| g.set_history(ModelRc::new(VecModel::from(rows))));
    }
}

pub fn build_home(s: &Store, id: &str) -> Home {
    let (hero_label, hero_feed, shelves): (&str, Option<Feed>, Vec<(&str, Feed)>) = if id == "discover" {
        ("", None, sections::DISCOVER.to_vec())
    } else if let Some(sec) = sections::section(id) {
        ("Featured", Some(sec.hero.clone()), sec.shelves.to_vec())
    } else {
        ("", None, vec![])
    };
    let (heroes, lists) = std::thread::scope(|sc| {
        let h = sc.spawn(|| {
            if let Some(f) = &hero_feed {
                let top = s.feed(f, 1, 12).unwrap_or_default();
                let picks: Vec<Package> = top.into_iter().filter(|p| p.origin() == Origin::Flatpak).take(4).collect();
                let ds: Vec<Details> = std::thread::scope(|s2| {
                    let hs: Vec<_> = picks.iter().map(|p| s2.spawn(move || s.details(p).ok())).collect();
                    hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
                });
                ds.into_iter()
                    .filter(|d| !d.screenshots.is_empty())
                    .map(|d| (tr(hero_label).to_string(), d))
                    .collect::<Vec<_>>()
            } else {
                s.featured().into_iter().map(|f| (tr(&f.label).to_string(), f.details)).collect::<Vec<_>>()
            }
        });
        let ls: Vec<_> =
            shelves.iter().map(|(t, f)| sc.spawn(move || (t.to_string(), f.id(), s.feed(f, 1, 24)))).collect();
        (h.join().unwrap_or_default(), ls.into_iter().filter_map(|h| h.join().ok()).collect::<Vec<_>>())
    });
    let mut seen: HashSet<String> = heroes.iter().map(|(_, d)| d.pkg.key()).collect();
    let mut out = Home { heroes, ..Default::default() };
    let mut errors = vec![];
    for (t, fid, r) in lists {
        match r {
            Ok(v) => {
                let picked: Vec<Package> = v.into_iter().filter(|p| seen.insert(p.key())).take(6).collect();
                if !picked.is_empty() {
                    out.shelves.push((tr(&t).to_string(), fid, picked));
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if out.shelves.is_empty() && out.heroes.is_empty() {
        out.error = errors
            .into_iter()
            .next()
            .unwrap_or_else(|| tr("No apps are available. Check your sources in Settings.").to_string());
    }
    out
}
