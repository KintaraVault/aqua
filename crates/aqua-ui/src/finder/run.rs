//! Window setup and callback wiring.
use super::*;

pub(super) fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    }
}

pub(super) fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::tr("Computer").into())
}

/// Separator between chosen paths on stdout (zenity: `|`, kdialog: space); one per line by default.
pub static OUTPUT_SEP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub(super) fn finish(paths: Vec<PathBuf>) -> ! {
    let sep = OUTPUT_SEP.get().map(String::as_str).unwrap_or("\n");
    let all: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    println!("{}", all.join(sep));
    std::process::exit(0);
}

pub(super) fn open_path(p: &Path) {
    let q = fs::sh_quote(p);
    aqua_apps::launch(&format!("xdg-open {q} || gio open {q}"));
}

pub(super) fn open_terminal(dir: &Path) {
    let q = fs::sh_quote(dir);
    let cfg = aqua_config::Config::load();
    for t in cfg.terminal.split('|') {
        let t = t.trim();
        if t.is_empty() || aqua_apps::find_in_path(t).is_none() {
            continue;
        }
        let cmd = match t {
            "foot" | "kitty" | "alacritty" | "wezterm" | "ghostty" => format!("cd {q} && exec {t}"),
            "gnome-terminal" | "kgx" | "xfce4-terminal" | "tilix" => format!("{t} --working-directory={q}"),
            "konsole" => format!("konsole --workdir {q}"),
            _ => format!("cd {q} && exec {t}"),
        };
        aqua_apps::launch(&cmd);
        return;
    }
}

pub(super) fn spawn_finder(target: &str) {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("aqua-finder")))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "aqua-finder".into());
    aqua_apps::launch(&format!("{} {}", fs::sh_quote(Path::new(&exe)), fs::sh_quote(Path::new(target))));
}

/// Add an opened file to GTK's recently-used list (so Recents fills up).
pub(super) fn record_recent(p: &Path) {
    let xbel = dirs::data_dir().unwrap_or_else(|| fs::home().join(".local/share")).join("recently-used.xbel");
    let uri = fs::file_uri(p);
    let mut s = std::fs::read_to_string(&xbel)
        .unwrap_or_else(|_| "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\">\n</xbel>\n".into());
    let n = fs::now_secs();
    let t = unsafe {
        let tt = n as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&tt, &mut tm);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    };
    let esc = uri.replace('&', "&amp;");
    if let Some(start) = s.find(&format!("href=\"{esc}\"")) {
        if let Some(rel) = s[start..].find("visited=\"") {
            let a = start + rel + "visited=\"".len();
            if let Some(len) = s[a..].find('"') {
                s.replace_range(a..a + len, &t);
            }
        }
    } else if let Some(end) = s.rfind("</xbel>") {
        s.insert_str(
            end,
            &format!("  <bookmark href=\"{esc}\" added=\"{t}\" modified=\"{t}\" visited=\"{t}\">\n  </bookmark>\n"),
        );
    }
    let _ = std::fs::write(&xbel, s);
}

/// Create the app state for `ui` and wire every callback (no windowing-system specifics).
pub(super) fn setup(
    ui: &FinderWindow,
    chooser: Option<Chooser>,
    start: Option<String>,
    show_hidden: bool,
) -> Rc<RefCell<App>> {
    let f = ui.global::<F>();
    f.set_crit_attrs(model(criteria::ATTRS.iter().map(|s| SharedString::from(crate::tr(s))).collect()));
    let mut st = Settings::load();
    if show_hidden {
        st.hidden = true;
    }
    let items = Rc::new(VecModel::<FItem>::default());
    f.set_items(ModelRc::from(items.clone() as Rc<dyn Model<Data = FItem>>));
    f.set_view(st.view);
    f.set_show_preview(st.preview);
    f.set_sort_key(st.sort.0);
    f.set_sort_desc(st.sort.1);
    f.set_side_w(st.side_w);
    if let Some(c) = &chooser {
        f.set_mode(if c.save { 2 } else { 1 });
        f.set_directory_mode(c.directory);
        f.set_accept_label(
            if !c.accept.is_empty() {
                c.accept.clone()
            } else if c.save {
                crate::tr("Save").into()
            } else if c.directory {
                crate::tr("Choose").into()
            } else {
                crate::tr("Open").into()
            }
            .into(),
        );
        f.set_save_name(c.name.clone().into());
        f.set_filters(model(c.filters.iter().map(|x| SharedString::from(x.0.as_str())).collect::<Vec<_>>()));
        let defs: Vec<FTagDef> =
            fs::TAGS.iter().map(|(n, c)| FTagDef { name: (*n).into(), color: rgb(*c), on: false }).collect();
        f.set_tag_defs(model(defs));
        if c.save && f.get_view() == 3 {
            f.set_view(0);
        }
    }
    if st.trash_30 && chooser.is_none() {
        fs::purge_trash(&fs::trash_dir(), 30, fs::now_secs());
    }
    let placeholder = Loc::Dir(fs::home());
    let app = Rc::new(RefCell::new(App::new(ui, st, chooser.clone(), placeholder, items)));
    {
        let mut a = app.borrow_mut();
        a.me = Rc::downgrade(&app);
        let start_loc = match (&chooser, start) {
            (_, Some(s)) => Loc::parse(&s),
            (Some(_), None) => Loc::Dir(a.st.chooser_dir.clone().filter(|d| d.is_dir()).unwrap_or_else(fs::home)),
            (None, None) => a.start_loc(),
        };
        a.loc = start_loc.clone();
        a.tabs[0].loc = start_loc;
        a.places();
        a.fill_view_options();
        a.fill_prefs();
        a.toolbar_apply();
        let l = a.loc.clone();
        a.go(l, false);
    }

    macro_rules! cb {
        ($name:ident, |$a:ident $(, $arg:ident)*| $body:expr) => {{
            let app = app.clone();
            f.$name(move |$($arg),*| {
                let Ok(mut $a) = app.try_borrow_mut() else { return Default::default() };
                $body
            });
        }};
    }
    cb!(on_go, |a, p| a.go(Loc::parse(&p), true));
    cb!(on_place_click, |a, p| a.go(Loc::parse(&p), true));
    cb!(on_crumb_click, |a, p| a.go(Loc::parse(&p), true));
    cb!(on_back, |a| a.back());
    cb!(on_forward, |a| a.forward());
    cb!(on_up, |a| a.up());
    cb!(on_select, |a, i, t, r| a.select(i.max(0) as usize, t, r));
    cb!(on_select_none, |a| a.select_none());
    cb!(on_open, |a, i| {
        let e = a.shown.get(i.max(0) as usize).map(|&x| a.all[x].clone());
        if let Some(e) = e {
            a.open_entry(e)
        }
    });
    cb!(on_open_tab, |a, i| {
        let e = a.shown.get(i.max(0) as usize).map(|&x| a.all[x].clone());
        if let Some(e) = e.filter(|e| e.is_dir) {
            a.new_tab(Loc::Dir(e.path), false)
        }
    });
    cb!(on_label_click, |a, i| {
        if a.sel == [i.max(0) as usize] {
            a.label_click = Some((i.max(0) as usize, std::time::Instant::now()));
        }
    });
    cb!(on_context, |a, i, x, y| a.context(i, x, y));
    cb!(on_place_context, |a, p, x, y| a.place_context(&p, x, y));
    cb!(on_crumb_context, |a, p, x, y| a.crumb_context(&p, x, y));
    cb!(on_header_context, |a, x, y| a.header_context(x, y));
    cb!(on_section_toggle, |a, s| a.toggle_section(&s));
    cb!(on_menu_action, |a, id| a.action(&id));
    cb!(on_menu_sub, |a, id, y| a.menu_sub(&id, y));
    cb!(on_menu_closed, |a| a.close_native_menus());
    cb!(on_set_view, |a, v| a.set_view(v));
    cb!(on_sort_by, |a, k| {
        if a.st.sort.0 == k {
            a.st.sort.1 = !a.st.sort.1
        } else {
            a.st.sort = (k, false)
        }
        a.resort()
    });
    cb!(on_search, |a, q| a.search(&q));
    cb!(on_search_commit, |a, q| a.search_commit(&q));
    cb!(on_set_scope, |a, i| a.set_scope(i));
    cb!(on_crit_add, |a, i| a.crit_add(i));
    cb!(on_crit_remove, |a, i| a.crit_remove(i));
    cb!(on_crit_set, |a, i, k, v| a.crit_set(i, &k, v));
    cb!(on_crit_value, |a, i, v| a.crit_value(i, &v));
    cb!(on_save_search, |a| a.save_search());
    cb!(on_save_search_done, |a| a.save_search_done());
    cb!(on_tb_toggle, |a, id| a.toolbar_toggle(&id));
    cb!(on_tb_reset, |a| a.toolbar_reset());
    {
        let app = app.clone();
        f.on_key(move |t, ctrl, meta, shift, alt, _cols| {
            app.try_borrow_mut().map(|mut a| a.key(&t, ctrl, meta, shift, alt)).unwrap_or(false)
        });
    }
    cb!(on_rename, |a, i, name| a.rename(i.max(0) as usize, &name));
    cb!(on_tag_toggle, |a, t| a.tag_toggle(&t));
    cb!(on_custom_color, |a, i| a.custom_set(Some(i.max(0) as usize), None));
    cb!(on_custom_symbol, |a, s| a.custom_set(None, Some(s.to_string())));
    cb!(on_column_click, |a, ci, i, t, r| a.column_click(ci.max(0) as usize, i.max(0) as usize, t, r));
    cb!(on_column_open, |a, ci, i| {
        let ui_ = a.ui();
        let f = ui_.global::<F>();
        let it = f.get_columns().row_data(ci.max(0) as usize).and_then(|c| c.items.row_data(i.max(0) as usize));
        if let Some(it) = it {
            if let Some(e) = fs::entry(Path::new(it.path.as_str())) {
                a.open_entry(e)
            }
        }
    });
    cb!(on_toolbar_menu, |a, which, x, y| a.toolbar_menu(&which, x, y));
    cb!(on_drag_start, |a, i| {
        if a.ext_drag.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(800)) {
            return;
        }
        let i = i.max(0) as usize;
        if !a.sel.contains(&i) {
            a.select(i, false, false);
        }
        a.label_click = None;
        a.drag_origin = None;
        a.drag = a.sel_paths();
        let ui_ = a.ui();
        let f = ui_.global::<F>();
        let label = if a.drag.len() == 1 {
            name_of(&a.drag[0])
        } else {
            crate::ntr("{n} item", "{n} items", a.drag.len() as i64)
        };
        f.set_drag_label(trunc(&label, 18).into());
        f.set_drag_count(a.drag.len() as i32);
        f.set_dragging(!a.drag.is_empty() && !matches!(a.loc, Loc::Apps));
    });
    cb!(on_drag_move, |a, x, y, copy| a.drag_move(x, y, copy));
    cb!(on_drag_end, |a, x, y, copy| {
        let (_, p, _) = a.hit(x, y);
        let ui_ = a.ui();
        let f = ui_.global::<F>();
        f.set_dragging(false);
        f.set_drop_idx(-1);
        f.set_drop_place("".into());
        f.set_drop_line(-1.0);
        a.spring = None;
        let here = a.loc.dir().is_some_and(|d| d.to_string_lossy() == p.as_str());
        if here && !copy && a.move_icons(x, y) {
            return;
        }
        a.drag_origin = None;
        if !p.is_empty() {
            a.drop_on(&p, copy);
        } else {
            a.drag.clear();
        }
    });
    cb!(on_marquee, |a, phase, x, y, add| a.marquee(phase, x, y, add));
    cb!(on_relayout, |a, v, cx, top, cw, ch| a.relayout(v, cx, top, cw, ch));
    cb!(on_expand, |a, i, all| {
        let i = i.max(0) as usize;
        let open = a.shown.get(i).map(|&k| !a.expanded.contains(&a.all[k].path)).unwrap_or(false);
        a.expand(i, open, all)
    });
    cb!(on_tab_select, |a, i| a.select_tab(i.max(0) as usize));
    cb!(on_tab_close, |a, i| a.close_tab(i.max(0) as usize));
    cb!(on_ql_play, |a| a.ql_play());
    cb!(on_ql_seek, |a, v| a.ql_seek(v));
    cb!(on_ql_page_step, |a, d| a.ql_page(d));
    cb!(on_cs_connect, |a, s| a.connect(&s));
    cb!(on_cs_add, |a, s| a.connect_add(&s));
    cb!(on_cs_remove, |a, i| a.connect_remove(i));
    cb!(on_cs_clear_recent, |a| {
        a.st.recent_servers.clear();
        a.st.save()
    });
    cb!(on_tab_move, |a, i, to| a.move_tab(i.max(0) as usize, to.max(0) as usize));
    cb!(on_tab_detach, |a, i| a.detach_tab(i.max(0) as usize));
    cb!(on_tab_context, |a, i, x, y| a.tab_context(i.max(0) as usize, x, y));
    cb!(on_tab_new, |a| {
        let l = a.loc.clone();
        a.new_tab(l, false)
    });
    cb!(on_col_resize, |a, k, w| {
        a.st.col_w.insert(k, w.clamp(50.0, 600.0));
        a.layout_rows();
        a.st.save();
    });
    cb!(on_set_zoom, |a, z| a.set_zoom(z));
    cb!(on_prog_cancel, |a| a.cancel_transfer());
    cb!(on_ql_close, |a| a.ql_close());
    cb!(on_ql_step, |a, d| a.ql_step(d));
    cb!(on_opt, |a, id, v| a.opt(&id, v));
    cb!(on_rn_changed, |a| a.rn_changed());
    cb!(on_rn_apply, |a| a.rn_apply());
    cb!(on_side_resized, |a, w| {
        a.st.side_w = w;
        a.st.save();
    });
    cb!(on_accept, |a| a.accept());
    f.on_cancel(|| std::process::exit(1));
    cb!(on_filter_changed, |a, i| {
        a.filter = i.max(0) as usize;
        a.sel.clear();
        a.refresh()
    });
    cb!(on_save_edited, |a, _t| a.chrome());
    cb!(on_goto, |a, t| a.goto(&t));
    cb!(on_goto_edited, |a, t| a.goto_edited(&t));
    cb!(on_alert_choose, |a, i| a.alert_choose(i));
    app
}

impl App {
    pub(super) fn drag_move(&mut self, x: f32, y: f32, copy: bool) {
        if self.drag_origin.is_none() {
            self.drag_origin = Some((x, y));
        }
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let w = ui_.window();
        let sz = w.size().to_logical(w.scale_factor());
        if !self.drag.is_empty() && (x < -2.0 || y < -2.0 || x > sz.width + 2.0 || y > sz.height + 2.0) {
            let uris: Vec<String> = self.drag.iter().map(|p| fs::file_uri(p)).collect();
            if crate::aqua_msg(&format!("dragfiles {}", uris.join(" "))) {
                self.ext_drag = Some(std::time::Instant::now());
                self.drag.clear();
                self.drag_origin = None;
                self.spring = None;
                f.set_dragging(false);
                f.set_drop_idx(-1);
                f.set_drop_place("".into());
                f.set_drop_line(-1.0);
                // The compositor now owns the button (system drag-and-drop): this window
                // never sees the release, so end Slint's press here — otherwise coming
                // back and releasing over the window leaves a phantom internal drag.
                release_pointer_later(&ui_);
                return;
            }
        }
        let (i, p, line) = self.hit(x, y);
        f.set_drag_x(x);
        f.set_drag_y(y);
        f.set_drop_idx(i);
        let spring_ok = !p.is_empty()
            && !p.starts_with("insert:")
            && Path::new(&p).is_dir()
            && self.loc.dir() != Some(Path::new(&p));
        if !spring_ok {
            self.spring = None;
        } else if self.spring.as_ref().map(|s| s.0 != p).unwrap_or(true) {
            self.spring = Some((p.clone(), std::time::Instant::now()));
        }
        f.set_drop_place(if p.starts_with("insert:") { SharedString::new() } else { p.into() });
        f.set_drop_line(line);
        f.set_drop_copy(copy);
    }
}

/// Run Finder (`chooser` = None) or the open/save panel.
pub fn run(chooser: Option<Chooser>, start: Option<String>, show_hidden: bool) -> Result<(), slint::PlatformError> {
    let ui = FinderWindow::new()?;
    crate::init_translations();
    crate::set_app_id();
    crate::apply_theme!(ui);
    let f = ui.global::<F>();
    let glass = crate::glass_supported();
    f.set_glass(glass);
    let app = setup(&ui, chooser.clone(), start, show_hidden);
    if chooser.is_none() {
        app.borrow_mut().ipc = ipc::Ipc::bind(&ipc::dir());
    }
    let size = app.borrow().st.size;
    if let Some(c) = &chooser {
        if !c.title.is_empty() {
            ui.set_title_override(c.title.clone().into());
        }
    }

    {
        use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
        let w = ui.as_weak();
        let last_press: Rc<std::cell::Cell<Option<std::time::Instant>>> = Rc::new(std::cell::Cell::new(None));
        f.on_win_drag(move || {
            let Some(u) = w.upgrade() else { return };
            let now = std::time::Instant::now();
            if last_press.get().is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(400)) {
                last_press.set(None);
                release_pointer_later(&u);
                if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                    let win = u.window();
                    win.set_maximized(!win.is_maximized());
                }
                return;
            }
            last_press.set(Some(now));
            u.window().with_winit_window(|win| {
                let _ = win.drag_window();
            });
            release_pointer_later(&u);
        });
        let w = ui.as_weak();
        f.on_win_resize(move |dir| {
            use winit::window::ResizeDirection as R;
            let d = [R::North, R::South, R::West, R::East, R::NorthWest, R::NorthEast, R::SouthWest, R::SouthEast]
                [dir.clamp(0, 7) as usize];
            if let Some(u) = w.upgrade() {
                u.window().with_winit_window(|win| {
                    let _ = win.drag_resize_window(d);
                });
                release_pointer_later(&u);
            }
        });
        let is_chooser = chooser.is_some();
        f.on_win_close(move || {
            if is_chooser {
                std::process::exit(1);
            }
            let _ = slint::quit_event_loop();
        });
        let w = ui.as_weak();
        f.on_win_minimize(move || {
            if let Some(u) = w.upgrade() {
                if u.window().with_winit_window(|w| w.set_minimized(true)).is_none() {
                    u.window().set_minimized(true);
                }
            }
        });
        let w = ui.as_weak();
        f.on_win_zoom(move || {
            if let Some(u) = w.upgrade() {
                if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                    let win = u.window();
                    win.set_maximized(!win.is_maximized());
                }
            }
        });
        let w = ui.as_weak();
        ui.window().on_winit_window_event(move |_, ev| {
            if let winit::event::WindowEvent::Focused(fo) = ev {
                if let Some(u) = w.upgrade() {
                    if *fo && u.window().is_minimized() {
                        u.window().set_minimized(false);
                    }
                    u.global::<F>().set_active(*fo);
                }
            }
            EventResult::Propagate
        });
        if glass {
            crate::enable_glass(&ui.as_weak());
        }
    }

    let timer = slint::Timer::default();
    {
        let app = app.clone();
        let mut tick = 0u32;
        timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(40), move || {
            tick = tick.wrapping_add(1);
            if let Ok(mut a) = app.try_borrow_mut() {
                a.poll();
                if tick.is_multiple_of(30) {
                    a.watch();
                }
                if tick.is_multiple_of(250) {
                    a.places();
                }
            }
        });
    }
    let _scale_watch = crate::watch_scale(ui.as_weak(), {
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_repaint(ui.get_repaint() + 1);
            }
        }
    });
    ui.window().on_close_requested({
        let is_chooser = chooser.is_some();
        move || {
            if is_chooser {
                std::process::exit(1);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    ui.window().set_size(slint::LogicalSize::new(
        if chooser.is_some() { 900.0 } else { size.0 },
        if chooser.is_some() { 560.0 } else { size.1 },
    ));
    let r = ui.run();
    if chooser.is_none() {
        let s = ui.window().size().to_logical(ui.window().scale_factor());
        let mut a = app.borrow_mut();
        a.st.size = (s.width, s.height);
        a.st.save();
    }
    crate::run(r)?;
    if chooser.is_some() {
        std::process::exit(1);
    }
    Ok(())
}

pub(super) fn release_pointer_later(u: &FinderWindow) {
    let w = u.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(u) = w.upgrade() else { return };
        use slint::platform::{PointerEventButton, WindowEvent};
        let outside = slint::LogicalPosition::new(-10000.0, -10000.0);
        u.window().dispatch_event(WindowEvent::PointerReleased { position: outside, button: PointerEventButton::Left });
        u.window().dispatch_event(WindowEvent::PointerExited);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation() {
        assert_eq!(trunc("short", 10), "short");
        assert_eq!(trunc("a long file name.txt", 8), "a long …");
        assert_eq!(trunc("файл", 3), "фа…");
        assert_eq!(trunc("x", 0), "…");
    }
}
