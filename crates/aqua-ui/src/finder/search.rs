//! Search and background polling (directory watching, thumbnails).
use super::*;

impl App {
    pub(super) fn start_search(&mut self, root: PathBuf, q: String) {
        self.search_gen += 1;
        let gen = self.search_gen;
        let (tx, rx) = std::sync::mpsc::channel();
        self.search_rx = Some(rx);
        self.ui().global::<F>().set_search_busy(true);
        let hidden = self.st.hidden;
        std::thread::spawn(move || {
            let ql = q.to_lowercase();
            let mut batch = vec![];
            let mut found = 0usize;
            let mut seen = 0usize;
            let start = std::time::Instant::now();
            let mut last_send = std::time::Instant::now();
            fs::walk(&root, 12, &mut |e: Entry| {
                seen += 1;
                if (hidden || !e.name.starts_with('.')) && e.name.to_lowercase().contains(&ql) {
                    batch.push(e);
                    found += 1;
                }
                if last_send.elapsed().as_millis() > 250 && !batch.is_empty() {
                    if tx.send((gen, std::mem::take(&mut batch), false)).is_err() {
                        return false;
                    }
                    last_send = std::time::Instant::now();
                }
                found < 2000 && seen < 400_000 && start.elapsed().as_secs() < 30
            });
            let _ = tx.send((gen, batch, true));
        });
    }

    pub(super) fn search(&mut self, q: &str) {
        let q = q.trim().to_string();
        if q.is_empty() {
            if let Loc::Search(root, _) = &self.loc {
                let r = root.clone();
                self.loc = Loc::Dir(r);
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

    pub(super) fn search_commit(&mut self, q: &str) {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        let root = match &self.loc {
            Loc::Dir(p) | Loc::Search(p, _) => p.clone(),
            _ => fs::home(),
        };
        if !matches!(self.loc, Loc::Search(..)) {
            self.back.push(self.loc.clone());
            self.fwd.clear();
        }
        self.loc = Loc::Search(root, q);
        self.load();
    }

    pub(super) fn poll(&mut self) {
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
            self.all.extend(got);
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
