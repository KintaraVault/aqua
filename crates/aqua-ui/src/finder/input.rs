//! Pointer hit-testing, marquee selection, keyboard handling and the chooser's accept action.
use super::*;

/// Sidebar geometry: rows start below the traffic lights, every row (header or place) is 32px.
pub(super) const SIDE_TOP: f32 = 52.0;
pub(super) const SIDE_ROW: f32 = 32.0;

/// Sidebar row under window point (x, y) given the scroll offset (content-y, ≤ 0).
pub(super) fn side_row(x: f32, y: f32, side_w: f32, scroll: f32, rows: usize) -> Option<(usize, f32)> {
    if !(8.0..8.0 + side_w).contains(&x) || y < SIDE_TOP {
        return None;
    }
    let cy = y - SIDE_TOP - scroll;
    let r = (cy / SIDE_ROW).floor();
    if r < 0.0 || r as usize >= rows {
        return None;
    }
    Some((r as usize, cy - r * SIDE_ROW))
}

impl App {
    /// Content point (x, y) relative to the item area, scroll included.
    fn content_point(&self, x: f32, y: f32) -> (f32, f32) {
        let uif = self.ui();
        let f = uif.global::<F>();
        let (cx, top, _, _) = self.geo;
        let list = if f.get_view() == 1 { layout::LIST_TOP } else { 0.0 };
        (x - cx, y - top - list - f.get_scroll_y())
    }

    /// Item index under a window point in the icon and list views.
    pub(super) fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let (cx, top, cw, ch) = self.geo;
        if x < cx || x > cx + cw || y < top || y > top + ch {
            return None;
        }
        let (px, py) = self.content_point(x, y);
        match self.view() {
            0 => match &self.spots {
                Some(sp) => sp.iter().rposition(|&(cx, cy)| self.grid.cell_hit(cx, cy, px, py)),
                None => self.grid.hit(&self.rows, &self.offs, self.total, px, py),
            },
            1 => {
                if py < 0.0 || px < 10.0 || px > cw - 10.0 {
                    return None;
                }
                let r = &self.rows[layout::row_at(&self.offs, self.total, py)?];
                (r.count > 0).then_some(r.start)
            }
            _ => None,
        }
    }

    /// Drop target under the pointer: (item index, target path, sidebar insertion line).
    pub(super) fn hit(&self, x: f32, y: f32) -> (i32, String, f32) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let side_w = if f.get_show_sidebar() || f.get_mode() != 0 { f.get_side_w() } else { 0.0 };
        if x < 8.0 + side_w {
            let Some((r, dy)) = side_row(x, y, side_w, f.get_side_scroll(), self.side_rows.len()) else {
                return (-1, String::new(), -1.0);
            };
            let p = &self.side_rows[r];
            let dirs_only = !self.drag.is_empty() && self.drag.iter().all(|d| d.is_dir());
            let fav = self.favorites_range();
            if dirs_only && fav.as_ref().is_some_and(|f| r >= f.start && r <= f.end) {
                let edge = if p.is_empty() || dy > SIDE_ROW - 7.0 {
                    Some(r + 1)
                } else if dy < 7.0 {
                    Some(r)
                } else {
                    None
                };
                if let Some(at) = edge {
                    let at = at.clamp(fav.as_ref().unwrap().start, fav.as_ref().unwrap().end);
                    return (-1, format!("insert:{at}"), at as f32 * SIDE_ROW);
                }
            }
            if !p.is_empty() && p != "recents:" && p != "apps:" {
                return (-1, p.clone(), -1.0);
            }
            return (-1, String::new(), -1.0);
        }
        if f.get_view() == 2 {
            return self.column_hit(x, y);
        }
        if let Some(i) = self.item_at(x, y) {
            let e = &self.all[self.shown[i]];
            if e.is_dir && !self.drag.contains(&e.path) {
                return (i as i32, e.path.to_string_lossy().into_owned(), -1.0);
            }
        }
        let (cx, top, cw, ch) = self.geo;
        if x >= cx && x <= cx + cw && y >= top && y <= top + ch {
            if let Some(d) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))) {
                let from_here = self.drag.iter().all(|p| p.parent() == Some(d));
                if !from_here {
                    return (-1, d.to_string_lossy().into_owned(), -1.0);
                }
            }
        }
        (-1, String::new(), -1.0)
    }

    fn column_hit(&self, x: f32, y: f32) -> (i32, String, f32) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let (cx, top, cw, ch) = self.geo;
        if x < cx || x > cx + cw || y < top || y > top + ch {
            return (-1, String::new(), -1.0);
        }
        let cols = f.get_columns();
        let n = cols.row_count();
        let pv = if self.st.col_preview { 280.0 } else { 0.0 };
        let total = n as f32 * layout::COLUMN_W + pv;
        let off = (cw - total).min(0.0);
        let ci = ((x - cx - off) / layout::COLUMN_W).floor();
        if ci < 0.0 || ci as usize >= n {
            return (-1, String::new(), -1.0);
        }
        let Some(col) = cols.row_data(ci as usize) else { return (-1, String::new(), -1.0) };
        let ri = ((y - top) / 24.0).floor() as usize;
        if let Some(it) = col.items.row_data(ri) {
            if it.is_dir && !self.drag.iter().any(|d| d.to_string_lossy() == it.path.as_str()) {
                return (-1, it.path.to_string(), -1.0);
            }
        }
        if Path::new(col.path.as_str()).is_dir() {
            return (-1, col.path.to_string(), -1.0);
        }
        (-1, String::new(), -1.0)
    }

    /// Rubber-band selection: phase 0 press, 1 drag, 2 release (window coordinates).
    pub(super) fn marquee(&mut self, phase: i32, x: f32, y: f32, additive: bool) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        match phase {
            0 => {
                f.set_renaming(-1);
                let (px, py) = self.content_point(x, y);
                let base = if additive { self.sel.clone() } else { vec![] };
                if !additive {
                    self.select_none();
                }
                self.mq = Some(Marquee { x: px, y: py, base, additive, moved: false });
            }
            1 => {
                let Some(m) = &self.mq else { return };
                let (px, py) = self.content_point(x, y);
                if !m.moved && (px - m.x).abs() < 3.0 && (py - m.y).abs() < 3.0 {
                    return;
                }
                let (sx, sy) = (m.x, m.y);
                let hits: Vec<usize> = match f.get_view() {
                    0 => match &self.spots {
                        Some(sp) => (0..sp.len())
                            .filter(|&i| self.grid.cell_touches(sp[i].0, sp[i].1, (sx, sy, px, py)))
                            .collect(),
                        None => self.grid.in_rect(&self.rows, &self.offs, (sx, sy, px, py)),
                    },
                    1 => {
                        let (y0, y1) = (sy.min(py), sy.max(py));
                        self.rows
                            .iter()
                            .zip(&self.offs)
                            .filter(|(r, &o)| r.count > 0 && o + layout::ROW_LIST > y0 && o < y1)
                            .map(|(r, _)| r.start)
                            .collect()
                    }
                    _ => vec![],
                };
                let m = self.mq.as_mut().unwrap();
                m.moved = true;
                let mut sel: Vec<usize> = if m.additive {
                    let mut s: Vec<usize> = m.base.iter().filter(|i| !hits.contains(i)).copied().collect();
                    s.extend(hits.iter().filter(|i| !m.base.contains(i)));
                    s
                } else {
                    hits
                };
                sel.retain(|&i| i < self.shown.len() && !self.dimmed(&self.all[self.shown[i]]));
                if self.chooser.as_ref().is_some_and(|c| !c.multiple) {
                    sel.truncate(1);
                }
                if sel != self.sel {
                    self.sel = sel;
                    self.focus = self.sel.last().copied();
                    self.anchor = self.sel.first().copied();
                    self.update_selection();
                }
                let (cx, top, cw, ch) = self.geo;
                let list = if f.get_view() == 1 { layout::LIST_TOP } else { 0.0 };
                let scroll = f.get_scroll_y();
                let to_win =
                    |px: f32, py: f32| ((cx + px).clamp(cx, cx + cw), (top + list + py + scroll).clamp(top, top + ch));
                let (ax, ay) = to_win(sx, sy);
                let (bx, by) = to_win(px, py);
                f.set_mq_x(ax.min(bx));
                f.set_mq_y(ay.min(by));
                f.set_mq_w((ax - bx).abs());
                f.set_mq_h((ay - by).abs());
                f.set_mq_on(true);
            }
            _ => {
                self.mq = None;
                f.set_mq_on(false);
            }
        }
    }

    pub(super) fn clean_up(&mut self, by: Option<i32>) {
        let (Some(fo), Some(sp), Loc::Dir(d)) = (self.free_folder(), self.spots.clone(), self.loc.clone()) else {
            return;
        };
        let names: Vec<String> = self.shown.iter().map(|&i| self.all[i].name.clone()).collect();
        let pos = match by {
            Some(k) => {
                let mut order: Vec<usize> = (0..self.shown.len()).collect();
                order.sort_by(|&a, &b| {
                    cmp_entries(&self.all[self.shown[a]], &self.all[self.shown[b]], k, self.st.folders_first)
                });
                let sorted: Vec<String> = order.iter().map(|&i| names[i].clone()).collect();
                arrange::in_order(&self.grid, &sorted)
            }
            None => arrange::clean_up(&self.grid, &names, &sp),
        };
        self.arr.set(&d, Some(arrange::Folder { pos, ..fo }));
        self.layout_rows();
    }

    /// Icons dragged to a new place inside a hand-arranged folder.
    pub(super) fn move_icons(&mut self, x: f32, y: f32) -> bool {
        let (Some(mut fo), Some(sp), Loc::Dir(d)) = (self.free_folder(), self.spots.clone(), self.loc.clone()) else {
            return false;
        };
        let Some((ox, oy)) = self.drag_origin.take() else { return false };
        let moving: Vec<usize> =
            self.drag.iter().filter_map(|p| self.shown.iter().position(|&i| &self.all[i].path == p)).collect();
        if moving.len() != self.drag.len() {
            return false;
        }
        let names: Vec<String> = self.shown.iter().map(|&i| self.all[i].name.clone()).collect();
        arrange::shift(&self.grid, &mut fo, &names, &sp, &moving, (x - ox, y - oy));
        self.arr.set(&d, Some(fo));
        self.drag.clear();
        self.layout_rows();
        true
    }

    pub(super) fn relayout(&mut self, _view: i32, cx: f32, top: f32, cw: f32, ch: f32) {
        let old = self.geo;
        self.geo = (cx, top, cw, ch);
        if old.2 != cw || self.rows.is_empty() != self.shown.is_empty() {
            self.layout_rows();
        }
    }

    pub(super) fn start_rename(&mut self, vi: usize) {
        if self.chooser.is_some()
            || vi >= self.shown.len()
            || !matches!(self.loc, Loc::Dir(_) | Loc::Search(..) | Loc::Smart(_) | Loc::Recents | Loc::Tag(_))
        {
            return;
        }
        let e = &self.all[self.shown[vi]];
        let shown_name = fs::display_name(&e.name, e.is_dir, self.st.show_ext);
        let stem = if e.is_dir { shown_name.len() } else { fs::split_ext(&shown_name).0.len() };
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_rename_sel(stem as i32);
        f.set_renaming(vi as i32);
        self.label_click = None;
        self.reveal(vi);
    }

    fn sel_or_focus(&self) -> Option<usize> {
        self.focus.filter(|f| self.sel.contains(f)).or_else(|| self.sel.last().copied())
    }

    /// Expand / collapse a folder row of the list view.
    pub(super) fn expand(&mut self, vi: usize, open: bool, all: bool) {
        let Some(&i) = self.shown.get(vi) else { return };
        let e = self.all[i].clone();
        if !e.is_dir || !self.tree() {
            return;
        }
        let sel = self.sel_paths();
        if open {
            self.expanded.insert(e.path.clone());
            if all {
                let mut stack = vec![(e.path.clone(), 0)];
                while let Some((d, depth)) = stack.pop() {
                    if depth >= 4 {
                        continue;
                    }
                    for k in self.children(&d) {
                        if k.is_dir && (self.st.hidden || !k.name.starts_with('.')) {
                            self.expanded.insert(k.path.clone());
                            stack.push((k.path.clone(), depth + 1));
                        }
                    }
                }
            }
        } else {
            self.expanded.retain(|p| !(p == &e.path || all && p.starts_with(&e.path)));
        }
        self.sort_and_filter();
        self.refresh();
        let keep: Vec<PathBuf> = sel.into_iter().filter(|p| self.row_of.contains_key(p)).collect();
        if keep.is_empty() && !open {
            self.select_path(&e.path);
        } else {
            self.select_paths(&keep);
        }
    }

    pub(super) fn key(&mut self, text: &str, ctrl: bool, meta: bool, shift: bool, alt: bool) -> bool {
        use slint::platform::Key;
        let k = |key: Key| -> bool { text == SharedString::from(key).as_str() };
        let cmd = ctrl || meta;
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let view = f.get_view();
        let n = self.shown.len();
        let cols = if view == 0 { self.grid.cols } else { 1 };
        if f.get_ql_open() && (text == " " || k(Key::Escape) || cmd && text.to_lowercase() == "y") {
            self.ql_close();
            return true;
        }
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
            s.reveal(next);
        };
        let vert = |s: &mut Self, down: bool| {
            if view != 0 || s.rows.is_empty() {
                return mv(s, if down { 1 } else { -1 });
            }
            let Some(cur) = s.focus else { return mv(s, if down { 1 } else { -1 }) };
            let Some(r) = s.rows.iter().position(|r| r.count > 0 && cur >= r.start && cur < r.start + r.count) else {
                return;
            };
            let col = cur - s.rows[r].start;
            let mut t = r as i64;
            loop {
                t += if down { 1 } else { -1 };
                if t < 0 || t as usize >= s.rows.len() {
                    return;
                }
                let row = &s.rows[t as usize];
                if row.count > 0 {
                    let target = row.start + col.min(row.count - 1);
                    return mv(s, target as i64 - cur as i64);
                }
            }
        };
        let lower = text.to_lowercase();
        if k(Key::Tab) && !cmd {
            if n > 0 {
                let mut order: Vec<usize> = (0..n).collect();
                order.sort_by(|&a, &b| fs::natural(&self.all[self.shown[a]].name, &self.all[self.shown[b]].name));
                let pos = self.focus.and_then(|c| order.iter().position(|&x| x == c));
                let next = match (pos, shift) {
                    (None, false) => order[0],
                    (None, true) => order[n - 1],
                    (Some(p), false) => order[(p + 1) % n],
                    (Some(p), true) => order[(p + n - 1) % n],
                };
                self.select(next, false, false);
                self.reveal(next);
            }
            return true;
        }
        if ctrl && k(Key::Tab) {
            if self.tabs.len() > 1 {
                let i = if shift {
                    (self.tab + self.tabs.len() - 1) % self.tabs.len()
                } else {
                    (self.tab + 1) % self.tabs.len()
                };
                self.select_tab(i);
            }
            return true;
        }
        if cmd {
            let both = ctrl && meta;
            if both && alt && shift {
                match lower.as_str() {
                    "u" => self.action("clean-up"),
                    "b" => self.action("tb-customize"),
                    "n" => self.action("detach-tab"),
                    "m" => self.action("merge-windows"),
                    _ => return false,
                }
                return true;
            }
            match lower.as_str() {
                "1" | "2" | "3" | "4" | "5" | "6" if both && alt => {
                    self.st.sort = (lower.parse::<i32>().unwrap() - 1, false);
                    self.resort();
                }
                "o" if shift => self.go(dirs::document_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "o" => self.open_selection(),
                "n" if both => self.action("new-folder-sel"),
                "n" if alt && !shift => self.action("new-smart"),
                "n" if shift => self.new_folder(),
                "n" => spawn_finder(&self.start_loc().key()),
                "t" if both || alt && shift => self.action("add-sidebar"),
                "t" if shift => self.action("tab-bar"),
                "t" => {
                    let l = self.start_loc();
                    self.new_tab(l, false)
                }
                "w" if alt => self.close_window(),
                "w" => self.close_tab(self.tab),
                "z" if shift => self.redo(),
                "z" => self.undo(),
                "c" if alt => self.action("copy-path"),
                "c" if shift => self.go(Loc::Dir("/".into()), true),
                "c" => self.copy(false),
                "x" => self.copy(true),
                "v" if alt => self.paste_move(),
                "v" => self.paste(),
                "a" if both => self.make_alias(),
                "a" if alt => self.select_none(),
                "a" if shift => self.go(Loc::Apps, true),
                "a" => {
                    if self.chooser.as_ref().map(|c| c.multiple).unwrap_or(true) {
                        self.sel = (0..n).filter(|&i| !self.dimmed(&self.all[self.shown[i]])).collect();
                        self.update_selection();
                    }
                }
                "d" if shift => self.go(dirs::desktop_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "d" => self.duplicate(),
                "e" => self.eject_selection(),
                "h" if shift => self.go(Loc::Dir(fs::home()), true),
                "l" if alt => self.go(dirs::download_dir().map(Loc::Dir).unwrap_or(Loc::Dir(fs::home())), true),
                "l" => self.make_alias(),
                "f" if both => f.set_searching(true),
                "f" if shift => self.go(Loc::Recents, true),
                "f" => f.set_searching(true),
                "g" if shift => self.action("goto"),
                "k" => self.action("connect"),
                "i" if alt => self.action("info"),
                "i" if both => self.action("summary-info"),
                "i" => self.action("info-window"),
                "j" => f.set_vo_open(!f.get_vo_open()),
                "y" => self.action("quicklook"),
                "p" if alt => self.action("path-bar"),
                "p" if shift => self.action("preview"),
                "s" if both || alt => self.action("sidebar"),
                "r" => self.action("reveal"),
                "/" => self.action("status-bar"),
                "," => f.set_prefs_open(true),
                "." | ">" if shift => self.action("hidden"),
                "=" | "+" => self.set_zoom(settings::zoom_of(self.st.icon * 1.25)),
                "-" => self.set_zoom(settings::zoom_of(self.st.icon / 1.25)),
                "[" => self.back(),
                "]" => self.forward(),
                "1" | "2" | "3" | "4" => {
                    let v = lower.parse::<i32>().unwrap() - 1;
                    self.set_view(v);
                }
                "0" if both => self.action("groups-toggle"),
                _ if k(Key::UpArrow) => self.up(),
                _ if k(Key::DownArrow) => self.open_selection(),
                _ if k(Key::Backspace) || k(Key::Delete) => {
                    if shift && matches!(self.loc, Loc::Trash) || shift && self.chooser.is_none() {
                        if alt {
                            self.empty_trash_now();
                        } else {
                            self.action("empty-trash");
                        }
                    } else if alt {
                        self.action("delete-now");
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
                1 => {
                    if let Some(vi) = self.sel_or_focus() {
                        let e = &self.all[self.shown[vi]];
                        if e.is_dir && self.expanded.contains(&e.path) {
                            self.expand(vi, false, alt);
                        } else if self.depth.get(vi).copied().unwrap_or(0) > 0 {
                            if let Some(p) = e.path.parent().map(|p| p.to_path_buf()) {
                                self.select_path(&p);
                            }
                        }
                    }
                }
                2 => self.up(),
                _ => return false,
            }
        } else if k(Key::RightArrow) {
            match view {
                0 | 3 => mv(self, 1),
                1 => {
                    if let Some(vi) = self.sel_or_focus() {
                        self.expand(vi, true, alt);
                    }
                }
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
            vert(self, false);
        } else if k(Key::DownArrow) {
            vert(self, true);
        } else if k(Key::PageUp) || k(Key::PageDown) {
            let step = if view == 0 {
                cols as i64 * ((self.geo.3 / self.grid.cell_h).floor() as i64).max(1)
            } else {
                ((self.geo.3 / layout::ROW_LIST).floor() as i64).max(1)
            };
            mv(self, if k(Key::PageUp) { -step } else { step });
        } else if k(Key::Return) || text == "\r" || text == "\n" {
            if self.chooser.is_some() {
                self.accept();
            } else if self.sel.len() > 1 {
                self.action("rename-multi");
            } else if let Some(&vi) = self.sel.first() {
                self.start_rename(vi);
            }
        } else if k(Key::F2) {
            if let Some(&vi) = self.sel.first() {
                self.start_rename(vi);
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
            && !ctrl
        {
            if self.type_buf.1.elapsed().as_millis() > 900 {
                self.type_buf.0.clear();
            }
            self.type_buf.0.push_str(&lower);
            self.type_buf.1 = std::time::Instant::now();
            let pre = self.type_buf.0.clone();
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|&a, &b| fs::natural(&self.all[self.shown[a]].name, &self.all[self.shown[b]].name));
            let hit = order
                .iter()
                .copied()
                .find(|&vi| self.all[self.shown[vi]].name.to_lowercase().starts_with(&pre))
                .or_else(|| {
                    order
                        .iter()
                        .copied()
                        .find(|&vi| self.all[self.shown[vi]].name.to_lowercase().as_str() >= pre.as_str())
                });
            if let Some(vi) = hit {
                self.select(vi, false, false);
                self.reveal(vi);
            }
        } else {
            return false;
        }
        true
    }

    pub(super) fn set_view(&mut self, v: i32) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let v = v.clamp(0, 3);
        let sel = self.sel_paths();
        f.set_view(v);
        f.set_scroll_y(0.0);
        self.st.view = v;
        self.st.save();
        self.sort_and_filter();
        self.refresh();
        self.select_paths(&sel);
        if v == 3 && self.sel.is_empty() && !self.shown.is_empty() {
            self.select(0, false, false);
        }
        self.want_sizes();
    }

    pub(super) fn set_zoom(&mut self, z: f32) {
        let icon = settings::icon_of(z);
        if icon == self.st.icon {
            return;
        }
        self.st.icon = icon;
        self.st.save();
        self.item_info.clear();
        self.layout_rows();
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
