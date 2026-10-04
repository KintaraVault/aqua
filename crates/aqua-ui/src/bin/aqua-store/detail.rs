use crate::app::{bg, ss, App};
use crate::conv::{self, Cell};
use crate::images::Req;
use aqua_store::model::{Details, Origin, Package, Ratings};
use aqua_store::reviews::Fetched;
use aqua_ui::{tr, trf, SDetail, SInfo, SPerm, SRelease, SReview, SShelf, SShot};
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;

fn cells(v: Vec<Cell>) -> ModelRc<SInfo> {
    ModelRc::new(VecModel::from(
        v.into_iter()
            .map(|c| SInfo {
                label: ss(c.label),
                value: ss(c.value),
                sub: ss(c.sub),
                link: ss(c.link),
                glyph: ss(c.glyph),
                stars: c.stars,
            })
            .collect::<Vec<_>>(),
    ))
}

pub struct Extras {
    ratings: Option<Ratings>,
    fetched: Option<Fetched>,
    perms_live: Vec<(String, String, bool)>,
}

impl App {
    pub fn load_app(&mut self, key: &str) {
        let pkg = self.pkgs.get(key).cloned().or_else(|| {
            aqua_store::model::parse_key(key).map(|(o, n)| {
                let mut p = Package::new(o, n);
                if o == Origin::Flatpak {
                    p.appstream_id = n.into();
                    p.repo = "flathub".into();
                }
                p
            })
        });
        let Some(pkg) = pkg else {
            self.g(|g| g.set_loading(false));
            return;
        };
        self.remember(&pkg);
        self.want_icon(&pkg);
        self.detail = None;
        self.ratings = None;
        self.reviews.clear();
        self.alts.clear();
        self.shots.clear();
        self.shot_model = Rc::new(VecModel::default());
        self.loader.drop_shots(self.gen);
        let app = self.sapp(self.pkgs.get(key).unwrap_or(&pkg));
        let d = SDetail {
            subtitle: ss(&pkg.summary),
            developer: ss(&pkg.developer),
            installed: pkg.installed,
            flatpak: pkg.origin() == Origin::Flatpak,
            source_label: ss(conv::source_label(&pkg)),
            app,
            loaded: false,
            ..Default::default()
        };
        self.g(|g| {
            g.set_detail(d);
            g.set_strip(ModelRc::default());
            g.set_info(ModelRc::default());
            g.set_links(ModelRc::default());
            g.set_shots(ModelRc::from(self.shot_model.clone()));
            g.set_releases(ModelRc::default());
            g.set_reviews(ModelRc::default());
            g.set_histogram(ModelRc::default());
            g.set_perms(ModelRc::default());
            g.set_related(ModelRc::default());
            g.set_sources(ModelRc::default());
            g.set_source_index(0);
        });
        let gen = self.gen;
        let p2 = pkg.clone();
        bg(
            &self.store,
            move |s| {
                let d = s.details(&p2);
                let alts = s.alternatives(&p2);
                (d, alts)
            },
            move |a, (d, alts)| {
                if a.gen != gen {
                    return;
                }
                let d = d.unwrap_or_else(|e| {
                    a.g(|g| g.set_error(ss(&e)));
                    Details { pkg: pkg.clone(), ..Default::default() }
                });
                a.apply_detail(d, alts);
            },
        );
    }

    fn apply_detail(&mut self, d: Details, alts: Vec<Package>) {
        let p = d.pkg.clone();
        let key = p.key();
        self.remember(&p);
        self.want_icon(&p);
        for x in &alts {
            self.remember(x);
        }
        let mut sources = vec![p.clone()];
        for x in alts {
            if x.key() != key && !sources.iter().any(|s| s.key() == x.key()) {
                sources.push(x);
            }
        }
        self.alts = sources;
        let labels: Vec<SharedString> = self
            .alts
            .iter()
            .map(|x| {
                let v = if x.version.is_empty() { String::new() } else { format!(" — {}", x.version) };
                ss(format!("{}{}", conv::source_label(x), v))
            })
            .collect();
        let distro = {
            let e = self.store.env();
            if e.distro_name.is_empty() {
                "Linux".to_string()
            } else {
                e.distro_name.clone()
            }
        };
        let releases: Vec<SRelease> = d
            .releases
            .iter()
            .map(|r| SRelease {
                version: ss(&r.version),
                date: ss(r.timestamp.map(conv::date).unwrap_or_default()),
                notes: ss(aqua_store::model::blocks_to_text(&r.notes)),
            })
            .collect();
        let now = aqua_store::units::now();
        let updated = d
            .releases
            .first()
            .and_then(|r| r.timestamp)
            .or(d.updated)
            .map(|t| conv::relative(t, now))
            .unwrap_or_default();
        let version = if !p.version.is_empty() {
            p.version.clone()
        } else {
            d.releases.first().map(|r| r.version.clone()).unwrap_or_default()
        };
        let perms: Vec<SPerm> = d
            .permissions
            .iter()
            .map(|x| SPerm {
                id: ss(&x.id),
                label: ss(tr(&x.label)),
                detail: ss(tr(&x.detail)),
                glyph: ss(conv::perm_glyph(&x.id)),
                risky: x.risky,
                on: true,
            })
            .collect();
        let (cat, cat_glyph) = conv::category_of(&p);
        let show_reviews = self.store.prefs().show_reviews;
        let sd = SDetail {
            app: self.sapp(self.pkgs.get(&key).unwrap_or(&p)),
            subtitle: ss(&p.summary),
            developer: ss(if !p.developer.is_empty() { &p.developer } else { &d.maintainer }),
            description: ss(aqua_store::model::blocks_to_text(&d.description)),
            version: ss(&version),
            updated: ss(updated),
            notes: ss(conv::notes_text(&d)),
            age: ss(conv::age_label(d.age)),
            category: ss(cat),
            category_glyph: ss(cat_glyph),
            homepage: ss(d.link("homepage").unwrap_or(&p.homepage)),
            web_url: ss(conv::web_url(&p)),
            source_label: ss(conv::source_label(&p)),
            license: ss(&p.license),
            has_releases: !d.releases.is_empty(),
            has_ratings: false,
            has_perms: !perms.is_empty(),
            can_manage_perms: p.origin() == Origin::Flatpak && p.installed,
            can_review: show_reviews && p.installed,
            installed: p.installed,
            flatpak: p.origin() == Origin::Flatpak,
            loaded: true,
            ..Default::default()
        };
        self.shots = d.screenshots.clone();
        let shot_rows: Vec<SShot> = self
            .shots
            .iter()
            .map(|s| SShot {
                img: Default::default(),
                loaded: false,
                ratio: conv::shot_ratio(s.width, s.height),
                caption: ss(&s.caption),
            })
            .collect();
        self.shot_model.set_vec(shot_rows);
        for (i, s) in self.shots.iter().enumerate().take(12) {
            let url = if !s.thumb.is_empty() && s.width >= 700 {
                s.thumb.clone()
            } else if !s.full.is_empty() {
                s.full.clone()
            } else {
                s.thumb.clone()
            };
            self.loader.push_front(Req::Shot { gen: self.gen, index: i, url, max_h: 640 });
        }
        let strip = conv::strip(&d, None);
        let info = conv::info(&d, &distro);
        let links = conv::links(&d);
        let si = self.alts.iter().position(|x| x.key() == key).unwrap_or(0) as i32;
        self.g(|g| {
            g.set_detail(sd);
            g.set_strip(cells(strip));
            g.set_info(cells(info));
            g.set_links(cells(links));
            g.set_releases(ModelRc::new(VecModel::from(releases)));
            g.set_perms(ModelRc::new(VecModel::from(perms)));
            g.set_sources(ModelRc::new(VecModel::from(labels)));
            g.set_source_index(si);
            g.set_loading(false);
        });
        self.detail = Some(d);
        self.refresh_detail_extras(&key);
        self.load_related();
    }

    pub fn refresh_detail_extras(&mut self, key: &str) {
        let Some(d) = self.detail.clone() else { return };
        if d.pkg.key() != key {
            return;
        }
        let mut pkg = d.pkg.clone();
        if let Some(p) = self.pkgs.get(key) {
            pkg.installed = p.installed;
            pkg.installed_version = p.installed_version.clone();
            pkg.scope = pkg.scope.or(p.scope);
        }
        let gen = self.gen;
        let show = self.store.prefs().show_reviews;
        let rid = if !pkg.appstream_id.is_empty() { pkg.appstream_id.clone() } else { pkg.name.clone() };
        let ver = if pkg.installed_version.is_empty() { pkg.version.clone() } else { pkg.installed_version.clone() };
        bg(
            &self.store,
            move |s| {
                let (ratings, fetched) = if show && pkg.is_app_like() {
                    std::thread::scope(|sc| {
                        let r = sc.spawn(|| s.reviews.ratings(&rid).ok());
                        let f = sc.spawn(|| s.reviews.fetch(&rid, &ver, 30).ok());
                        (r.join().ok().flatten(), f.join().ok().flatten())
                    })
                } else {
                    (None, None)
                };
                Extras { ratings, fetched, perms_live: s.permissions(&pkg) }
            },
            move |a, x| {
                if a.gen != gen {
                    return;
                }
                a.apply_extras(x);
            },
        );
    }

    fn apply_extras(&mut self, x: Extras) {
        let Some(d) = self.detail.clone() else { return };
        self.ratings = x.ratings.clone();
        let mut sd = self.g(|g| g.get_detail()).unwrap_or_default();
        if let Some(r) = x.ratings.as_ref().filter(|r| r.total() > 0) {
            sd.has_ratings = true;
            sd.rating = ss(format!("{:.1}", r.average()));
            sd.stars = r.average();
            sd.rating_count = ss(aqua_ui::ntr("{n} Rating", "{n} Ratings", r.total() as i64));
            let hist: Vec<f32> = (0..5).map(|i| r.fraction(5 - i)).collect();
            self.g(|g| g.set_histogram(ModelRc::new(VecModel::from(hist))));
        } else if sd.can_review {
            sd.has_ratings = true;
            sd.rating = ss("–");
            sd.rating_count = ss(tr("Not enough ratings"));
            self.g(|g| g.set_histogram(ModelRc::new(VecModel::from(vec![0f32; 5]))));
        }
        if let Some(f) = x.fetched {
            self.skey = f.user_skey.clone();
            self.reviews = f.reviews;
            self.apply_reviews();
            if !self.reviews.is_empty() {
                sd.has_ratings = true;
            }
        }
        let p = self.pkgs.get(&d.pkg.key()).cloned().unwrap_or(d.pkg.clone());
        sd.can_manage_perms = p.origin() == Origin::Flatpak && p.installed && !x.perms_live.is_empty();
        sd.installed = p.installed;
        sd.can_review = self.store.prefs().show_reviews && p.installed && !self.skey.is_empty();
        let strip = conv::strip(&d, self.ratings.as_ref());
        self.g(|g| {
            g.set_detail(sd);
            g.set_strip(cells(strip));
        });
    }

    pub fn apply_reviews(&mut self) {
        let now = aqua_store::units::now();
        let rows: Vec<SReview> = self
            .reviews
            .iter()
            .map(|r| SReview {
                id: r.id as i32,
                author: ss(if r.author.is_empty() { tr("Anonymous") } else { &r.author }),
                title: ss(&r.summary),
                body: ss(&r.body),
                stars: r.stars() as i32,
                date: ss(if r.date > 0 { conv::relative(r.date, now) } else { String::new() }),
                version: ss(&r.version),
                up: r.karma_up as i32,
                down: r.karma_down as i32,
                mine: r.mine,
                voted: self.voted.contains(&r.id),
            })
            .collect();
        self.g(|g| g.set_reviews(ModelRc::new(VecModel::from(rows))));
    }

    fn load_related(&mut self) {
        let Some(d) = self.detail.clone() else { return };
        let gen = self.gen;
        let p = d.pkg.clone();
        bg(
            &self.store,
            move |s| {
                let mut out: Vec<(String, String, Vec<Package>)> = vec![];
                let key = p.key();
                if p.origin() == Origin::Flatpak && !p.developer.is_empty() {
                    let v: Vec<Package> =
                        s.developer(&p.developer).into_iter().filter(|x| x.key() != key).take(6).collect();
                    if !v.is_empty() {
                        out.push((trf("More by {d}", &[("d", &p.developer)]), String::new(), v));
                    }
                }
                if let Some(cid) = conv::category_id(&p) {
                    if let Some(c) = aqua_store::sections::category(cid) {
                        let v: Vec<Package> = s
                            .feed(&c.feed, 1, 18)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|x| x.key() != key && !out.iter().any(|o| o.2.iter().any(|y| y.key() == x.key())))
                            .take(6)
                            .collect();
                        if !v.is_empty() {
                            out.push((tr("You Might Also Like").to_string(), format!("category:{cid}"), v));
                        }
                    }
                }
                out
            },
            move |a, out| {
                if a.gen != gen {
                    return;
                }
                let rows: Vec<SShelf> = out
                    .iter()
                    .map(|(t, id, v)| SShelf {
                        id: ss(id),
                        title: ss(t),
                        subtitle: SharedString::new(),
                        apps: a.model(v),
                    })
                    .collect();
                a.g(|g| g.set_related(ModelRc::new(VecModel::from(rows))));
            },
        );
    }

    pub fn shot_ready(&mut self, gen: u64, index: usize, img: slint::Image) {
        if gen != self.gen {
            return;
        }
        use slint::Model;
        if let Some(mut r) = self.shot_model.row_data(index) {
            r.img = img;
            r.loaded = true;
            self.shot_model.set_row_data(index, r);
        }
    }

    pub fn source_changed(&mut self, i: i32) {
        if let Some(p) = self.alts.get(i.max(0) as usize).cloned() {
            let k = p.key();
            self.remember(&p);
            self.route = crate::conv::Route::App(k.clone());
            self.gen += 1;
            self.load_app(&k);
        }
    }
}

trait AppLike {
    fn is_app_like(&self) -> bool;
}

impl AppLike for Package {
    fn is_app_like(&self) -> bool {
        self.origin() == Origin::Flatpak || self.is_app || !self.appstream_id.is_empty()
    }
}
