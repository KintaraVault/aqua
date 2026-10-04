//! Search, Go to Folder suggestions and background polling (watching, thumbnails, transfers).
use super::*;

const SKIP_ROOTS: [&str; 7] = ["/proc", "/sys", "/dev", "/run", "/tmp", "/var/lib", "/snap"];

/// Whether `p` is a text file containing `q` (lowercase).
pub(super) fn contains_text(p: &Path, q: &str) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() || m.len() > 4 * 1024 * 1024 {
        return false;
    }
    let Ok(bytes) = std::fs::read(p) else { return false };
    if bytes.iter().take(4096).any(|&b| b == 0) {
        return false;
    }
    String::from_utf8_lossy(&bytes).to_lowercase().contains(q)
}

/// Folders matching a partly typed path (for Go to Folder).
pub(super) fn complete_path(text: &str, hidden: bool) -> Vec<String> {
    let t = text.trim();
    if t.is_empty() {
        return vec![];
    }
    let home = fs::home();
    let expanded = if t == "~" {
        format!("{}/", home.display())
    } else if let Some(r) = t.strip_prefix("~/") {
        format!("{}/{r}", home.display())
    } else if t.starts_with('/') {
        t.to_string()
    } else {
        format!("{}/{t}", home.display())
    };
    let (dir, prefix) = match expanded.rfind('/') {
        Some(i) => (expanded[..i.max(1)].to_string(), expanded[i + 1..].to_lowercase()),
        None => return vec![],
    };
    let mut v: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    n.to_lowercase().starts_with(&prefix) && (hidden || !n.starts_with('.') || prefix.starts_with('.'))
                })
                .map(|n| Path::new(&dir).join(n).to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| fs::natural(a, b));
    v.truncate(8);
    v
}

/// Add a scheme when only a host was typed.
pub(super) fn normalize_server(addr: &str) -> String {
    let a = addr.trim();
    if a.is_empty() || a.contains("://") || a.starts_with('/') || a.starts_with('~') {
        return a.to_string();
    }
    format!("smb://{a}")
}

/// Where GVfs shows a mounted location.
fn mounted_path(addr: &str) -> Option<PathBuf> {
    let o = std::process::Command::new("gio").args(["info", addr]).output().ok()?;
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("local path:").map(|p| PathBuf::from(p.trim())))
        .filter(|p| p.exists())
}

/// Kinds whose name starts with the typed text (the “Kinds” suggestions).
fn kind_suggestions(q: &str) -> Vec<FOpt> {
    let ql = q.to_lowercase();
    if ql.chars().count() < 2 {
        return vec![];
    }
    criteria::KINDS
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, k)| {
            let t = crate::tr(k).to_lowercase();
            t.starts_with(&ql) || k.to_lowercase().starts_with(&ql)
        })
        .take(3)
        .map(|(i, k)| FOpt { id: (100 + i as i32).to_string().into(), label: crate::tr(k).into(), on: false })
        .collect()
}

impl App {
    pub(super) fn start_search(&mut self, root: PathBuf, q: String) {
        self.search_gen += 1;
        let gen = self.search_gen;
        let (tx, rx) = std::sync::mpsc::channel();
        self.search_rx = Some(rx);
        self.ui().global::<F>().set_search_busy(true);
        let hidden = self.st.hidden;
        let content = self.search_content;
        let crit = self.criteria.clone();
        std::thread::spawn(move || {
            let now = fs::now_secs();
            let ql = q.to_lowercase();
            let mut batch = vec![];
            let mut found = 0usize;
            let mut seen = 0usize;
            let start = std::time::Instant::now();
            let mut last_send = std::time::Instant::now();
            let home = fs::home();
            let roots: Vec<PathBuf> =
                if root == Path::new("/") { vec![home.clone(), root.clone()] } else { vec![root.clone()] };
            let mut seen_paths = HashSet::new();
            for r in roots {
                let everything = r == Path::new("/");
                let ok = fs::walk(&r, 12, &mut |e: Entry| {
                    seen += 1;
                    if everything && (e.path.starts_with(&home) || SKIP_ROOTS.iter().any(|s| e.path.starts_with(s))) {
                        return true;
                    }
                    let name_hit = e.name.to_lowercase().contains(&ql);
                    let hit = (hidden || !e.name.starts_with('.'))
                        && (name_hit || content && !e.is_dir && matches!(e.kind, 6 | 1) && contains_text(&e.path, &ql))
                        && criteria::all_match(&crit, &e, now, &contains_text);
                    if hit && seen_paths.insert(e.path.clone()) {
                        batch.push(e);
                        found += 1;
                    }
                    if last_send.elapsed().as_millis() > 250 && !batch.is_empty() {
                        if tx.send((gen, std::mem::take(&mut batch), false)).is_err() {
                            return false;
                        }
                        last_send = std::time::Instant::now();
                    }
                    found < 2000 && seen < 600_000 && start.elapsed().as_secs() < 30
                });
                if !ok {
                    break;
                }
            }
            let _ = tx.send((gen, batch, true));
        });
    }

    pub(super) fn search(&mut self, q: &str) {
        let q = q.trim().to_string();
        self.ui().global::<F>().set_sugg_kinds(model(kind_suggestions(&q)));
        if let Loc::Search(_, old) = &self.loc {
            if !q.is_empty() || !self.criteria.is_empty() || self.new_smart {
                if &q != old {
                    self.pending_q = Some((std::time::Instant::now(), q));
                } else {
                    self.pending_q = None;
                }
                return;
            }
        }
        if !q.is_empty() && self.chooser.is_none() && matches!(self.loc, Loc::Dir(_) | Loc::Recents | Loc::Apps | Loc::Tag(_))
        {
            self.pending_q = Some((std::time::Instant::now(), q.clone()));
        } else {
            self.pending_q = None;
        }
        if q.is_empty() && !self.criteria.is_empty() {
            if let Loc::Search(root, old) = &self.loc {
                if !old.is_empty() {
                    self.loc = Loc::Search(root.clone(), q);
                    self.load();
                }
            }
            return;
        }
        if q.is_empty() {
            if let Loc::Search(root, _) = &self.loc {
                let r = if root == Path::new("/") {
                    self.back.last().cloned().unwrap_or(Loc::Dir(fs::home()))
                } else {
                    Loc::Dir(root.clone())
                };
                if self.back.last() == Some(&r) {
                    self.back.pop();
                }
                self.loc = r;
                self.load();
            } else {
                self.sort_and_filter();
                self.refresh();
            }
            return;
        }
        if !matches!(self.loc, Loc::Search(..)) {
            self.sort_and_filter();
            self.refresh();
        }
    }

    /// Run the typed query now (a search suggestion was picked or typing paused).
    pub(super) fn search_now(&mut self, q: &str) {
        self.pending_q = None;
        let q = q.trim().to_string();
        if let Loc::Search(root, _) = self.loc.clone() {
            self.loc = Loc::Search(root, q);
            return self.load();
        }
        self.search_commit(&q);
    }

    /// Typing paused: turn the instant filter into a real search.
    pub(super) fn pending_search(&mut self) {
        let due = self.pending_q.as_ref().is_some_and(|(t, _)| t.elapsed().as_millis() >= 320);
        if !due {
            return;
        }
        if let Some((_, q)) = self.pending_q.take() {
            if self.ui().global::<F>().get_query().trim() == q {
                self.search_now(&q);
            }
        }
    }

    /// File > New Smart Folder: an empty search of this computer, waiting for criteria.
    pub(super) fn new_smart_folder(&mut self) {
        if !matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) {
            self.back.push(self.loc.clone());
            self.fwd.clear();
        }
        self.criteria.clear();
        self.search_content = false;
        self.pending_q = None;
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_query(SharedString::new());
        f.set_searching(true);
        self.new_smart = true;
        self.loc = Loc::Search(PathBuf::from("/"), String::new());
        self.load();
    }

    /// A suggestion under the search field: 0 name contains, 1 contents contain, 100+k kind is.
    pub(super) fn search_pick(&mut self, what: i32) {
        let uif = self.ui();
        let f = uif.global::<F>();
        let q = f.get_query().trim().to_string();
        match what {
            0 | 1 => {
                self.search_content = what == 1;
                self.search_now(&q);
            }
            k if k >= 100 => {
                let kind = k - 100;
                self.criteria.retain(|c| c.attr != 0);
                self.criteria.insert(0, criteria::Criterion { attr: 0, op: kind, ..Default::default() });
                f.set_query(SharedString::new());
                f.set_sugg_kinds(model(vec![]));
                self.search_now("");
            }
            _ => {}
        }
    }

    pub(super) fn scope_root(&self, scope: i32) -> PathBuf {
        let folder = match &self.loc {
            Loc::Dir(p) => Some(p.clone()),
            Loc::Search(p, _) if p != Path::new("/") => Some(p.clone()),
            Loc::Smart(_) => self.smart.as_ref().map(|s| s.1.root.clone()).filter(|p| p != Path::new("/")),
            _ => None,
        };
        let folder = folder.or_else(|| {
            self.back.iter().rev().find_map(|l| match l {
                Loc::Dir(p) => Some(p.clone()),
                _ => None,
            })
        });
        match (scope, folder) {
            (1, Some(f)) => f,
            _ => PathBuf::from("/"),
        }
    }

    pub(super) fn search_commit(&mut self, q: &str) {
        let q = q.trim().to_string();
        if q.is_empty() && self.criteria.is_empty() {
            return;
        }
        let scope = match &self.loc {
            Loc::Search(root, _) => i32::from(root != Path::new("/")),
            Loc::Smart(_) => self.smart.as_ref().map(|s| i32::from(s.1.root != Path::new("/"))).unwrap_or(0),
            _ => match self.st.scope {
                1 => 1,
                2 => self.st.last_scope,
                _ => 0,
            },
        };
        let root = self.scope_root(scope);
        if !matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) {
            self.back.push(self.loc.clone());
            self.fwd.clear();
        }
        self.loc = Loc::Search(root, q);
        self.load();
    }

    /// Search bar scope buttons: 0 this computer, 1 current folder, 10 names, 11 contents.
    pub(super) fn set_scope(&mut self, i: i32) {
        if matches!(self.loc, Loc::Smart(_)) {
            let root = self.scope_root(i);
            if let Some((_, s)) = self.smart.as_mut() {
                match i {
                    0 | 1 => s.root = root,
                    10 | 11 => {
                        s.content = i == 11;
                        self.search_content = s.content;
                    }
                    _ => return,
                }
            }
            return self.load();
        }
        let Loc::Search(_, q) = self.loc.clone() else { return };
        match i {
            0 | 1 => {
                self.st.last_scope = i;
                self.st.save();
                let root = self.scope_root(i);
                self.loc = Loc::Search(root, q);
            }
            10 | 11 => self.search_content = i == 11,
            _ => return,
        }
        self.load();
    }

    pub(super) fn crit_rows(&self) -> Vec<FCrit> {
        self.criteria
            .iter()
            .map(|c| FCrit {
                attr: c.attr,
                op: c.op,
                unit: c.unit,
                value: c.value.clone().into(),
                ops: model(c.ops().iter().map(|s| SharedString::from(crate::tr(s))).collect()),
                units: model(c.units().iter().map(|s| SharedString::from(crate::tr(s))).collect()),
                has_value: c.has_value(),
            })
            .collect()
    }

    /// Criteria rows changed: show them and search again.
    fn crit_changed(&mut self, rebuild: bool) {
        if let Some((_, s)) = self.smart.as_mut() {
            s.criteria = self.criteria.clone();
        }
        if rebuild {
            self.ui().global::<F>().set_crit(model(self.crit_rows()));
        }
        if self.criteria.iter().all(|c| !c.has_value() || !c.value.trim().is_empty() || c.attr == 5) {
            match &self.loc {
                Loc::Search(..) | Loc::Smart(_) => self.load(),
                _ => {}
            }
        }
    }

    /// `+` on a criteria row (or the search bar when `after` < 0).
    pub(super) fn crit_add(&mut self, after: i32) {
        let used: Vec<i32> = self.criteria.iter().map(|c| c.attr).collect();
        let attr = [0, 1, 4, 6, 2, 3, 5].into_iter().find(|a| !used.contains(a)).unwrap_or(0);
        let at = if after < 0 { self.criteria.len() } else { (after as usize + 1).min(self.criteria.len()) };
        self.criteria.insert(at, criteria::Criterion::new(attr));
        self.crit_changed(true);
    }

    pub(super) fn crit_remove(&mut self, i: i32) {
        if (i as usize) < self.criteria.len() {
            self.criteria.remove(i as usize);
            if self.criteria.is_empty() {
                if let Loc::Search(_, q) = &self.loc {
                    if q.is_empty() {
                        self.crit_changed(true);
                        return self.search("");
                    }
                }
            }
            self.crit_changed(true);
        }
    }

    pub(super) fn crit_set(&mut self, i: i32, field: &str, v: i32) {
        let Some(c) = self.criteria.get_mut(i as usize) else { return };
        match field {
            "attr" if c.attr != v => *c = criteria::Criterion::new(v),
            "op" => {
                c.op = v;
                if !c.has_value() {
                    c.value.clear();
                } else if c.value.is_empty() && (1..=3).contains(&c.attr) && v == 0 {
                    c.value = "7".into();
                }
            }
            "unit" => c.unit = v,
            _ => return,
        }
        self.crit_changed(true);
    }

    pub(super) fn crit_value(&mut self, i: i32, v: &str) {
        let Some(c) = self.criteria.get_mut(i as usize) else { return };
        c.value = v.to_string();
        self.crit_changed(false);
    }

    /// The search as it would be saved.
    fn current_search(&self) -> Option<criteria::Saved> {
        match &self.loc {
            Loc::Search(root, q) => Some(criteria::Saved {
                root: root.clone(),
                query: q.clone(),
                content: self.search_content,
                criteria: self.criteria.clone(),
            }),
            Loc::Smart(_) => self.smart.as_ref().map(|s| s.1.clone()),
            _ => None,
        }
    }

    pub(super) fn save_search(&mut self) {
        let uif = self.ui();
        let f = uif.global::<F>();
        let name = match (&self.loc, &self.smart) {
            (Loc::Smart(p), _) => criteria::saved_name(p),
            (Loc::Search(_, q), _) if !q.is_empty() => q.clone(),
            _ => crate::tr("Untitled").into(),
        };
        f.set_ss_name(name.into());
        f.set_ss_open(true);
    }

    pub(super) fn save_search_done(&mut self) {
        let uif = self.ui();
        let f = uif.global::<F>();
        let name = f.get_ss_name().trim().to_string();
        let Some(saved) = self.current_search().filter(|_| !name.is_empty()) else { return };
        f.set_ss_open(false);
        match saved.save(&criteria::saved_dir(), &name) {
            Ok(p) => {
                let ps = p.to_string_lossy().into_owned();
                self.st.side_smart.retain(|x| x != &ps);
                if f.get_ss_side() {
                    self.st.side_smart.push(ps);
                }
                self.st.save();
                self.smart = Some((p.clone(), saved));
                self.loc = Loc::Smart(p);
                self.places();
                self.refresh();
            }
            Err(e) => {
                self.message(&crate::trf("The search “{name}” couldn't be saved.", &[("name", &name)]), &e.to_string())
            }
        }
    }

    fn connect_lists(&self) {
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_cs_favs(model(self.st.servers.iter().map(SharedString::from).collect()));
        f.set_cs_recent(model(self.st.recent_servers.iter().map(SharedString::from).collect()));
    }

    pub(super) fn connect_open(&mut self) {
        self.connect_lists();
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_cs_sel(-1);
        f.set_cs_addr(self.st.recent_servers.first().cloned().unwrap_or_default().into());
        f.set_cs_open(true);
    }

    pub(super) fn connect_add(&mut self, addr: &str) {
        let a = addr.trim().to_string();
        if !a.is_empty() && !self.st.servers.contains(&a) {
            self.st.servers.push(a);
            self.st.save();
        }
        self.connect_lists();
    }

    pub(super) fn connect_remove(&mut self, i: i32) {
        if (i as usize) < self.st.servers.len() {
            self.st.servers.remove(i as usize);
            self.st.save();
        }
        self.ui().global::<F>().set_cs_sel(-1);
        self.connect_lists();
    }

    /// Mount the server in the background, then open it.
    pub(super) fn connect(&mut self, addr: &str) {
        let a = normalize_server(addr);
        if a.is_empty() {
            return;
        }
        self.st.recent_servers.retain(|x| x != &a);
        self.st.recent_servers.insert(0, a.clone());
        self.st.recent_servers.truncate(10);
        self.st.save();
        if a.starts_with('/') || a.starts_with("file://") {
            self.ui().global::<F>().set_cs_open(false);
            return self.go(Loc::parse(&a), true);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.connect_rx = Some(rx);
        self.ui().global::<F>().set_cs_busy(true);
        std::thread::spawn(move || {
            let out = std::process::Command::new("gio").args(["mount", &a]).stdin(std::process::Stdio::null()).output();
            let already = |s: &str| s.contains("already mounted") || s.contains("Already mounted");
            let res = match out {
                Ok(o) if o.status.success() || already(&String::from_utf8_lossy(&o.stderr)) => mounted_path(&a)
                    .ok_or_else(|| crate::tr("The server was mounted but its folder couldn't be found.").to_string()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(res);
        });
    }

    fn connect_poll(&mut self) {
        let Some(res) = self.connect_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.connect_rx = None;
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_cs_busy(false);
        match res {
            Ok(p) => {
                f.set_cs_open(false);
                self.go(Loc::Dir(p), true);
                self.places();
            }
            Err(e) => self.message("There was a problem connecting to the server.", &e),
        }
    }

    pub(super) fn goto_edited(&mut self, text: &str) {
        let uif = self.ui();
        let f = uif.global::<F>();
        if f.get_goto_kind() != 0 {
            return;
        }
        let v: Vec<SharedString> = complete_path(text, self.st.hidden).into_iter().map(SharedString::from).collect();
        f.set_goto_sugg(model(v));
    }

    pub(super) fn goto(&mut self, text: &str) {
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_goto_open(false);
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        if f.get_goto_kind() == 1 || t.contains("://") && !t.starts_with("file://") {
            let t = t.to_string();
            return self.connect(&t);
        }
        let t = t.strip_prefix("file://").map(fs::percent_decode).unwrap_or_else(|| t.to_string());
        let home = fs::home();
        let p = if t == "~" {
            home
        } else if let Some(r) = t.strip_prefix("~/") {
            home.join(r)
        } else if t.starts_with('/') || t.contains(':') {
            return self.go(Loc::parse(&t), true);
        } else {
            self.loc.dir().map(|d| d.join(&t)).filter(|p| p.exists()).unwrap_or_else(|| home.join(&t))
        };
        self.go(Loc::Dir(p), true);
    }

    pub(super) fn poll(&mut self) {
        self.serve_ipc();
        self.pending_search();
        self.connect_poll();
        self.ql_tick();
        let mut changed = vec![];
        while let Ok(d) = self.worker.rx.try_recv() {
            let img = d.img.filter(|(w, h, data, _)| data.len() >= (*w * *h * 4) as usize && *w > 0 && *h > 0).map(
                |(w, h, data, premul)| {
                    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
                    buf.make_mut_bytes().copy_from_slice(&data[..(w * h * 4) as usize]);
                    if premul {
                        Image::from_rgba8_premultiplied(buf)
                    } else {
                        Image::from_rgba8(buf)
                    }
                },
            );
            if self.thumbs.len() > 3000 {
                self.thumbs.clear();
                self.requested.clear();
            }
            self.thumbs.insert(d.key.clone(), (img, d.snippet.into()));
            changed.push(d.key);
        }
        if !changed.is_empty() {
            let pv = self.sel_paths();
            let mut pv_dirty = false;
            for k in &changed {
                self.update_row(k);
                pv_dirty |= pv.len() == 1 && &pv[0] == k;
            }
            if pv_dirty || self.ui().global::<F>().get_view() == 2 {
                self.chrome();
            }
            if self.ui().global::<F>().get_ql_open() && pv.len() == 1 && changed.contains(&pv[0]) {
                self.ql_update();
            }
        }
        let mut got = vec![];
        let mut finished = false;
        if let Some(rx) = &self.search_rx {
            while let Ok((gen, v, done)) = rx.try_recv() {
                if gen == self.search_gen {
                    got.extend(v);
                    finished |= done;
                }
            }
        }
        if !got.is_empty() || finished {
            let sel = self.sel_paths();
            self.all.truncate(self.base_len);
            self.all.extend(got);
            self.base_len = self.all.len();
            self.sort_and_filter();
            self.sel = sel.iter().filter_map(|p| self.shown.iter().position(|&i| &self.all[i].path == p)).collect();
            self.refresh();
            if finished {
                self.search_rx = None;
                let ui_ = self.ui();
                let f = ui_.global::<F>();
                f.set_search_busy(false);
                if self.all.is_empty() {
                    f.set_empty_text(crate::tr("No results").into());
                }
            }
        }
        let mut sized = vec![];
        if let Some(rx) = &self.size_rx {
            while let Ok((d, n)) = rx.try_recv() {
                sized.push((d, n));
            }
        }
        for (d, n) in sized {
            self.sizes.insert(d.clone(), n);
            self.update_row(&d);
        }
        self.poll_transfer();
        self.poll_infos();
        if let Some((vi, t)) = self.label_click {
            if t.elapsed().as_millis() > 650 {
                self.label_click = None;
                let uif = self.ui();
                let f = uif.global::<F>();
                if self.sel == [vi] && f.get_renaming() < 0 && !f.get_dragging() && !f.get_menu_open() {
                    self.start_rename(vi);
                }
            }
        }
        if let Some((target, t)) = self.spring.clone() {
            let uif = self.ui();
            let f = uif.global::<F>();
            if !f.get_dragging() {
                self.spring = None;
            } else if t.elapsed().as_millis() > 1100 {
                self.spring = None;
                let loc = Loc::parse(&target);
                if matches!(&loc, Loc::Dir(d) if d.is_dir()) && loc != self.loc {
                    self.go(loc, true);
                }
            }
        }
    }
    /// The folder changed on disk (other apps, downloads): reload, keep the selection.
    pub(super) fn watch(&mut self) {
        let stamp = match &self.loc {
            Loc::Dir(p) => std::fs::metadata(p).ok().and_then(|m| m.modified().ok()),
            Loc::Trash => std::fs::metadata(fs::trash_dir().join("files")).ok().and_then(|m| m.modified().ok()),
            _ => return,
        };
        if stamp != self.dir_stamp
            && self.ui().global::<F>().get_renaming() < 0
            && !self.ui().global::<F>().get_dragging()
        {
            let sel = self.sel_paths();
            self.reload_keep(sel);
        }
    }
}
