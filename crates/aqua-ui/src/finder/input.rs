//! Pointer hit-testing, keyboard handling and the chooser's accept action.
use super::*;

impl App {
    /// Drop target under the pointer: (item index, sidebar place).
    pub(super) fn hit(&self, x: f32, y: f32) -> (i32, String) {
        if (8.0..208.0).contains(&x) {
            if let Some(p) = self.places.iter().find(|p| (p.0..p.1).contains(&y)) {
                if p.2 != "recents:" && p.2 != "apps:" {
                    return (-1, p.2.clone());
                }
            }
            return (-1, String::new());
        }
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let (cw, ch, cols) = self.geometry();
        let top = if f.get_mode() == 2 { 130.0 } else { 52.0 };
        let lx = x - 165.0;
        if lx < 0.0 || lx > cw || y < top || y > top + ch {
            return (-1, String::new());
        }
        let sy = -f.get_scroll_y();
        let idx = match f.get_view() {
            0 => {
                let pad = (cw - 16.0 - cols as f32 * 127.0) / 2.0;
                let c = ((lx - pad) / 127.0).floor();
                let r = ((y - top + sy) / 116.0).floor();
                if c < 0.0 || c as usize >= cols || r < 0.0 {
                    -1
                } else {
                    (r as usize * cols + c as usize) as i32
                }
            }
            1 => {
                let r = ((y - top - 32.0 + sy) / 20.0).floor();
                if r < 0.0 {
                    -1
                } else {
                    r as i32
                }
            }
            _ => -1,
        };
        if idx >= 0 && (idx as usize) < self.shown.len() {
            let e = &self.all[self.shown[idx as usize]];
            if e.is_dir && !self.drag.contains(&e.path) {
                return (idx, e.path.to_string_lossy().into_owned());
            }
        }
        (-1, String::new())
    }

    pub(super) fn key(&mut self, text: &str, cmd: bool, shift: bool, alt: bool, cols: usize) -> bool {
        use slint::platform::Key;
        let k = |key: Key| -> bool { text == SharedString::from(key).as_str() };
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let view = f.get_view();
        let n = self.shown.len();
        let mv = |s: &mut Self, d: i64| {
            if n == 0 {
                return;
            }
            let cur = s.focus.map(|x| x as i64).unwrap_or(if d > 0 { -1 } else { n as i64 });
            let next = (cur + d).clamp(0, n as i64 - 1) as usize;
            if shift {
                s.select(next, false, true);
                s.focus = Some(next);
            } else {
                s.select(next, false, false);
            }
        };
        let lower = text.to_lowercase();
        if cmd {
            match lower.as_str() {
                "o" if shift => self.go(dirs::document_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "o" => self.open_selection(),
                "n" if shift => self.new_folder(),
                "n" => spawn_finder(&self.loc.key()),
                "w" => {
                    if self.chooser.is_some() {
                        std::process::exit(1)
                    }
                    let _ = slint::quit_event_loop();
                }
                "c" if alt => self.action("copy-path"),
                "c" => self.copy(false),
                "x" => self.copy(true),
                "v" => self.paste(),
                "a" if !shift => {
                    if self.chooser.as_ref().map(|c| c.multiple).unwrap_or(true) {
                        self.sel = (0..n).filter(|&i| !self.dimmed(&self.all[self.shown[i]])).collect();
                        self.update_selection();
                    }
                }
                "a" if shift => self.go(Loc::Apps, true),
                "d" if shift => self.go(dirs::desktop_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "d" => self.duplicate(),
                "h" if shift => self.go(Loc::Dir(fs::home()), true),
                "l" if alt => self.go(dirs::download_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "f" if shift => self.go(Loc::Recents, true),
                "f" => f.set_searching(true),
                "g" if shift => f.set_goto_open(true),
                "i" => self.action("info"),
                "p" if shift => self.action("preview"),
                "r" => self.action("reveal"),
                "." | ">" if shift => self.action("hidden"),
                "[" => self.back(),
                "]" => self.forward(),
                "1" | "2" | "3" | "4" => {
                    let v = lower.parse::<i32>().unwrap() - 1;
                    self.set_view(v);
                }
                _ if k(Key::UpArrow) => self.up(),
                _ if k(Key::DownArrow) => self.open_selection(),
                _ if k(Key::Backspace) || k(Key::Delete) => {
                    if shift && matches!(self.loc, Loc::Trash) {
                        self.action("empty-trash")
                    } else if alt {
                        self.action("delete-now")
                    } else if matches!(self.loc, Loc::Trash) {
                        self.put_back()
                    } else {
                        self.trash_selection()
                    }
                }
                _ => return false,
            }
            return true;
        }
        if k(Key::LeftArrow) {
            match view {
                0 | 3 => mv(self, -1),
                2 => self.up(),
                _ => return false,
            }
        } else if k(Key::RightArrow) {
            match view {
                0 | 3 => mv(self, 1),
                2 => {
                    if let Some(e) = self.sel_entries().first().filter(|e| e.is_dir).map(|e| (*e).clone()) {
                        self.go(Loc::Dir(e.path.clone()), true);
                        if !self.shown.is_empty() {
                            self.select(0, false, false);
                        }
                    }
                }
                _ => return false,
            }
        } else if k(Key::UpArrow) {
            mv(self, if view == 0 { -(cols as i64) } else { -1 });
        } else if k(Key::DownArrow) {
            mv(self, if view == 0 { cols as i64 } else { 1 });
        } else if k(Key::Return) || text == "\r" || text == "\n" {
            if self.chooser.is_some() {
                self.accept();
            } else if let Some(&vi) = self.sel.first() {
                f.set_renaming(vi as i32);
            }
        } else if k(Key::F2) {
            if let Some(&vi) = self.sel.first() {
                f.set_renaming(vi as i32);
            }
        } else if k(Key::Delete) && self.chooser.is_none() {
            self.trash_selection();
        } else if k(Key::Backspace) && self.chooser.is_none() && alt {
            self.up();
        } else if text == " " {
            self.action("quicklook");
        } else if k(Key::Escape) {
            if self.chooser.is_some() {
                std::process::exit(1);
            }
            if f.get_searching() {
                f.set_searching(false);
                f.set_query("".into());
                self.search("");
            } else {
                self.select_none();
            }
        } else if k(Key::Home) {
            mv(self, -(n as i64));
        } else if k(Key::End) {
            mv(self, n as i64);
        } else if text.chars().count() == 1
            && text.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-')
        {
            if self.type_buf.1.elapsed().as_millis() > 900 {
                self.type_buf.0.clear();
            }
            self.type_buf.0.push_str(&lower);
            self.type_buf.1 = std::time::Instant::now();
            let pre = self.type_buf.0.clone();
            if let Some(vi) = self.shown.iter().position(|&i| self.all[i].name.to_lowercase().starts_with(&pre)) {
                self.select(vi, false, false);
            }
        } else {
            return false;
        }
        true
    }

    pub(super) fn set_view(&mut self, v: i32) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_view(v.clamp(0, 3));
        f.set_scroll_y(0.0);
        self.st.view = v.clamp(0, 3);
        self.st.save();
        if v == 3 && self.sel.is_empty() && !self.shown.is_empty() {
            self.select(0, false, false);
        }
        self.chrome();
    }

    pub(super) fn column_click(&mut self, ci: usize, i: usize, toggle: bool, range: bool) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let cols = f.get_columns();
        let n = cols.row_count();
        if ci + 1 == n {
            self.select(i, toggle, range);
            if !toggle && !range {
                if let Some(e) = self.sel_entries().first().filter(|e| e.is_dir).map(|e| (*e).clone()) {
                    self.go(Loc::Dir(e.path.clone()), true);
                }
            }
            return;
        }
        let Some(col) = cols.row_data(ci) else { return };
        let Some(it) = col.items.row_data(i) else { return };
        let p = PathBuf::from(it.path.as_str());
        if it.is_dir {
            self.go(Loc::Dir(p), true);
        } else {
            self.go(Loc::Dir(PathBuf::from(col.path.as_str())), true);
            self.select_path(&p);
        }
    }

    pub(super) fn accept(&mut self) {
        let Some(c) = self.chooser.clone() else {
            return self.open_selection();
        };
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let remember = |s: &mut Self, d: &Path| {
            s.st.chooser_dir = Some(d.to_path_buf());
            s.st.save();
        };
        if c.save {
            let name = f.get_save_name().trim().to_string();
            let Some(dir) = self.loc.dir().map(|d| d.to_path_buf()) else { return };
            if name.is_empty() {
                return;
            }
            if let Some(e) = self.sel_entries().first().filter(|e| e.is_dir && e.name == name).map(|e| (*e).clone()) {
                return self.go(Loc::Dir(e.path), true);
            }
            let target = dir.join(&name);
            if target.exists() && !matches!(self.pending, Pending::Replace(_)) {
                self.pending = Pending::Replace(target.clone());
                self.alert(&crate::trf("“{name}” already exists. Do you want to replace it?", &[("name", &name)]), "A file with the same name already exists in this folder. Replacing it will overwrite its current contents.", true);
                f.set_alert_ok(crate::tr("Replace").into());
                return;
            }
            self.pending = Pending::None;
            remember(self, &dir);
            let tags: Vec<String> = f.get_tag_defs().iter().filter(|d| d.on).map(|d| d.name.to_string()).collect();
            if !tags.is_empty() {
                for t in tags {
                    self.meta.toggle_tag(&target, &t, true);
                }
                self.meta.save();
            }
            finish(vec![target]);
        }
        let sel: Vec<Entry> = self.sel_entries().into_iter().cloned().collect();
        if c.directory {
            let dirs: Vec<PathBuf> = sel.iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
            let out =
                if dirs.is_empty() { self.loc.dir().map(|d| vec![d.to_path_buf()]).unwrap_or_default() } else { dirs };
            if let Some(d) = out.first().and_then(|d| d.parent()) {
                remember(self, d);
            }
            if !out.is_empty() {
                finish(out);
            }
            return;
        }
        if sel.len() == 1 && sel[0].is_dir {
            return self.go(Loc::Dir(sel[0].path.clone()), true);
        }
        let files: Vec<PathBuf> = sel.iter().filter(|e| !e.is_dir && !self.dimmed(e)).map(|e| e.path.clone()).collect();
        if let Some(d) = files.first().and_then(|d| d.parent()) {
            remember(self, d);
        }
        if !files.is_empty() {
            finish(files);
        }
    }
}
