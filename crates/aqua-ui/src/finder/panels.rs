//! Quick Look, View Options and Settings.
use super::*;

fn opt(id: &str, label: &str, on: bool) -> FOpt {
    FOpt { id: id.into(), label: crate::tr(label).into(), on }
}

fn load_large(p: &Path) -> Option<Image> {
    let img = aqua_gfx::decode_limited(&aqua_gfx::read_regular(p, 512 << 20)?)?;
    let img = if img.width() > 1600 || img.height() > 1600 { img.thumbnail(1600, 1600) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(rgba.as_raw(), w, h);
    Some(Image::from_rgba8(buf))
}

fn rgba_image(w: u32, h: u32, data: &[u8]) -> Image {
    let n = (w * h * 4) as usize;
    if data.len() < n || n == 0 {
        return Image::default();
    }
    Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&data[..n], w, h))
}

fn read_text(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = super::thumbs::open_regular(p)?;
    let mut buf = vec![0u8; 96 * 1024];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.iter().take(4096).any(|&b| b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

impl App {
    pub(super) fn ql_open(&mut self) {
        if self.sel.is_empty() {
            if let Some(f) = self.focus {
                self.sel = vec![f];
                self.update_selection();
            } else {
                return;
            }
        }
        self.ui().global::<F>().set_ql_open(true);
        self.ql_update();
    }

    pub(super) fn ql_close(&mut self) {
        self.player = None;
        self.ql_pdf = None;
        self.ui().global::<F>().set_ql_open(false);
    }

    fn ql_media_state(&self) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let Some(p) = &self.player else {
            f.set_ql_media(false);
            return;
        };
        f.set_ql_media(true);
        f.set_ql_playing(p.playing());
        let d = p.info.duration;
        let pos = p.position();
        f.set_ql_frac(if d > 0.0 { (pos / d) as f32 } else { 0.0 });
        f.set_ql_time(format!("{} / {}", media::clock(pos), media::clock(d)).into());
    }

    /// Advance playback: show the newest frame and the time.
    pub(super) fn ql_tick(&mut self) {
        if self.player.is_none() {
            return;
        }
        if !self.ui().global::<F>().get_ql_open() {
            self.player = None;
            return;
        }
        let Some(p) = self.player.as_mut() else { return };
        let frame = p.frame().map(|f| (p.size, f));
        p.stop_if_done();
        if let Some(((w, h), data)) = frame {
            let ui_ = self.ui();
            let f = ui_.global::<F>();
            f.set_ql_img(rgba_image(w, h, &data));
            f.set_ql_has_img(true);
        }
        self.ql_media_state();
    }

    pub(super) fn ql_play(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.toggle();
        }
        self.ql_media_state();
    }

    pub(super) fn ql_seek(&mut self, frac: f32) {
        if let Some(p) = self.player.as_mut() {
            p.seek(frac as f64);
        }
        self.ql_media_state();
    }

    pub(super) fn ql_page(&mut self, d: i32) {
        let Some((path, page, pages)) = self.ql_pdf.clone() else { return };
        let next = (page as i32 + d).clamp(1, pages as i32) as usize;
        if next == page {
            return;
        }
        self.ql_pdf = Some((path.clone(), next, pages));
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        if let Some((w, h, data)) = media::pdf_page(&path, next) {
            f.set_ql_img(rgba_image(w, h, &data));
            f.set_ql_has_img(true);
        }
        f.set_ql_page(next as i32);
    }

    pub(super) fn ql_update(&mut self) {
        let Some(vi) = self.sel_or_focus_pub() else { return self.ql_close() };
        let e = self.all[self.shown[vi]].clone();
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let it = self.item(&e, false, &[]);
        let (img, text) = if e.kind == 2 && !e.is_dir {
            (load_large(&e.path).or_else(|| it.has_thumb.then(|| it.thumb.clone())), None)
        } else if it.has_thumb && matches!(e.kind, 4 | 8) {
            (Some(it.thumb.clone()), None)
        } else if e.kind == 6 && !e.is_dir {
            (None, read_text(&e.path))
        } else {
            (None, None)
        };
        self.player = None;
        self.ql_pdf = None;
        let mut img = img;
        let mut pages = 0;
        if e.kind == 8 && !e.is_dir {
            pages = media::pdf_pages(&e.path);
            if let Some((w, h, data)) = (pages > 0).then(|| media::pdf_page(&e.path, 1)).flatten() {
                img = Some(rgba_image(w, h, &data));
                self.ql_pdf = Some((e.path.clone(), 1, pages));
            }
        }
        if matches!(e.kind, 3 | 4) && !e.is_dir {
            if let Some(info) = media::probe(&e.path) {
                let mut p = media::Player::new(&e.path, info);
                if let Some(data) = media::poster(&e.path, p.size) {
                    img = Some(rgba_image(p.size.0, p.size.1, &data));
                }
                if !p.info.has_video() {
                    img = None;
                }
                p.play(0.0);
                self.player = Some(p);
            }
        }
        f.set_ql_pages(if self.ql_pdf.is_some() { pages as i32 } else { 0 });
        f.set_ql_page(1);
        f.set_ql_has_img(img.is_some());
        f.set_ql_img(img.unwrap_or_default());
        self.ql_media_state();
        f.set_ql_text(text.unwrap_or_default().into());
        let mut info = vec![e.label.clone()];
        if !e.is_dir && e.app.is_none() {
            info.push(fs::human(e.size));
        }
        if e.app.is_none() {
            info.push(crate::trf("Modified: {date}", &[("date", &fs::short_date(e.mtime))]));
        }
        f.set_ql_info(info.join("  ·  ").into());
        let n = if self.sel.len() > 1 { self.sel.len() } else { self.shown.len() };
        let pos = if self.sel.len() > 1 { self.sel.iter().position(|&s| s == vi).unwrap_or(0) } else { vi };
        f.set_ql_pos(if n > 1 {
            crate::trf("{i} of {n}", &[("i", &(pos + 1)), ("n", &n)]).into()
        } else {
            SharedString::new()
        });
        f.set_ql(it);
    }

    pub(super) fn sel_or_focus_pub(&self) -> Option<usize> {
        self.focus.filter(|f| self.sel.contains(f)).or_else(|| self.sel.last().copied())
    }

    pub(super) fn ql_step(&mut self, d: i32) {
        let n = self.shown.len();
        if n == 0 {
            return;
        }
        if self.sel.len() > 1 {
            let cur = self.sel_or_focus_pub().unwrap_or(self.sel[0]);
            let pos = self.sel.iter().position(|&s| s == cur).unwrap_or(0) as i32;
            let next = (pos + d).rem_euclid(self.sel.len() as i32) as usize;
            self.focus = Some(self.sel[next]);
            self.ql_update();
            return;
        }
        let cur = self.focus.unwrap_or(0) as i32;
        let next = (cur + d).clamp(0, n as i32 - 1) as usize;
        self.select(next, false, false);
        self.reveal(next);
    }

    pub(super) fn fill_view_options(&mut self) {
        let uif = self.ui();
        let f = uif.global::<F>();
        let cols: Vec<FOpt> =
            [(2, "Date Modified"), (4, "Date Created"), (5, "Date Last Opened"), (3, "Size"), (1, "Kind"), (6, "Tags")]
                .iter()
                .map(|(k, n)| opt(&k.to_string(), n, self.st.list_cols.contains(k)))
                .collect();
        f.set_vo_cols(model(cols));
    }

    pub(super) fn fill_prefs(&mut self) {
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_pref_new_window(self.st.new_window);
        f.set_pref_scope(self.st.scope);
        f.set_pref_general(model(vec![opt(
            "open-tabs",
            "Open folders in tabs instead of new windows",
            self.st.open_tabs,
        )]));
        let side_on = |id: &str| !self.st.side_hidden.iter().any(|h| h == id);
        f.set_pref_side(model(
            [
                ("recents", "Recents"),
                ("apps", "Applications"),
                ("desktop", "Desktop"),
                ("docs", "Documents"),
                ("downloads", "Downloads"),
                ("pictures", "Pictures"),
                ("music", "Music"),
                ("movies", "Movies"),
                ("home", "Home"),
                ("computer", "Computer"),
                ("disks", "External disks"),
                ("trash", "Trash"),
            ]
            .iter()
            .map(|(id, l)| opt(id, l, side_on(id)))
            .collect(),
        ));
        f.set_pref_tags(model(
            fs::TAGS.iter().map(|(n, _)| opt(n, n, !self.st.tags_hidden.iter().any(|t| t == n))).collect(),
        ));
        f.set_pref_adv(model(vec![
            opt("show-ext", "Show all filename extensions", self.st.show_ext),
            opt("warn-ext", "Show warning before changing an extension", self.st.warn_ext),
            opt("warn-trash", "Show warning before emptying the Trash", self.st.warn_trash),
            opt("trash-30", "Remove items from the Trash after 30 days", self.st.trash_30),
            opt("folders-first", "Keep folders on top when sorting by name", self.st.folders_first),
        ]));
    }

    /// View Options / Settings controls: `id` names the control, `v` its new value.
    pub(super) fn opt(&mut self, id: &str, v: i32) {
        let on = v != 0;
        let toggle_in = |list: &mut Vec<String>, item: &str, show: bool| {
            list.retain(|x| x != item);
            if !show {
                list.push(item.to_string());
            }
        };
        let mut reload = false;
        let mut relayout = false;
        match id {
            "group" => {
                self.st.group = v.clamp(0, 7);
                reload = true;
            }
            "sort" => {
                self.st.sort = (v.clamp(0, 5), self.st.sort.1);
                reload = true;
            }
            "icon-sort" => {
                let Loc::Dir(d) = self.loc.clone() else { return };
                if v <= 1 {
                    let fo = self.free_folder().unwrap_or_else(|| {
                        let names: Vec<String> = self.shown.iter().map(|&i| self.all[i].name.clone()).collect();
                        arrange::Folder { snap: false, pos: arrange::in_order(&self.grid, &names) }
                    });
                    self.arr.set(&d, Some(arrange::Folder { snap: v == 1, ..fo }));
                    relayout = true;
                } else {
                    if self.arr.get(&d).is_some() {
                        self.arr.set(&d, None);
                    }
                    self.st.sort = ((v - 2).clamp(0, 5), false);
                    reload = true;
                }
            }
            "gap" => {
                self.st.gap = v.clamp(0, 100) as f32;
                relayout = true;
            }
            "text" => {
                if self.view() == 0 {
                    self.st.icon_text = v.clamp(10, 16) as f32;
                } else {
                    self.st.list_text = v.clamp(10, 16) as f32;
                }
                relayout = true;
            }
            "info" => {
                self.st.item_info = on;
                reload = true;
            }
            "icon-preview" => {
                self.st.icon_preview = on;
                reload = true;
            }
            "col-icons" => self.st.col_icons = on,
            "col-preview" => self.st.col_preview = on,
            "rel-dates" => {
                self.st.rel_dates = on;
                reload = true;
            }
            "calc-sizes" => {
                self.st.calc_sizes = on;
                reload = true;
            }
            "defaults" => {}
            "new-window" => self.st.new_window = v.clamp(0, 4),
            "scope" => self.st.scope = v.clamp(0, 2),
            "side-style" => crate::set_sidebar_style(v),
            "general:open-tabs" => self.st.open_tabs = on,
            "adv:show-ext" => {
                self.st.show_ext = on;
                reload = true;
            }
            "adv:warn-ext" => self.st.warn_ext = on,
            "adv:warn-trash" => self.st.warn_trash = on,
            "adv:trash-30" => self.st.trash_30 = on,
            "adv:folders-first" => {
                self.st.folders_first = on;
                reload = true;
            }
            s if s.starts_with("side:") => {
                toggle_in(&mut self.st.side_hidden, &s[5..], on);
                self.st.save();
                self.places();
            }
            s if s.starts_with("tags:") => {
                toggle_in(&mut self.st.tags_hidden, &s[5..], on);
                self.st.save();
                self.places();
            }
            s if s.starts_with("col:") => {
                let k: i32 = s[4..].parse().unwrap_or(0);
                self.st.list_cols.retain(|&c| c != k);
                if on {
                    self.st.list_cols.push(k);
                }
                relayout = true;
            }
            _ => return,
        }
        self.st.save();
        if reload {
            self.item_info.clear();
            self.resort();
            self.want_sizes();
        } else if relayout {
            self.layout_rows();
        }
        self.chrome();
        self.fill_view_options();
        self.fill_prefs();
    }
}
