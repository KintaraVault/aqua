use crate::app::{bg, ss, App, Pending};
use crate::conv::{self, JobState, Route};
use aqua_store::history;
use aqua_store::jobs::{Event, Kind};
use aqua_store::model::{Origin, Package, Update};
use aqua_store::runner::Cmd;
use aqua_store::setup::Requirement;
use aqua_ui::{tr, trf, SMenu, SPerm, SSetup};
use slint::{ModelRc, SharedString, VecModel};

fn item(id: &str, label: &str, glyph: &str) -> SMenu {
    SMenu { id: ss(id), label: ss(label), glyph: ss(glyph), destructive: false, separator: false, enabled: true }
}

fn sep() -> SMenu {
    SMenu { separator: true, ..Default::default() }
}

pub fn open_url(url: &str) {
    if url.is_empty() {
        return;
    }
    let q = aqua_apps::shell_quote(url);
    aqua_apps::launch(&format!("xdg-open {q}"));
}

pub fn show_folder(p: &std::path::Path) {
    let q = aqua_apps::shell_quote(&p.to_string_lossy());
    if aqua_apps::find_in_path("aqua-finder").is_some() {
        aqua_apps::launch(&format!("aqua-finder {q}"));
    } else {
        aqua_apps::launch(&format!("xdg-open {q}"));
    }
}

impl App {
    pub fn pkg(&self, key: &str) -> Option<Package> {
        self.pkgs.get(key).cloned().or_else(|| aqua_store::model::parse_key(key).map(|(o, n)| Package::new(o, n)))
    }

    pub fn primary(&mut self, key: &str) {
        let Some(p) = self.pkg(key) else { return };
        if self.job_for(key).is_some() {
            return;
        }
        if !p.installed {
            if p.origin() == Origin::Aur {
                self.ask(
                    Pending::Install(key.to_string()),
                    &trf("Install “{n}” from the AUR?", &[("n", &p.display_name())]),
                    tr("AUR packages are maintained by the community and built on this computer. Only install packages you trust."),
                    tr("Install"),
                    None,
                );
                return;
            }
            self.install(key);
        } else if p.has_update() {
            self.update_keys(vec![key.to_string()]);
        } else if Self::openable(&p) {
            bg(
                &self.store,
                move |s| s.launch(&p),
                |a, ok| {
                    if !ok {
                        a.toast(tr("The app couldn’t be opened."));
                    }
                },
            );
        }
    }

    pub fn cancel(&mut self, key: &str) {
        self.jobs.cancel_key(key);
        let groups: Vec<String> =
            self.groups.iter().filter(|(_, m)| m.iter().any(|k| k == key)).map(|(g, _)| g.clone()).collect();
        for g in groups {
            self.jobs.cancel_key(&g);
        }
    }

    fn require(&mut self, id: String, then: Pending) {
        self.pending = Some(then);
        bg(
            &self.store,
            |s| s.requirements(),
            move |a, reqs| {
                let found = reqs.iter().find(|r| r.id == id).cloned();
                let admin = reqs.iter().find(|r| r.id == "polkit").cloned();
                let found = match (found, admin) {
                    (Some(f), Some(p)) if f.steps.iter().any(|c| c.root) => Some(p),
                    (f, _) => f,
                };
                let r = found.or_else(|| {
                    (id == "polkit").then(|| Requirement {
                        id: "polkit",
                        title: "Administrator Access".into(),
                        detail: "App Store needs a way to ask for your password (polkit). Install it with your system tools.".into(),
                        action: String::new(),
                        steps: vec![],
                        terminal: false,
                        essential: true,
                    })
                });
                match r {
                    Some(r) => {
                        let need = SSetup {
                            id: ss(r.id),
                            title: ss(conv::requirement_title(r.id)),
                            detail: ss(tr(&r.detail)),
                            action: ss(if r.steps.is_empty() { tr("OK") } else { tr(&r.action) }),
                            state: 0,
                            essential: r.essential,
                            terminal: r.terminal,
                        };
                        a.need = Some(r);
                        a.g(|g| {
                            g.set_need(need);
                            g.set_sheet(ss("need"));
                        });
                    }
                    None => {
                        a.pending = None;
                        a.error(
                            tr("This app can’t be installed"),
                            tr("The source it comes from isn’t available on this computer."),
                            "",
                        );
                    }
                }
            },
        );
    }

    pub fn install(&mut self, key: &str) {
        let Some(p) = self.pkg(key) else { return };
        match self.store.install_steps(&p) {
            Ok(steps) => {
                let title = p.display_name().to_string();
                self.jobs.submit(key, &title, Kind::Install, steps);
            }
            Err(req) => self.require(req, Pending::Install(key.to_string())),
        }
    }

    pub fn reinstall(&mut self, key: &str) {
        let Some(p) = self.pkg(key) else { return };
        let mut steps = match self.store.remove_steps(&p, false) {
            Ok(s) => s,
            Err(req) => return self.require(req, Pending::Reinstall(key.to_string())),
        };
        let mut np = p.clone();
        np.installed = false;
        match self.store.install_steps(&np) {
            Ok(s) => steps.extend(s),
            Err(req) => return self.require(req, Pending::Reinstall(key.to_string())),
        }
        self.jobs.submit(key, p.display_name(), Kind::Install, steps);
    }

    pub fn remove(&mut self, key: &str, purge: bool) {
        let Some(p) = self.pkg(key) else { return };
        match self.store.remove_steps(&p, purge) {
            Ok(steps) => {
                self.job_state.insert(
                    key.to_string(),
                    JobState { removing: true, started: false, fraction: -1.0, status: tr("Removing…").into() },
                );
                self.refresh_key(key);
                self.jobs.submit(key, p.display_name(), Kind::Remove, steps);
            }
            Err(req) => self.require(req, Pending::Remove(key.to_string(), purge)),
        }
    }

    pub fn ask_remove(&mut self, key: &str) {
        let Some(p) = self.pkg(key) else { return };
        let name = p.display_name().to_string();
        let opt = match p.origin() {
            Origin::Flatpak => Some(tr("Also delete the app’s data and settings").to_string()),
            Origin::Apt | Origin::Pacman | Origin::Aur => Some(tr("Also remove its configuration files").to_string()),
            Origin::Dnf => None,
        };
        let text = if p.origin().is_native() {
            trf("“{n}” and the packages only it needs will be removed from this computer.", &[("n", &name)])
        } else {
            trf("“{n}” will be removed from this computer.", &[("n", &name)])
        };
        self.ask(
            Pending::Remove(key.to_string(), false),
            &trf("Uninstall “{n}”?", &[("n", &name)]),
            &text,
            tr("Uninstall"),
            opt,
        );
    }

    pub fn ask(&mut self, action: Pending, title: &str, text: &str, button: &str, option: Option<String>) {
        self.confirm = Some(action);
        let has = option.is_some();
        self.g(|g| {
            g.set_confirm_title(ss(title));
            g.set_confirm_text(ss(text));
            g.set_confirm_button(ss(button));
            g.set_confirm_has_option(has);
            g.set_confirm_option(ss(option.unwrap_or_default()));
            g.set_confirm_checked(false);
            g.set_sheet(ss("confirm"));
        });
    }

    pub fn confirm_accept(&mut self) {
        let checked = self.g(|g| g.get_confirm_checked()).unwrap_or(false);
        self.g(|g| g.set_sheet(SharedString::new()));
        match self.confirm.take() {
            Some(Pending::Install(k)) => self.install(&k),
            Some(Pending::Remove(k, _)) => self.remove(&k, checked),
            Some(Pending::Reinstall(k)) => self.reinstall(&k),
            Some(Pending::Update(keys)) => self.run_update(keys),
            None => {}
        }
    }

    pub fn update_keys(&mut self, keys: Vec<String>) {
        let ups: Vec<&Update> = self.updates.iter().filter(|u| keys.contains(&u.key())).collect();
        let whole =
            ups.iter().any(|u| self.store.manager(u.origin()).map(|m| m.upgrade_is_whole_system()).unwrap_or(false));
        if whole && keys.len() == 1 {
            let n = self.updates.iter().filter(|u| u.origin().is_native() && u.origin() != Origin::Aur).count();
            self.ask(
                Pending::Update(keys),
                tr("Update System Packages?"),
                &aqua_ui::ntr(
                    "Your system updates all its packages together, so {n} package will be updated.",
                    "Your system updates all its packages together, so {n} packages will be updated.",
                    n as i64,
                ),
                tr("Update"),
                None,
            );
            return;
        }
        self.run_update(keys);
    }

    pub fn run_update(&mut self, keys: Vec<String>) {
        let ups: Vec<Update> = self.updates.iter().filter(|u| keys.contains(&u.key())).cloned().collect();
        if ups.is_empty() {
            return;
        }
        match self.store.update_steps(&ups) {
            Ok(steps) => {
                let mut members: Vec<String> = ups.iter().map(|u| u.key()).collect();
                if ups
                    .iter()
                    .any(|u| self.store.manager(u.origin()).map(|m| m.upgrade_is_whole_system()).unwrap_or(false))
                {
                    for u in &self.updates {
                        if u.origin() == ups[0].origin() && !members.contains(&u.key()) {
                            members.push(u.key());
                        }
                    }
                }
                let gk = if members.len() == 1 {
                    members[0].clone()
                } else {
                    format!("group:{}", aqua_store::http::hash(&members.join(",")))
                };
                if members.len() > 1 {
                    self.groups.insert(gk.clone(), members.clone());
                }
                let title = if members.len() == 1 {
                    self.pkgs.get(&members[0]).map(|p| p.display_name().to_string()).unwrap_or_default()
                } else {
                    tr("Updates").to_string()
                };
                self.jobs.submit(&gk, &title, Kind::Update, steps);
                if members.len() > 1 {
                    self.g(|g| g.set_updating_all(true));
                }
            }
            Err(req) => self.require(req, Pending::Update(keys)),
        }
    }

    pub fn update_all(&mut self) {
        let hidden = self.store.prefs().hidden;
        let keys: Vec<String> = self.updates.iter().map(|u| u.key()).filter(|k| !hidden.contains(k)).collect();
        self.run_update(keys);
    }

    pub fn on_job(&mut self, ev: Event) {
        let key = ev.key().to_string();
        if let Some(id) = key.strip_prefix("setup:") {
            return self.on_setup_job(id.to_string(), ev);
        }
        if key.starts_with("task:") {
            if let Event::Finished { ok, cancelled, error, log, .. } = ev {
                self.g(|g| g.set_setup_busy(false));
                if ok {
                    self.toast(tr("Done."));
                    self.after_change();
                } else if !cancelled {
                    self.error(tr("The operation failed"), &error, &log);
                }
            }
            return;
        }
        match ev {
            Event::Queued { .. } => {
                let removing = self.job_state.get(&key).map(|j| j.removing).unwrap_or(false);
                self.job_state
                    .insert(key.clone(), JobState { removing, started: false, fraction: -1.0, status: String::new() });
            }
            Event::Started { .. } => {
                let removing = self.job_state.get(&key).map(|j| j.removing).unwrap_or(false);
                self.job_state.insert(
                    key.clone(),
                    JobState { removing, started: true, fraction: -1.0, status: tr("Preparing…").into() },
                );
            }
            Event::Progress { fraction, status, .. } => {
                if let Some(j) = self.job_state.get_mut(&key) {
                    j.started = true;
                    j.fraction = fraction;
                    j.status = conv::tr_status(&status);
                }
            }
            Event::Finished { kind, ok, cancelled, error, log, .. } => {
                self.job_state.remove(&key);
                let members = self.groups.remove(&key).unwrap_or_else(|| vec![key.clone()]);
                if members.len() > 1 || key.starts_with("group:") {
                    self.g(|g| g.set_updating_all(false));
                }
                for k in &members {
                    if ok {
                        self.record(k, &kind);
                        if let Some(p) = self.pkgs.get_mut(k) {
                            match kind {
                                Kind::Remove => {
                                    p.installed = false;
                                    p.installed_version.clear();
                                    p.update_version.clear();
                                }
                                Kind::Install | Kind::Update => {
                                    p.installed = true;
                                    if !p.update_version.is_empty() {
                                        p.installed_version = std::mem::take(&mut p.update_version);
                                    } else if p.installed_version.is_empty() {
                                        p.installed_version = p.version.clone();
                                    }
                                }
                                _ => {}
                            }
                        }
                        if kind == Kind::Update || kind == Kind::Remove {
                            self.updates.retain(|u| &u.key() != k);
                        }
                    }
                }
                if ok {
                    let name = self.pkgs.get(&members[0]).map(|p| p.display_name().to_string()).unwrap_or_default();
                    let msg = match kind {
                        Kind::Install => trf("“{n}” is installed.", &[("n", &name)]),
                        Kind::Remove => trf("“{n}” was removed.", &[("n", &name)]),
                        Kind::Update if members.len() > 1 => tr("Updates installed.").to_string(),
                        Kind::Update => trf("“{n}” is up to date.", &[("n", &name)]),
                        _ => tr("Done.").to_string(),
                    };
                    self.toast(&msg);
                    let ups = self.updates.clone();
                    self.apply_updates(ups);
                    self.after_change();
                } else if !cancelled {
                    let name = self.pkgs.get(&members[0]).map(|p| p.display_name().to_string()).unwrap_or_default();
                    let title = match kind {
                        Kind::Install => trf("“{n}” couldn’t be installed", &[("n", &name)]),
                        Kind::Remove => trf("“{n}” couldn’t be removed", &[("n", &name)]),
                        _ => tr("Updates couldn’t be installed").to_string(),
                    };
                    self.error(&title, tr(&error), &log);
                }
                for k in &members {
                    self.refresh_key(k);
                }
            }
        }
        if self.job_state.contains_key(&key) {
            let members = self.groups.get(&key).cloned().unwrap_or_else(|| vec![key.clone()]);
            for k in members {
                self.refresh_key(&k);
            }
        }
    }

    fn record(&mut self, key: &str, kind: &Kind) {
        let Some(p) = self.pkgs.get(key) else { return };
        let version = match kind {
            Kind::Update if !p.update_version.is_empty() => p.update_version.clone(),
            _ if !p.version.is_empty() => p.version.clone(),
            _ => p.installed_version.clone(),
        };
        history::record(history::Entry {
            time: aqua_store::units::now(),
            action: kind.verb().to_string(),
            key: key.to_string(),
            name: p.display_name().to_string(),
            version,
            source: conv::source_label(p),
        });
    }

    pub fn error(&mut self, title: &str, text: &str, log: &str) {
        let tail: String = {
            let lines: Vec<&str> = log.lines().collect();
            let start = lines.len().saturating_sub(400);
            lines[start..].join("\n")
        };
        self.g(|g| {
            g.set_error_title(ss(title));
            g.set_error_text(ss(text));
            g.set_error_log(ss(tail));
            g.set_sheet(ss("error"));
        });
    }

    pub fn app_menu(&mut self, key: &str, x: f32, y: f32, page: bool) {
        let Some(p) = self.pkg(key) else { return };
        self.menu_key = key.to_string();
        let mut m = vec![];
        let busy = self.job_for(key).is_some();
        if busy {
            m.push(item("cancel", tr("Cancel"), "xmark"));
            m.push(sep());
        } else if p.installed {
            if Self::openable(&p) {
                m.push(item("open", tr("Open"), "play"));
            }
            if p.has_update() {
                m.push(item("update", tr("Update"), "updates"));
            }
        } else {
            m.push(item("get", tr("Get"), "cloud"));
        }
        if !page {
            m.push(item("info", tr("Show Details"), "info"));
        }
        if p.installed && !busy {
            m.push(sep());
            if p.origin() == Origin::Flatpak {
                m.push(item("perms", tr("Permissions…"), "lockshield"));
                m.push(item("data", tr("Show Data Folder"), "folder"));
            }
            m.push(item("reinstall", tr("Reinstall"), "refresh"));
            if p.has_update() {
                let hidden = self.store.prefs().hidden.contains(&key.to_string());
                m.push(item("hide", if hidden { tr("Show This Update") } else { tr("Ignore This Update") }, "bell"));
            }
        }
        m.push(sep());
        m.push(item("copy", tr("Copy Link"), "link"));
        m.push(item("web", tr("Open in Browser"), "globe"));
        if page {
            if let Some(u) = self.detail.as_ref().and_then(|d| d.link("bugtracker").map(str::to_string)) {
                m.push(item(&format!("url:{u}"), tr("Report a Problem"), "flag"));
            }
        }
        if p.installed && !busy {
            m.push(sep());
            let mut u = item("remove", tr("Uninstall…"), "trash");
            u.destructive = true;
            m.push(u);
        }
        self.g(|g| {
            g.set_menu(ModelRc::new(VecModel::from(m)));
            g.set_menu_x(x);
            g.set_menu_y(y);
            g.set_menu_open(true);
        });
    }

    pub fn share_menu(&mut self, x: f32, y: f32) {
        let Route::App(key) = self.route.clone() else { return };
        self.menu_key = key;
        let m = vec![
            item("copy", tr("Copy Link"), "link"),
            item("copy-id", tr("Copy App ID"), "info"),
            item("web", tr("Open in Browser"), "globe"),
        ];
        self.g(|g| {
            g.set_menu(ModelRc::new(VecModel::from(m)));
            g.set_menu_x(x);
            g.set_menu_y(y);
            g.set_menu_open(true);
        });
    }

    pub fn copy(&mut self, text: &str) {
        if let Some(u) = self.ui() {
            u.invoke_copy_text(ss(text));
            self.toast(tr("Copied"));
        }
    }

    pub fn menu_action(&mut self, id: &str) {
        let key = self.menu_key.clone();
        if let Some(c) = id.strip_prefix("category:") {
            return self.open(Route::Category(c.to_string()));
        }
        if let Some(d) = id.strip_prefix("developer:") {
            return self.open(Route::Developer(d.to_string()));
        }
        if let Some(u) = id.strip_prefix("url:") {
            return open_url(u);
        }
        if id == "reviews" {
            if !self.reviews.is_empty() {
                self.g(|g| g.set_sheet(ss("reviews")));
            }
            return;
        }
        let Some(p) = self.pkg(&key) else { return };
        match id {
            "cancel" => self.cancel(&key),
            "open" | "update" | "get" => self.primary(&key),
            "info" => self.open(Route::App(key)),
            "perms" => self.open_perms(&key),
            "data" => {
                let d = aqua_store::flatpak::data_dir(&p.name);
                if d.exists() {
                    show_folder(&d);
                } else {
                    self.toast(tr("This app hasn’t stored any data yet."));
                }
            }
            "reinstall" => self.ask(
                Pending::Reinstall(key.clone()),
                &trf("Reinstall “{n}”?", &[("n", &p.display_name())]),
                tr("The app will be removed and installed again. Your data is kept."),
                tr("Reinstall"),
                None,
            ),
            "hide" => {
                let mut pr = self.store.prefs();
                if let Some(i) = pr.hidden.iter().position(|h| h == &key) {
                    pr.hidden.remove(i);
                } else {
                    pr.hidden.push(key.clone());
                }
                self.store.set_prefs(pr);
                let ups = self.updates.clone();
                self.apply_updates(ups);
            }
            "copy" => self.copy(&conv::web_url(&p)),
            "copy-id" => self.copy(if p.appstream_id.is_empty() { &p.name } else { &p.appstream_id }),
            "web" => open_url(&conv::web_url(&p)),
            "remove" => self.ask_remove(&key),
            _ => {}
        }
    }

    pub fn open_perms(&mut self, key: &str) {
        let Some(p) = self.pkg(key) else { return };
        if !matches!(self.route, Route::App(ref k) if k == key) {
            let app = self.sapp(&p);
            self.g(|g| {
                let mut d = g.get_detail();
                d.app = app;
                g.set_detail(d);
            });
        }
        self.menu_key = key.to_string();
        bg(&self.store, move |s| s.permissions(&p), |a, v| a.apply_perm_toggles(v));
    }

    fn apply_perm_toggles(&mut self, v: Vec<(String, String, bool)>) {
        let rows: Vec<SPerm> = v
            .iter()
            .map(|(id, label, on)| SPerm {
                id: ss(id),
                label: ss(tr(label)),
                detail: ss(perm_detail(id)),
                glyph: ss(conv::perm_glyph(id)),
                risky: id == "host" || id == "devices",
                on: *on,
            })
            .collect();
        self.g(|g| {
            g.set_perms(ModelRc::new(VecModel::from(rows)));
            g.set_sheet(ss("perms"));
        });
    }

    pub fn perm_toggled(&mut self, id: &str, on: bool) {
        let Some(p) = self.pkg(&self.menu_key.clone()) else { return };
        let cmd = aqua_store::flatpak::override_cmd(&p.name, id, on);
        self.run_quick(vec![cmd], p);
    }

    pub fn perm_reset(&mut self) {
        let Some(p) = self.pkg(&self.menu_key.clone()) else { return };
        self.run_quick(vec![aqua_store::flatpak::Flatpak::reset_overrides(&p.name)], p);
    }

    fn run_quick(&mut self, cmds: Vec<Cmd>, p: Package) {
        bg(
            &self.store,
            move |s| {
                let mut err = String::new();
                for c in &cmds {
                    let o = s.run.run(c);
                    if !o.ok() {
                        err = o.stderr.clone();
                    }
                }
                (err, s.permissions(&p))
            },
            |a, (err, v)| {
                if !err.is_empty() {
                    a.toast(err.lines().last().unwrap_or(""));
                }
                a.apply_perm_toggles(v);
            },
        );
    }

    pub fn open_setup(&mut self) {
        self.g(|g| g.set_sheet(ss("setup")));
        self.load_setup(false);
    }

    pub fn load_setup(&mut self, auto: bool) {
        bg(
            &self.store,
            |s| {
                s.reload_env();
                s.requirements()
            },
            move |a, reqs| {
                let dismissed = a.store.prefs().dismissed;
                if auto {
                    let show = reqs.iter().any(|r| r.essential && !dismissed.iter().any(|d| d == r.id));
                    if !show || a.g(|g| g.get_sheet() != "").unwrap_or(true) {
                        a.reqs = reqs;
                        return;
                    }
                    a.g(|g| g.set_sheet(ss("setup")));
                }
                for r in &reqs {
                    a.req_state.entry(r.id.to_string()).or_insert(0);
                }
                a.reqs = reqs;
                a.apply_setup();
                a.apply_env();
            },
        );
    }

    pub fn apply_setup(&mut self) {
        let rows: Vec<SSetup> = self
            .reqs
            .iter()
            .map(|r| SSetup {
                id: ss(r.id),
                title: ss(tr(&r.title)),
                detail: ss(tr(&r.detail)),
                action: ss(tr(&r.action)),
                state: *self.req_state.get(r.id).unwrap_or(&0),
                essential: r.essential,
                terminal: r.terminal,
            })
            .collect();
        let busy = self.req_state.values().any(|s| *s == 1);
        self.g(|g| {
            g.set_setup(ModelRc::new(VecModel::from(rows)));
            g.set_setup_busy(busy);
        });
    }

    pub fn setup_install(&mut self, id: &str) {
        let Some(r) =
            self.reqs.iter().find(|r| r.id == id).cloned().or_else(|| self.need.clone().filter(|n| n.id == id))
        else {
            return;
        };
        if r.steps.is_empty() {
            return;
        }
        self.req_state.insert(id.to_string(), 1);
        if r.terminal {
            let line = r.steps.iter().map(|c| c.line()).collect::<Vec<_>>().join(" && ");
            let script = format!("{line}; echo; read -p \"{}\" _", tr("Press Enter to close this window"));
            let cmd = format!("sh -c {}", aqua_apps::shell_quote(&script));
            aqua_apps::launch(&aqua_apps::terminal_wrap(&cmd));
            self.watch_terminal(id.to_string(), 0);
        } else {
            self.jobs.submit(&format!("setup:{id}"), &r.title, Kind::Setup, r.steps.clone());
        }
        self.apply_setup();
    }

    fn watch_terminal(&mut self, id: String, n: u32) {
        let store = self.store.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(3), move || {
            bg(
                &store,
                |s| {
                    s.reload_env();
                    s.requirements()
                },
                move |a, reqs| {
                    let done = !reqs.iter().any(|r| r.id == id);
                    if done || n > 100 {
                        a.req_state.insert(id.clone(), if done { 2 } else { 0 });
                        a.reqs = reqs;
                        a.apply_setup();
                        a.apply_env();
                        if done {
                            a.resume();
                        }
                    } else {
                        a.watch_terminal(id, n + 1);
                    }
                },
            );
        });
    }

    pub fn setup_all(&mut self) {
        let ids: Vec<&'static str> = self
            .reqs
            .iter()
            .filter(|r| !r.steps.is_empty() && self.req_state.get(r.id) != Some(&2))
            .map(|r| r.id)
            .collect();
        for id in ids {
            self.setup_install(id);
        }
    }

    pub fn setup_dismiss(&mut self) {
        let mut p = self.store.prefs();
        for r in &self.reqs {
            if !p.dismissed.iter().any(|d| d == r.id) {
                p.dismissed.push(r.id.to_string());
            }
        }
        self.store.set_prefs(p);
        self.g(|g| g.set_sheet(SharedString::new()));
    }

    fn on_setup_job(&mut self, id: String, ev: Event) {
        if let Event::Finished { ok, cancelled, error, log, .. } = ev {
            self.req_state.insert(
                id.clone(),
                if ok {
                    2
                } else if cancelled {
                    0
                } else {
                    3
                },
            );
            if !ok && !cancelled {
                let is_need = self.g(|g| g.get_sheet() == "need").unwrap_or(false);
                if is_need || self.g(|g| g.get_sheet() == "").unwrap_or(false) {
                    self.pending = None;
                    self.error(tr("Setup didn’t finish"), tr(&error), &log);
                }
            }
            self.apply_setup();
            bg(
                &self.store,
                |s| {
                    s.reload_env();
                    s.invalidate();
                    s.load_catalog();
                    s.requirements()
                },
                move |a, reqs| {
                    a.reqs = reqs;
                    a.apply_setup();
                    a.apply_env();
                    a.home_cache.clear();
                    if ok {
                        a.resume();
                    }
                },
            );
        }
    }

    fn resume(&mut self) {
        let sheet = self.g(|g| g.get_sheet().to_string()).unwrap_or_default();
        if sheet == "need" {
            self.g(|g| g.set_sheet(SharedString::new()));
        }
        self.need = None;
        match self.pending.take() {
            Some(Pending::Install(k)) => self.install(&k),
            Some(Pending::Remove(k, purge)) => self.remove(&k, purge),
            Some(Pending::Reinstall(k)) => self.reinstall(&k),
            Some(Pending::Update(keys)) => self.run_update(keys),
            _ => {}
        }
    }

    pub fn need_accept(&mut self) {
        let Some(r) = self.need.clone() else {
            self.g(|g| g.set_sheet(SharedString::new()));
            return;
        };
        if r.steps.is_empty() {
            self.pending = None;
            self.g(|g| g.set_sheet(SharedString::new()));
            return;
        }
        if !self.reqs.iter().any(|x| x.id == r.id) {
            self.reqs.push(r.clone());
        }
        self.setup_install(r.id);
        let mut n = self.g(|g| g.get_need()).unwrap_or_default();
        n.state = 1;
        self.g(|g| g.set_need(n));
    }

    pub fn apply_env(&mut self) {
        let env = self.store.env();
        let prefs = self.store.prefs();
        conv::set_native_label(&env.native_label());
        let mut items = vec![ss("Flathub")];
        if env.native.is_some() {
            items.push(ss(env.native_label()));
        }
        let info = {
            let mut parts = vec![];
            if !env.distro_name.is_empty() {
                parts.push(env.distro_name.clone());
            }
            if let Some(o) = env.native {
                parts.push(aqua_store::system::native_program(o).to_string());
            }
            if env.flatpak {
                parts.push("Flatpak".into());
            }
            if let Some(h) = &env.aur_helper {
                parts.push(h.clone());
            }
            parts.join(" · ")
        };
        self.g(|g| {
            g.set_set_auto_check(prefs.auto_check);
            g.set_set_auto_update(prefs.auto_update);
            g.set_set_notify(prefs.notify);
            g.set_set_system(prefs.include_system);
            g.set_set_flathub(prefs.use_flathub);
            g.set_set_native(prefs.use_native);
            g.set_set_aur(prefs.use_aur);
            g.set_set_reviews(prefs.show_reviews);
            g.set_set_scope(if prefs.flatpak_scope == aqua_store::model::Scope::System { 1 } else { 0 });
            g.set_set_preferred(if prefs.preferred == "native" && env.native.is_some() { 1 } else { 0 });
            g.set_reviewer(ss(&prefs.reviewer_name));
            g.set_preferred_items(ModelRc::new(VecModel::from(items)));
            g.set_native_label(ss(env.native_label()));
            g.set_has_native(env.native.is_some());
            g.set_has_aur(env.has_aur());
            g.set_has_flatpak(env.flatpak);
            g.set_system_info(ss(info));
        });
    }

    pub fn setting(&mut self, id: &str, v: bool) {
        let mut p = self.store.prefs();
        let sources = matches!(id, "use_flathub" | "use_native" | "use_aur" | "include_system");
        match id {
            "auto_check" => p.auto_check = v,
            "auto_update" => p.auto_update = v,
            "notify" => p.notify = v,
            "include_system" => p.include_system = v,
            "use_flathub" => p.use_flathub = v,
            "use_native" => p.use_native = v,
            "use_aur" => p.use_aur = v,
            "show_reviews" => p.show_reviews = v,
            _ => {}
        }
        self.store.set_prefs(p);
        if id == "auto_check" || id == "auto_update" {
            crate::notify::sync_autostart(v || self.store.prefs().auto_check);
        }
        if sources {
            self.home_cache.clear();
            self.store.invalidate();
        }
    }

    pub fn scope_changed(&mut self, i: i32) {
        let mut p = self.store.prefs();
        p.flatpak_scope = if i == 1 { aqua_store::model::Scope::System } else { aqua_store::model::Scope::User };
        self.store.set_prefs(p);
    }

    pub fn preferred_changed(&mut self, i: i32) {
        let mut p = self.store.prefs();
        p.preferred = if i == 1 { "native".into() } else { "flatpak".into() };
        self.store.set_prefs(p);
        self.home_cache.clear();
    }

    pub fn reviewer(&mut self, name: &str) {
        let mut p = self.store.prefs();
        p.reviewer_name = name.to_string();
        self.store.set_prefs(p);
    }

    pub fn maintenance(&mut self, what: &str) {
        let scope = self.store.prefs().flatpak_scope;
        let steps = match what {
            "unused" => aqua_store::flatpak::Flatpak::remove_unused(),
            "repair" => vec![aqua_store::flatpak::Flatpak::repair(scope)],
            _ => self.store.refresh_steps(),
        };
        if what == "refresh" {
            self.home_cache.clear();
            self.store.http.expire_json();
        }
        if steps.is_empty() {
            self.toast(tr("Done."));
            return;
        }
        self.g(|g| g.set_setup_busy(true));
        self.toast(tr("Working…"));
        self.jobs.submit(&format!("task:{what}"), what, Kind::Other, steps);
    }

    pub fn write_review(&mut self) {
        let name = self.store.prefs().reviewer_name;
        let name = if name.is_empty() { aqua_ui::user_names().1 } else { name };
        self.g(|g| {
            g.set_review_stars(0);
            g.set_review_title(SharedString::new());
            g.set_review_body(SharedString::new());
            g.set_review_error(SharedString::new());
            g.set_reviewer(ss(name));
            g.set_sheet(ss("review"));
        });
    }

    pub fn submit_review(&mut self) {
        let Some(d) = self.detail.clone() else { return };
        let (stars, title, body, author) = self
            .g(|g| {
                (
                    g.get_review_stars(),
                    g.get_review_title().to_string(),
                    g.get_review_body().to_string(),
                    g.get_reviewer().to_string(),
                )
            })
            .unwrap_or_default();
        let id = if d.pkg.appstream_id.is_empty() { d.pkg.name.clone() } else { d.pkg.appstream_id.clone() };
        let ver = self.pkgs.get(&d.pkg.key()).map(|p| p.installed_version.clone()).unwrap_or_default();
        let skey = self.skey.clone();
        self.reviewer(&author);
        self.g(|g| g.set_review_sending(true));
        let key = d.pkg.key();
        bg(
            &self.store,
            move |s| s.reviews.submit(&id, &skey, &ver, &author, &title, &body, stars.clamp(1, 5) as u8),
            move |a, r| {
                a.g(|g| g.set_review_sending(false));
                match r {
                    Ok(()) => {
                        a.g(|g| g.set_sheet(SharedString::new()));
                        a.toast(tr("Thanks! Your review was sent."));
                        a.refresh_detail_extras(&key);
                    }
                    Err(e) => {
                        a.g(|g| g.set_review_error(ss(trf("Your review couldn’t be sent: {e}", &[("e", &e)]))));
                    }
                }
            },
        );
    }

    pub fn vote(&mut self, id: i32, kind: &str) {
        let Some(d) = self.detail.clone() else { return };
        let app = if d.pkg.appstream_id.is_empty() { d.pkg.name.clone() } else { d.pkg.appstream_id.clone() };
        let skey = self.skey.clone();
        let kind = kind.to_string();
        self.voted.insert(id as i64);
        self.apply_reviews();
        let report = kind == "report";
        bg(
            &self.store,
            move |s| s.reviews.vote(&app, &skey, id as i64, &kind),
            move |a, r| {
                if r.is_ok() && report {
                    a.toast(tr("Thanks. The review was reported."));
                }
            },
        );
    }
}

pub fn perm_detail(id: &str) -> String {
    match id {
        "network" => tr("Can access the internet and local network"),
        "home" => tr("Can read and change files in your home folder"),
        "host" => tr("Can read and change all files on this computer"),
        "devices" => tr("Can use cameras, game controllers and other devices"),
        "sound" => tr("Can play and record sound"),
        "x11" => tr("Uses the legacy display server, which is less isolated"),
        _ => "",
    }
    .to_string()
}
