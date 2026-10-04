use crate::app::{self, with, App};
use crate::conv::Route;
use crate::images::{self, Loader, Req};
use aqua_store::jobs::Jobs;
use aqua_store::store::Store;
use aqua_ui::{StoreWindow, AS};
use slint::ComponentHandle;
use std::sync::Arc;

fn release_pointer_later(u: &StoreWindow) {
    let w = u.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(u) = w.upgrade() else { return };
        use slint::platform::{PointerEventButton, WindowEvent};
        let outside = slint::LogicalPosition::new(-10000.0, -10000.0);
        u.window().dispatch_event(WindowEvent::PointerReleased { position: outside, button: PointerEventButton::Left });
        u.window().dispatch_event(WindowEvent::PointerExited);
    });
}

fn chrome(ui: &StoreWindow) {
    use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
    let g = ui.global::<AS>();
    let w = ui.as_weak();
    let last = std::rc::Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
    g.on_win_drag(move || {
        let Some(u) = w.upgrade() else { return };
        let now = std::time::Instant::now();
        if last.get().is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(400)) {
            last.set(None);
            release_pointer_later(&u);
            if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                u.window().set_maximized(!u.window().is_maximized());
            }
            return;
        }
        last.set(Some(now));
        u.window().with_winit_window(|win| {
            let _ = win.drag_window();
        });
        release_pointer_later(&u);
    });
    let w = ui.as_weak();
    g.on_win_resize(move |dir| {
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
    g.on_win_close(|| {
        let _ = slint::quit_event_loop();
    });
    let w = ui.as_weak();
    g.on_win_minimize(move || {
        if let Some(u) = w.upgrade() {
            if u.window().with_winit_window(|w| w.set_minimized(true)).is_none() {
                u.window().set_minimized(true);
            }
        }
    });
    let w = ui.as_weak();
    g.on_win_zoom(move || {
        if let Some(u) = w.upgrade() {
            if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                u.window().set_maximized(!u.window().is_maximized());
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
                u.global::<AS>().set_active(*fo);
            }
        }
        EventResult::Propagate
    });
}

pub fn wire(ui: &StoreWindow) {
    let g = ui.global::<AS>();
    g.on_navigate(|i| with(move |a| a.navigate(i.max(0) as usize)));
    g.on_open_feed(|id, title| {
        let (id, title) = (id.to_string(), title.to_string());
        with(move |a| {
            if let Some(c) = id.strip_prefix("category:") {
                a.open(Route::Category(c.to_string()));
            } else if !id.is_empty() {
                a.open(Route::Feed { id, title });
            }
        })
    });
    g.on_open_app(|k| {
        let k = k.to_string();
        with(move |a| a.open(Route::App(k)))
    });
    g.on_open_category(|c| {
        let c = c.to_string();
        with(move |a| a.open(Route::Category(c)))
    });
    g.on_open_developer(|d| {
        let d = d.to_string();
        with(move |a| a.open(Route::Developer(d)))
    });
    g.on_back(|| with(|a| a.back()));
    g.on_forward(|| with(|a| a.forward()));
    g.on_search(|q| {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        with(move |a| {
            a.suggest_gen += 1;
            a.g(|g| g.set_suggestions(Default::default()));
            if matches!(a.route, Route::Search(_)) {
                a.show(Route::Search(q), false);
            } else {
                a.open(Route::Search(q));
            }
        })
    });
    g.on_search_edited(|t| {
        let t = t.to_string();
        with(move |a| a.suggest(t))
    });
    g.on_load_more(|| with(|a| a.load_more()));
    g.on_retry(|| with(|a| a.reload()));
    g.on_refresh(|| {
        with(|a| {
            a.store.http.expire_json();
            a.home_cache.clear();
            if a.route == Route::Updates {
                a.load_updates(true);
            } else {
                a.reload();
            }
        })
    });
    g.on_primary(|k| {
        let k = k.to_string();
        with(move |a| a.primary(&k))
    });
    g.on_cancel(|k| {
        let k = k.to_string();
        with(move |a| a.cancel(&k))
    });
    g.on_app_menu(|k, x, y| {
        let k = k.to_string();
        with(move |a| a.app_menu(&k, x, y, false))
    });
    g.on_page_menu(|x, y| {
        with(move |a| {
            if let Route::App(k) = a.route.clone() {
                a.app_menu(&k, x, y, true);
            }
        })
    });
    g.on_share_menu(|x, y| with(move |a| a.share_menu(x, y)));
    g.on_menu_action(|id| {
        let id = id.to_string();
        with(move |a| a.menu_action(&id))
    });
    g.on_update_all(|| with(|a| a.update_all()));
    g.on_check_updates(|| with(|a| a.load_updates(true)));
    g.on_source_changed(|i| with(move |a| a.source_changed(i)));
    g.on_open_url(|u| crate::actions::open_url(&u));
    g.on_copy_link(|| {
        with(|a| {
            if let Route::App(k) = a.route.clone() {
                if let Some(p) = a.pkg(&k) {
                    a.copy(&crate::conv::web_url(&p));
                }
            }
        })
    });
    g.on_write_review(|| with(|a| a.write_review()));
    g.on_submit_review(|| with(|a| a.submit_review()));
    g.on_vote(|id, kind| {
        let kind = kind.to_string();
        with(move |a| a.vote(id, &kind))
    });
    g.on_all_reviews(|| with(|a| a.menu_action("reviews")));
    g.on_open_versions(|| with(|a| a.g(|g| g.set_sheet("versions".into())).unwrap_or(())));
    g.on_open_shot(|i| {
        with(move |a| {
            a.g(|g| {
                g.set_viewer(i);
                g.set_sheet("viewer".into());
            });
            let gen = a.gen;
            if let Some(s) = a.shots.get(i.max(0) as usize) {
                if !s.full.is_empty() && s.full != s.thumb {
                    a.loader.push_front(Req::Shot { gen, index: i as usize, url: s.full.clone(), max_h: 1400 });
                }
            }
        })
    });
    g.on_manage_perms(|| {
        with(|a| {
            if let Route::App(k) = a.route.clone() {
                a.open_perms(&k);
            }
        })
    });
    g.on_perm_toggled(|id, on| {
        let id = id.to_string();
        with(move |a| a.perm_toggled(&id, on))
    });
    g.on_perm_reset(|| with(|a| a.perm_reset()));
    g.on_setting_toggled(|id, v| {
        let id = id.to_string();
        with(move |a| a.setting(&id, v))
    });
    g.on_scope_changed(|i| with(move |a| a.scope_changed(i)));
    g.on_preferred_changed(|i| with(move |a| a.preferred_changed(i)));
    g.on_reviewer_edited(|t| {
        let t = t.to_string();
        with(move |a| a.reviewer(&t))
    });
    g.on_setup_install(|id| {
        let id = id.to_string();
        with(move |a| a.setup_install(&id))
    });
    g.on_setup_all(|| with(|a| a.setup_all()));
    g.on_setup_dismiss(|| with(|a| a.setup_dismiss()));
    g.on_open_setup(|| with(|a| a.open_setup()));
    g.on_need_accept(|| with(|a| a.need_accept()));
    g.on_confirm_accept(|| with(|a| a.confirm_accept()));
    g.on_close_sheet(|| {
        with(|a| {
            let sheet = a.g(|g| g.get_sheet().to_string()).unwrap_or_default();
            if sheet == "need" {
                a.pending = None;
                a.need = None;
            }
            if sheet == "confirm" {
                a.confirm = None;
            }
            if sheet == "settings" {
                a.reload();
            }
            a.g(|g| g.set_sheet("".into()));
        })
    });
    g.on_filter_changed(|_| with(|a| a.apply_installed()));
    g.on_query_edited(|_| with(|a| a.apply_installed()));
    g.on_open_account(|| {
        with(|a| {
            a.stack.clear();
            a.show(Route::Account, false);
        })
    });
    g.on_open_settings(|| {
        with(|a| {
            a.apply_env();
            a.g(|g| g.set_sheet("settings".into()));
        })
    });
    g.on_open_history(|| with(|a| a.open_history()));
    g.on_history_edited(|q| {
        let q = q.to_string();
        with(move |a| a.apply_history(&q))
    });
    g.on_maintenance(|w| {
        let w = w.to_string();
        with(move |a| a.maintenance(&w))
    });
}

pub fn run(start: Route) -> Result<(), slint::PlatformError> {
    let ui = StoreWindow::new()?;
    aqua_ui::init_translations();
    aqua_ui::set_app_id();
    aqua_ui::apply_theme!(ui);
    let glass = aqua_ui::glass_supported();
    ui.global::<AS>().set_glass(glass);
    chrome(&ui);
    wire(&ui);

    let store = Arc::new(Store::system());
    let jobs = Jobs::spawn(store.run.clone(), |ev| {
        let _ = slint::invoke_from_event_loop(move || with(move |a| a.on_job(ev)));
    });
    let loader = Loader::spawn(store.http.clone(), 4, |req, px| {
        let Some(px) = px else { return };
        let _ = slint::invoke_from_event_loop(move || {
            let img = images::to_image(&px);
            with(move |a| match req {
                Req::Icon { key, .. } => a.icon_ready(key, img),
                Req::Art { key, .. } => a.art_ready(&key, img),
                Req::Shot { gen, index, .. } => a.shot_ready(gen, index, img),
                Req::File { tag, .. } => {
                    if tag == "avatar" {
                        a.g(|g| {
                            g.set_avatar(img);
                            g.set_has_avatar(true);
                        });
                    }
                }
            })
        });
    });
    let app = App::new(&ui, store.clone(), jobs, loader);
    let _rc = app::install(app);

    let (_, full) = aqua_ui::user_names();
    ui.global::<AS>().set_user_name(full.into());
    with(move |a| {
        if let Some(p) = aqua_ui::sysdata::avatar() {
            a.loader.push_front(Req::File { tag: "avatar".into(), path: p, size: 160 });
        }
        a.apply_env();
        a.show(start, false);
        a.load_setup(true);
        a.load_updates(false);
    });
    app::bg(
        &store,
        |s| {
            s.load_catalog();
            s.index();
        },
        |a, ()| {
            a.remark();
            if let Route::Home(_) | Route::Categories = a.route {
                if a.g(|g| g.get_error() != "").unwrap_or(false) {
                    a.reload();
                }
            }
        },
    );

    let ticker = slint::Timer::default();
    ticker.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(3600), || {
        with(|a| {
            if a.store.prefs().auto_check && a.jobs.pending() == 0 {
                a.load_updates(true);
            }
        })
    });
    let _scale = aqua_ui::watch_scale(ui.as_weak(), {
        let w = ui.as_weak();
        move || {
            if let Some(u) = w.upgrade() {
                u.set_repaint(u.get_repaint() + 1);
            }
        }
    });
    if glass {
        aqua_ui::enable_glass(&ui.as_weak());
    }
    if store.prefs().auto_check {
        crate::notify::sync_autostart(true);
    }
    ui.window().set_size(slint::LogicalSize::new(1180.0, 760.0));
    let r = ui.run();
    aqua_ui::run(r)
}
