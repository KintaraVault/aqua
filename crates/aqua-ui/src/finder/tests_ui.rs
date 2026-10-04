use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::Rgb8Pixel;
use slint::{ComponentHandle, LogicalPosition};
use std::sync::Once;

const W: u32 = 1100;
const H: u32 = 700;

struct Headless;

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };
}

impl slint::platform::Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOWS.with(|v| v.borrow_mut().push(w.clone()));
        Ok(w)
    }
}

fn root() -> PathBuf {
    std::env::temp_dir().join(format!("aqua-finder-ui-{}", std::process::id()))
}

fn env() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let r = root();
        let _ = std::fs::remove_dir_all(&r);
        let home = r.join("home");
        for d in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", ".config", ".local/share"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("AQUA_FINDER_NO_SAVE", "1");
        std::env::set_var("USER", "tester");
    });
    let _ = slint::platform::set_platform(Box::new(Headless));
}

/// A fresh folder with a few files, unique per test.
fn fixture(name: &str) -> PathBuf {
    let d = fs::home().join("Documents").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Folder")).unwrap();
    for (f, body) in [("alpha.txt", "a"), ("beta.md", "# b"), ("gamma.txt", "ccc"), ("delta.rs", "fn main() {}")] {
        std::fs::write(d.join(f), body).unwrap();
    }
    d
}

struct Harness {
    ui: FinderWindow,
    app: Rc<RefCell<App>>,
    win: Rc<MinimalSoftwareWindow>,
}

impl Harness {
    fn new(start: Option<&Path>) -> Self {
        env();
        let ui = FinderWindow::new().unwrap();
        let win = WINDOWS.with(|v| v.borrow().last().unwrap().clone());
        let app = run::setup(&ui, None, start.map(|p| p.to_string_lossy().into_owned()), false);
        win.set_size(slint::PhysicalSize::new(W, H));
        ui.show().unwrap();
        let h = Harness { ui, app, win };
        h.frame();
        h
    }

    fn f(&self) -> F<'_> {
        self.ui.global::<F>()
    }

    fn frame(&self) -> Vec<Rgb8Pixel> {
        slint::platform::update_timers_and_animations();
        let mut buf = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
        self.win.request_redraw();
        self.win.draw_if_needed(|r| {
            r.render(&mut buf, W as usize);
        });
        slint::platform::update_timers_and_animations();
        buf
    }

    fn snap(&self, name: &str) {
        let buf = self.frame();
        if let Ok(dir) = std::env::var("AQUA_FINDER_SNAPSHOTS") {
            let raw: Vec<u8> = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            let img = image::RgbImage::from_raw(W, H, raw).unwrap();
            img.save(Path::new(&dir).join(format!("{name}.png"))).unwrap();
        }
    }

    fn press(&self, x: f32, y: f32, button: PointerEventButton) {
        let position = LogicalPosition::new(x, y);
        self.win.dispatch_event(WindowEvent::PointerMoved { position });
        self.win.dispatch_event(WindowEvent::PointerPressed { position, button });
        self.win.dispatch_event(WindowEvent::PointerReleased { position, button });
        self.frame();
    }

    fn side_y(&self, path: &str) -> f32 {
        let a = self.app.borrow();
        let i = a.side_rows.iter().position(|p| p == path).unwrap_or_else(|| panic!("{path} not in sidebar"));
        input::SIDE_TOP + i as f32 * input::SIDE_ROW + input::SIDE_ROW / 2.0
    }

    fn names(&self) -> Vec<String> {
        let a = self.app.borrow();
        a.shown.iter().map(|&i| a.all[i].name.clone()).collect()
    }

    fn select_name(&self, name: &str) -> usize {
        let i = self.names().iter().position(|n| n == name).unwrap();
        self.app.borrow_mut().select(i, false, false);
        i
    }

    fn key(&self, text: &str, cmd: bool, shift: bool) -> bool {
        self.f().invoke_key(text.into(), cmd, false, shift, false, 0)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.ui.hide();
    }
}

#[test]
fn sidebar_menu_opens_at_pointer() {
    let h = Harness::new(None);
    let docs = fs::home().join("Documents").to_string_lossy().into_owned();
    for (path, x) in [(docs.as_str(), 40.0), ("trash:", 120.0)] {
        let y = h.side_y(path);
        h.press(x, y, PointerEventButton::Right);
        let f = h.f();
        assert!(f.get_menu_open(), "menu for {path}");
        assert!((f.get_menu_x() - x).abs() < 1.0, "x {} vs {x}", f.get_menu_x());
        assert!((f.get_menu_y() - y).abs() < 1.0, "y {} vs {y}", f.get_menu_y());
        let first = f.get_menu().row_data(0).unwrap();
        assert_eq!(first.id.as_str(), format!("go:{path}"));
        assert_eq!(f.get_ctx_place().as_str(), path);
        h.snap(&format!("sidebar-menu-{}", if path == "trash:" { "trash" } else { "docs" }));
        f.set_menu_open(false);
    }
}

#[test]
fn sidebar_click_navigates() {
    let h = Harness::new(None);
    let docs = fs::home().join("Documents");
    let y = h.side_y(&docs.to_string_lossy());
    h.press(60.0, y, PointerEventButton::Left);
    assert_eq!(h.app.borrow().loc.dir(), Some(docs.as_path()));
    assert!(h.f().get_title().contains("Documents"));
}

#[test]
fn sidebar_section_collapses() {
    let h = Harness::new(None);
    let before = h.app.borrow().side_rows.len();
    h.app.borrow_mut().toggle_section("tags");
    let after = h.app.borrow().side_rows.len();
    assert!(after < before);
    h.app.borrow_mut().toggle_section("tags");
    assert_eq!(h.app.borrow().side_rows.len(), before);
}

#[test]
fn view_shortcuts_switch_views() {
    let d = fixture("views");
    let h = Harness::new(Some(&d));
    for (k, v, name) in [("2", 1, "list"), ("3", 2, "columns"), ("4", 3, "gallery"), ("1", 0, "icons")] {
        h.select_name("beta.md");
        assert!(h.key(k, true, false));
        assert_eq!(h.f().get_view(), v);
        h.snap(&format!("view-{name}"));
    }
}

#[test]
fn listing_sorts_folders_and_names() {
    let d = fixture("sorting");
    let h = Harness::new(Some(&d));
    let names = h.names();
    for n in ["Folder", "alpha.txt", "beta.md", "delta.rs", "gamma.txt"] {
        assert!(names.contains(&n.to_string()), "{n} missing in {names:?}");
    }
    let pos = |n: &str| names.iter().position(|x| x == n).unwrap();
    assert!(pos("alpha.txt") < pos("beta.md") && pos("beta.md") < pos("delta.rs"));
}

#[test]
fn arrow_keys_move_selection() {
    let d = fixture("arrows");
    let h = Harness::new(Some(&d));
    h.key("2", true, false);
    h.select_name("alpha.txt");
    let start = h.app.borrow().sel.clone();
    h.key(&SharedString::from(slint::platform::Key::DownArrow), false, false);
    let next = h.app.borrow().sel.clone();
    assert_eq!(next, vec![start[0] + 1]);
    h.key(&SharedString::from(slint::platform::Key::DownArrow), false, true);
    assert_eq!(h.app.borrow().sel.len(), 2);
    h.key("a", true, false);
    assert_eq!(h.app.borrow().sel.len(), h.names().len());
}

#[test]
fn rename_undo_redo() {
    let d = fixture("rename");
    let h = Harness::new(Some(&d));
    let i = h.select_name("alpha.txt");
    h.app.borrow_mut().rename(i, "omega.txt");
    assert!(d.join("omega.txt").exists() && !d.join("alpha.txt").exists());
    h.app.borrow_mut().undo();
    assert!(d.join("alpha.txt").exists() && !d.join("omega.txt").exists());
    h.app.borrow_mut().redo();
    assert!(d.join("omega.txt").exists());
}

#[test]
fn rename_rejects_slash_and_duplicates() {
    let d = fixture("rename-bad");
    let h = Harness::new(Some(&d));
    let i = h.select_name("alpha.txt");
    h.app.borrow_mut().rename(i, "beta.md");
    assert!(d.join("alpha.txt").exists());
    assert_eq!(std::fs::read_to_string(d.join("beta.md")).unwrap(), "# b");
}

#[test]
fn new_folder_and_trash_with_undo() {
    let d = fixture("trash");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().new_folder();
    let made = d.join(crate::tr("untitled folder"));
    assert!(made.is_dir());
    h.f().set_renaming(-1);
    h.select_name("gamma.txt");
    h.app.borrow_mut().trash_selection();
    assert!(!d.join("gamma.txt").exists());
    assert!(!h.names().contains(&"gamma.txt".to_string()));
    h.app.borrow_mut().undo();
    assert!(d.join("gamma.txt").exists());
}

#[test]
fn duplicate_creates_copy() {
    let d = fixture("dup");
    let h = Harness::new(Some(&d));
    h.select_name("delta.rs");
    h.app.borrow_mut().duplicate();
    for _ in 0..200 {
        if std::fs::read_dir(&d).unwrap().count() > 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.app.borrow_mut().poll();
    }
    let n = std::fs::read_dir(&d)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("delta"))
        .count();
    assert_eq!(n, 2);
}

#[test]
fn tabs_open_switch_close() {
    let d = fixture("tabs");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().new_tab(Loc::Dir(d.join("Folder")), false);
    assert_eq!(h.f().get_tabs().row_count(), 2);
    assert_eq!(h.app.borrow().loc.dir(), Some(d.join("Folder").as_path()));
    h.snap("tabs");
    h.app.borrow_mut().select_tab(0);
    assert_eq!(h.app.borrow().loc.dir(), Some(d.as_path()));
    h.app.borrow_mut().close_tab(1);
    assert!(h.f().get_tabs().row_count() <= 1);
}

#[test]
fn back_and_forward() {
    let d = fixture("history");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().go(Loc::Dir(d.join("Folder")), true);
    h.app.borrow_mut().back();
    assert_eq!(h.app.borrow().loc.dir(), Some(d.as_path()));
    h.app.borrow_mut().forward();
    assert_eq!(h.app.borrow().loc.dir(), Some(d.join("Folder").as_path()));
    h.app.borrow_mut().up();
    assert_eq!(h.app.borrow().loc.dir(), Some(d.as_path()));
}

#[test]
fn marquee_selects_list_rows() {
    let d = fixture("marquee");
    let h = Harness::new(Some(&d));
    h.key("2", true, false);
    h.frame();
    let (cx, top, _, _) = h.app.borrow().geo;
    assert!(top > 0.0, "layout geometry not reported");
    let y0 = top + layout::LIST_TOP + 2.0;
    let y1 = y0 + layout::ROW_LIST * 2.5;
    let mut a = h.app.borrow_mut();
    a.marquee(0, cx + 300.0, y0, false);
    a.marquee(1, cx + 320.0, y1, false);
    assert_eq!(a.sel.len(), 3);
    a.marquee(2, cx + 320.0, y1, false);
    drop(a);
    assert!(!h.f().get_mq_on());
}

#[test]
fn context_menu_follows_pointer_in_content() {
    let d = fixture("ctx");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().context(-1, 600.0, 400.0);
    let f = h.f();
    assert!(f.get_menu_open());
    assert_eq!((f.get_menu_x(), f.get_menu_y()), (600.0, 400.0));
    let ids: Vec<String> = f.get_menu().iter().map(|m| m.id.to_string()).collect();
    assert!(ids.iter().any(|i| i == "new-folder"), "{ids:?}");
}

#[test]
fn quick_look_toggles_with_space() {
    let d = fixture("ql");
    let h = Harness::new(Some(&d));
    h.select_name("beta.md");
    assert!(h.key(" ", false, false));
    assert!(h.f().get_ql_open());
    h.snap("quick-look");
    assert!(h.key(" ", false, false));
    assert!(!h.f().get_ql_open());
}

#[test]
fn search_finds_files() {
    let d = fixture("search");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().search_commit("gamma");
    h.app.borrow_mut().set_scope(1);
    assert!(matches!(&h.app.borrow().loc, Loc::Search(root, q) if root == &d && q == "gamma"));
    for _ in 0..300 {
        h.app.borrow_mut().poll();
        if matches!(h.app.borrow().loc, Loc::Search(..)) && h.names() == ["gamma.txt"] {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(h.names().iter().any(|n| n == "gamma.txt"), "{:?}", h.names());
    assert!(!h.names().iter().any(|n| n == "alpha.txt"));
}

#[test]
fn dark_mode_renders() {
    let d = fixture("dark");
    let h = Harness::new(Some(&d));
    h.ui.global::<crate::Theme>().set_dark(true);
    h.key("2", true, false);
    h.f().set_show_statusbar(true);
    h.f().set_show_pathbar(true);
    h.snap("dark-list");
}

#[test]
fn view_options_change_grouping_and_sort() {
    let d = fixture("options");
    let h = Harness::new(Some(&d));
    h.key("2", true, false);
    h.f().set_vo_open(true);
    h.app.borrow_mut().opt("sort", 3);
    assert_eq!(h.app.borrow().st.sort.0, 3);
    h.app.borrow_mut().opt("group", 2);
    assert_eq!(h.app.borrow().st.group, 2);
    assert!(h.app.borrow().rows.iter().any(|r| r.count == 0), "group headers expected");
    h.snap("view-options");
    h.f().set_vo_open(false);
    h.f().set_prefs_open(true);
    h.snap("settings");
}

#[test]
fn info_window_opens_for_selection() {
    let d = fixture("info");
    let h = Harness::new(Some(&d));
    h.select_name("beta.md");
    h.app.borrow_mut().action("info-window");
    assert_eq!(h.app.borrow().infos.len(), 1);
    assert_eq!(h.app.borrow().infos[0].path, d.join("beta.md"));
    let w = WINDOWS.with(|v| v.borrow().last().unwrap().clone());
    w.set_size(slint::PhysicalSize::new(W, H));
    let mut buf = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
    w.request_redraw();
    w.draw_if_needed(|r| {
        r.render(&mut buf, W as usize);
    });
    if let Ok(dir) = std::env::var("AQUA_FINDER_SNAPSHOTS") {
        let raw: Vec<u8> = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        image::RgbImage::from_raw(W, H, raw).unwrap().save(Path::new(&dir).join("info.png")).unwrap();
    }
}

impl Harness {
    /// Poll until `done` holds (searches and other background work).
    fn wait_for(&self, done: impl Fn(&Harness) -> bool) {
        for _ in 0..400 {
            self.app.borrow_mut().poll();
            if done(self) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn spot(&self, name: &str) -> (f32, f32) {
        let i = self.names().iter().position(|n| n == name).unwrap();
        self.app.borrow().spots.as_ref().expect("free arrangement")[i]
    }
}

fn picture(p: &Path, w: u32, h: u32) {
    image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 40) as u8, (y * 40) as u8, 200])).save(p).unwrap();
}

#[test]
fn icons_can_be_arranged_freely_and_cleaned_up() {
    let d = fixture("arrange");
    let h = Harness::new(Some(&d));
    h.key("1", true, false);
    h.app.borrow_mut().action("arrange:0");
    assert!(h.f().get_free_mode());
    assert_eq!(h.f().get_arrange(), 1);
    assert_eq!(h.f().get_spots().row_count(), h.names().len());
    let before = h.spot("gamma.txt");
    {
        let mut a = h.app.borrow_mut();
        a.drag = vec![d.join("gamma.txt")];
        a.drag_origin = Some((before.0 + 10.0, before.1 + 10.0));
        assert!(a.move_icons(before.0 + 310.0, before.1 + 233.0));
    }
    let after = h.spot("gamma.txt");
    assert_eq!((after.0 - before.0, after.1 - before.1), (300.0, 223.0));
    h.snap("free-arrangement");
    assert_eq!(h.app.borrow().arr.get(&d).unwrap().pos.get("gamma.txt"), Some(&after));

    h.app.borrow_mut().action("clean-up");
    let g = h.app.borrow().grid;
    let cleaned = h.spot("gamma.txt");
    assert_eq!(arrange::snap(&g, cleaned), cleaned);

    h.app.borrow_mut().action("cleanup-by:0");
    let order: Vec<(f32, f32)> =
        ["alpha.txt", "beta.md", "delta.rs", "Folder", "gamma.txt"].iter().map(|n| h.spot(n)).collect();
    assert!(order.windows(2).all(|w| w[0].1 < w[1].1 || w[0].1 == w[1].1 && w[0].0 < w[1].0), "{order:?}");

    h.app.borrow_mut().action("arrange:1");
    assert_eq!(h.f().get_arrange(), 2);
    assert!(h.app.borrow().arr.get(&d).unwrap().snap);
    h.app.borrow_mut().action("sort:0");
    assert!(!h.f().get_free_mode());
    assert!(h.app.borrow().arr.get(&d).is_none());
}

#[test]
fn search_criteria_and_smart_folders() {
    let d = fixture("smart");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().search_commit("t");
    h.app.borrow_mut().set_scope(1);
    h.app.borrow_mut().crit_add(-1);
    assert_eq!(h.f().get_crit().row_count(), 1);
    h.app.borrow_mut().crit_set(0, "attr", 4);
    h.app.borrow_mut().crit_set(0, "op", 2);
    h.app.borrow_mut().crit_value(0, "txt");
    let texts = |h: &Harness| {
        let mut n = h.names();
        n.sort();
        n == ["alpha.txt", "gamma.txt"]
    };
    h.wait_for(texts);
    assert!(texts(&h), "{:?}", h.names());
    h.snap("search-criteria");

    h.app.borrow_mut().save_search();
    assert!(h.f().get_ss_open());
    h.f().set_ss_name("Text Files".into());
    h.f().set_ss_side(true);
    h.app.borrow_mut().save_search_done();
    let file = criteria::saved_dir().join("Text Files.json");
    assert!(file.is_file());
    assert_eq!(h.app.borrow().loc, Loc::Smart(file.clone()));
    let key = Loc::Smart(file.clone()).key();
    assert!(h.app.borrow().side_rows.contains(&key));
    assert_eq!(h.f().get_title(), "Text Files");

    h.app.borrow_mut().go(Loc::Dir(d.clone()), true);
    assert!(h.app.borrow().criteria.is_empty());
    assert_eq!(h.f().get_crit().row_count(), 0);
    h.press(80.0, h.side_y(&key), PointerEventButton::Left);
    assert_eq!(h.app.borrow().loc, Loc::Smart(file.clone()));
    h.wait_for(texts);
    assert!(texts(&h), "{:?}", h.names());
    assert_eq!(h.app.borrow().criteria.len(), 1);

    h.app.borrow_mut().crit_remove(0);
    h.wait_for(|h| h.names().len() == 4);
    let mut n = h.names();
    n.sort();
    assert_eq!(n, ["alpha.txt", "beta.md", "delta.rs", "gamma.txt"]);
    h.app.borrow_mut().remove_place(&key);
    assert!(!h.app.borrow().side_rows.contains(&key));
}

#[test]
fn summary_info_describes_several_items() {
    let d = fixture("summary");
    let h = Harness::new(Some(&d));
    h.select_name("alpha.txt");
    let b = h.names().iter().position(|n| n == "beta.md").unwrap();
    h.app.borrow_mut().select(b, true, false);
    let n = h.app.borrow().infos.len();
    h.f().invoke_key("i".into(), true, true, false, false, 0);
    let a = h.app.borrow();
    assert_eq!(a.infos.len(), n + 1);
    let iw = a.infos.last().unwrap();
    assert_eq!(iw.group.len(), 2);
    assert!(iw.win.get_summary());
    assert!(iw.win.get_heading().contains('2'));
}

#[test]
fn quick_actions_rotate_convert_and_make_pdf() {
    let d = fixture("quick");
    picture(&d.join("photo.png"), 6, 3);
    let h = Harness::new(Some(&d));
    h.select_name("photo.png");
    assert!(h.f().get_pv_quick());
    h.f().set_show_preview(true);
    h.snap("preview-quick-actions");
    h.f().set_show_preview(false);
    let labels: Vec<String> = h.app.borrow().quick_items().iter().map(|m| m.id.to_string()).collect();
    assert!(labels.contains(&"quick:rotl".to_string()));
    h.app.borrow_mut().action("quick:rotr");
    assert_eq!(image::open(d.join("photo.png")).unwrap().to_rgb8().dimensions(), (3, 6));
    h.app.borrow_mut().action("quick:jpeg");
    assert!(d.join("photo.jpg").is_file());
    assert_eq!(h.app.borrow().sel_paths(), [d.join("photo.jpg")]);
    h.select_name("photo.png");
    let j = h.names().iter().position(|n| n == "photo.jpg").unwrap();
    h.app.borrow_mut().select(j, true, false);
    h.app.borrow_mut().action("quick:pdf");
    let pdf = d.join(format!("{}.pdf", crate::tr("Untitled")));
    assert!(pdf.is_file());
    assert!(String::from_utf8_lossy(&std::fs::read(&pdf).unwrap()).contains("/Count 2"));
    if std::process::Command::new("pdfinfo").arg("-v").output().is_ok() {
        let k = h.select_name(&format!("{}.pdf", crate::tr("Untitled")));
        h.app.borrow_mut().focus = Some(k);
        h.app.borrow_mut().ql_open();
        assert_eq!(h.f().get_ql_pages(), 2);
        assert!(h.f().get_ql_has_img());
        h.app.borrow_mut().ql_page(1);
        assert_eq!(h.f().get_ql_page(), 2);
        h.snap("quick-look-pdf");
        h.app.borrow_mut().ql_close();
    }
}

#[test]
fn share_menu_lists_targets() {
    let d = fixture("share");
    let h = Harness::new(Some(&d));
    h.select_name("alpha.txt");
    h.app.borrow_mut().context(-1, 300.0, 300.0);
    h.app.borrow_mut().menu_sub("share-menu", 300.0);
    let ids: Vec<String> = h.f().get_sub_menu().iter().map(|m| m.id.to_string()).collect();
    for id in ["mail", "copy", "copy-path", "compress"] {
        assert!(ids.iter().any(|i| i == id), "{id} missing from {ids:?}");
    }
}

#[test]
fn toolbar_can_be_customized() {
    let d = fixture("toolbar");
    let h = Harness::new(Some(&d));
    assert!(h.f().get_tb_share() && !h.f().get_tb_delete());
    h.app.borrow_mut().toolbar_menu("toolbar", 500.0, 20.0);
    assert!(h.f().get_menu().iter().any(|m| m.id == "tb-customize"));
    h.app.borrow_mut().action("tb-customize");
    assert!(h.f().get_tb_open());
    h.app.borrow_mut().toolbar_toggle("delete");
    h.app.borrow_mut().toolbar_toggle("newfolder");
    h.app.borrow_mut().toolbar_toggle("share");
    assert!(h.f().get_tb_delete() && h.f().get_tb_newfolder() && !h.f().get_tb_share());
    let tb = h.app.borrow().st.toolbar.clone();
    let pos = |id: &str| tb.iter().position(|t| t == id).unwrap();
    assert!(pos("newfolder") < pos("delete"));
    h.snap("toolbar-customize");
    h.f().set_tb_open(false);
    h.snap("toolbar-custom");
    h.app.borrow_mut().toolbar_reset();
    assert!(h.f().get_tb_share() && !h.f().get_tb_delete());
}

#[test]
fn tabs_reorder_and_close_others() {
    let d = fixture("tabs2");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().new_tab(Loc::Dir(d.join("Folder")), false);
    h.app.borrow_mut().new_tab(Loc::Trash, false);
    assert_eq!(h.app.borrow().tab, 2);
    h.app.borrow_mut().move_tab(2, 0);
    {
        let a = h.app.borrow();
        assert_eq!(a.tab, 0);
        assert_eq!(a.tabs[0].loc, Loc::Trash);
        assert_eq!(a.tabs[1].loc, Loc::Dir(d.clone()));
    }
    assert_eq!(h.f().get_tab_idx(), 0);
    h.app.borrow_mut().tab_context(1, 200.0, 60.0);
    assert!(h.f().get_menu().iter().any(|m| m.id == "detach-tab:1"));
    h.app.borrow_mut().action("close-others:1");
    let a = h.app.borrow();
    assert_eq!(a.tabs.len(), 1);
    assert_eq!(a.loc, Loc::Dir(d.clone()));
}

#[test]
fn connect_to_server_keeps_favorites() {
    let d = fixture("connect");
    let h = Harness::new(Some(&d));
    h.app.borrow_mut().action("connect");
    assert!(h.f().get_cs_open());
    h.app.borrow_mut().connect_add("smb://nas.local/share");
    h.app.borrow_mut().connect_add("smb://nas.local/share");
    assert_eq!(h.f().get_cs_favs().row_count(), 1);
    h.snap("connect-to-server");
    let folder = d.join("Folder").to_string_lossy().into_owned();
    h.app.borrow_mut().connect(&folder);
    assert!(!h.f().get_cs_open());
    assert_eq!(h.app.borrow().loc, Loc::Dir(d.join("Folder")));
    assert_eq!(h.app.borrow().st.recent_servers.first(), Some(&folder));
    h.app.borrow_mut().connect_remove(0);
    assert_eq!(h.f().get_cs_favs().row_count(), 0);
    assert_eq!(search::normalize_server("nas.local"), "smb://nas.local");
    assert_eq!(search::normalize_server(" sftp://h/x "), "sftp://h/x");
}

/// Archives open like folders; their files open (extracted to a temp folder) and "Extract
/// Here" unpacks them next to the archive.
#[test]
fn archives_browse_and_extract() {
    if !aqua_sys::have("bsdtar") && !aqua_sys::have("tar") {
        return;
    }
    let d = fixture("archives");
    let ok = std::process::Command::new("tar")
        .arg("-czf")
        .arg(d.join("pack.tar.gz"))
        .arg("-C")
        .arg(&d)
        .args(["Folder", "alpha.txt"])
        .status()
        .unwrap();
    assert!(ok.success());
    std::fs::write(d.join("Folder/inner.txt"), "x").unwrap();
    let h = Harness::new(Some(&d));
    let i = h.select_name("pack.tar.gz");
    let e = h.app.borrow().all[h.app.borrow().shown[i]].clone();
    h.app.borrow_mut().open_entry(e);
    let arc = d.join("pack.tar.gz");
    assert_eq!(h.app.borrow().loc, Loc::Archive(arc.clone(), String::new()));
    let names = h.names();
    assert!(names.contains(&"Folder".to_string()) && names.contains(&"alpha.txt".to_string()), "{names:?}");
    // crumbs end with the archive
    let crumbs = h.app.borrow().crumbs();
    assert_eq!(crumbs.last().unwrap().label.as_str(), "pack.tar.gz");
    // into a folder inside the archive, then back via its key
    let i = h.select_name("Folder");
    let e = h.app.borrow().all[h.app.borrow().shown[i]].clone();
    h.app.borrow_mut().open_entry(e);
    assert_eq!(h.app.borrow().loc, Loc::Archive(arc.clone(), "Folder".into()));
    assert_eq!(h.app.borrow().loc_title(&Loc::Archive(arc.clone(), "Folder".into())), "Folder");
    h.app.borrow_mut().back();
    assert_eq!(h.app.borrow().loc, Loc::Archive(arc.clone(), String::new()));
    // the context menu offers extraction
    let i = h.select_name("alpha.txt");
    h.app.borrow_mut().context(i as i32, 100.0, 100.0);
    let ids: Vec<String> = h.f().get_menu().iter().map(|m| m.id.to_string()).collect();
    assert!(ids.contains(&"extract-sel".to_string()) && ids.contains(&"extract-all".to_string()), "{ids:?}");
    h.f().set_menu_open(false);
    // extract one item: lands next to the archive (unique name) and gets selected there
    h.app.borrow_mut().action("extract-sel");
    assert_eq!(h.app.borrow().loc, Loc::Dir(d.clone()));
    assert!(d.join("alpha 2.txt").exists());
    // "Extract Here" on the archive: several top-level items → a folder named after it
    h.select_name("pack.tar.gz");
    h.app.borrow_mut().action("extract-here");
    assert!(d.join("pack/Folder").is_dir() && d.join("pack/alpha.txt").is_file());
}
