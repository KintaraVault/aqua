//! Session lifecycle: startup, periodic housekeeping and configuration reload.
use crate::state::Aqua;
use smithay::reexports::calloop::EventLoop;
use std::time::Duration;

/// Housekeeping twice a second: idle/lock/sleep, config hot-reload, clipboard
/// history, system events from logind, alert countdowns.
pub fn schedule_system_tick(event_loop: &mut EventLoop<'static, Aqua>) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(500)), |_, _, st| {
            st.release_memory();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| st.system_tick()));
            if r.is_err() {
                tracing::error!("panic in system tick (recovered)");
            }
            TimeoutAction::ToDuration(Duration::from_millis(500))
        })
        .ok();
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(50)), |_, _, st| {
            if st.poll_clipboard() | st.poll_lock() | st.poll_portal() | st.poll_screenshot() {
                st.needs_redraw = true;
            }
            TimeoutAction::ToDuration(Duration::from_millis(50))
        })
        .ok();
}

impl Aqua {
    /// Startup: XWayland, environment for apps / portals / D-Bus activation, autostart.
    pub fn export_session_env(&mut self) {
        let (r, g, b) = self.cfg.accent_rgb();
        self.appearance.set_accent((r as f64, g as f64, b as f64));
        crate::system::env::export_session(&self.socket_name.to_string_lossy());
        if let Some(n) = self.xdisplay {
            crate::system::env::export_env("DISPLAY", &format!(":{n}"));
        }
    }

    /// Everything a session needs once the Wayland socket exists, for every backend:
    /// XWayland (and with it `DISPLAY`), the exported environment, the tray, the polkit
    /// agent and autostart entries.
    pub fn start_session(&mut self, commands: &[String]) {
        self.report_config_issues(aqua_config::Config::load_checked().1);
        self.start_xwayland();
        self.export_session_env();
        aqua_apps::preload_shell_env();
        aqua_tray::start();
        if aqua_config::Config::load().global_menu {
            aqua_tray::appmenu::start();
        }
        self.after_layout_change();
        if self.cfg.stage_manager {
            self.set_stage_manager(true);
        }
        if self.lock.is_locked() {
            self.on_locked();
        }
        for c in commands {
            aqua_apps::launch(c);
        }
        if std::env::var_os("AQUA_NO_POLKIT").is_none() {
            spawn_polkit_agent();
        }
        if std::env::var_os("AQUA_NO_AUTOSTART").is_none() {
            for c in self.cfg.autostart.clone() {
                aqua_apps::launch(&c);
            }
            for e in aqua_config::autostart::entries() {
                if e.enabled && !self.cfg.autostart.iter().any(|c| c.trim() == e.exec.trim()) {
                    tracing::info!("autostart {} ({})", e.id, e.exec);
                    aqua_apps::launch(&e.exec);
                }
            }
        }
    }

    pub fn system_tick(&mut self) {
        self.tick_idle();
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            static LAST: AtomicU64 = AtomicU64::new(0);
            let t =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let k = if self.cfg.clock_seconds { t } else { t / 60 };
            if LAST.swap(k, Ordering::Relaxed) != k {
                self.needs_redraw = true;
            }
        }
        self.publish_outputs();
        let evs: Vec<crate::system::logind::SysEvent> =
            self.sys_rx.as_ref().map(|r| r.try_iter().collect()).unwrap_or_default();
        for e in evs {
            use crate::system::logind::SysEvent::*;
            match e {
                PrepareSleep => {
                    if self.cfg.idle.lock_on_sleep && !self.lock.is_locked() {
                        self.lock_session();
                    }
                }
                Resumed => {
                    self.notify_activity();
                    aqua_sys::invalidate();
                    self.needs_redraw = true;
                }
                Lock => self.lock_session(),
                Unlock => {
                    if self.lock.is_locked() {
                        self.lock.mode = crate::system::lock::Mode::Unlocked;
                        self.shell.locked = false;
                        self.on_unlocked();
                    }
                }
            }
        }
        let mt = aqua_config::Config::mtime();
        if mt != self.cfg_watch {
            self.cfg_watch = mt;
            self.reload_config();
        }
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static T: AtomicU32 = AtomicU32::new(0);
            if T.fetch_add(1, Ordering::Relaxed) % 60 == 59 && self.cfg.appearance == "auto" {
                let d = aqua_config::auto_dark_now();
                if d != self.cfg.dark {
                    self.cfg.dark = d;
                    crate::system::theming::set_dark(d);
                    self.appearance.set_dark(d);
                    self.shell.set_dark(d);
                    self.dark_anim = Some((std::time::Instant::now(), d));
                    self.needs_redraw = true;
                }
            }
        }
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            if N.fetch_add(1, Ordering::Relaxed) % 4 == 3 && self.shell.rescan_apps_if_changed() {
                tracing::info!("application list changed: {} apps", self.shell.apps.len());
                self.needs_redraw = true;
            }
        }
        let acts = self.shell.poll_actions();
        if !acts.is_empty() {
            self.handle_actions(acts);
        }
        self.sync_shell_windows();
    }

    /// Show a notification when the config file has problems (once per distinct set).
    pub fn report_config_issues(&mut self, issues: Vec<aqua_config::schema::Issue>) {
        let text: Vec<String> = issues.iter().map(|i| i.to_string()).collect();
        if text == self.config_issues {
            return;
        }
        for t in &text {
            tracing::warn!("config: {t}");
        }
        self.config_issues = text.clone();
        if text.is_empty() {
            return;
        }
        let more =
            if text.len() > 2 { format!("\n… {} more: aqua check-config", text.len() - 2) } else { String::new() };
        self.shell.notify(aqua_shell::notifications::Note {
            app_id: "aqua-settings".into(),
            app_name: "Aqua".into(),
            summary: aqua_i18n::tr("Problems in config.toml").into(),
            body: format!("{}{more}", text.iter().take(2).cloned().collect::<Vec<_>>().join("\n")),
            timeout: 10.0,
            ..Default::default()
        });
    }

    pub fn reload_config(&mut self) {
        let (new, issues) = match aqua_config::Config::reload_checked() {
            Ok(r) => r,
            Err(issue) => {
                if let Some(i) = issue {
                    self.report_config_issues(vec![i]);
                }
                return;
            }
        };
        self.report_config_issues(issues);
        if format!("{:?}", new) == format!("{:?}", self.cfg) {
            return;
        }
        tracing::info!("configuration changed: reloading");
        let old = std::mem::replace(&mut self.cfg, new.clone());
        crate::system::theming::apply(&new);
        self.appearance.set_accent({
            let (r, g, b) = new.accent_rgb();
            (r as f64, g as f64, b as f64)
        });
        if old.dark != new.dark {
            crate::system::theming::set_dark(new.dark);
            self.appearance.set_dark(new.dark);
            self.dark_anim = Some((std::time::Instant::now(), new.dark));
        }
        self.shell.cfg = new.clone();
        self.shell.apply_config();
        crate::state::set_reduce_motion(new.reduce_motion);
        self.set_spaces_per_output(new.spaces_per_output);
        aqua_render::set_blur_max_fps(new.blur_max_fps);
        if old.stage_manager != new.stage_manager {
            self.set_stage_manager(new.stage_manager);
        }
        if old.do_not_disturb != new.do_not_disturb {
            self.shell.notes.dnd = new.do_not_disturb;
            self.shell.control.focus = new.do_not_disturb;
        }
        if old.cursor_size != new.cursor_size {
            self.render_cache.cursor.clear();
        }
        self.idle.cfg = new.idle.clone();
        self.apply_input_config();
        if format!("{:?}", old.outputs) != format!("{:?}", new.outputs) {
            self.apply_output_config();
        }
        if old.wallpaper != new.wallpaper
            || old.dock_icon_size != new.dock_icon_size
            || old.menubar_height != new.menubar_height
        {
            self.on_output_resized();
        }
        self.needs_redraw = true;
    }

    /// Publish the live output list to `$XDG_RUNTIME_DIR/aqua-outputs` so System Settings
    /// (a separate client) can show real displays: `name<TAB>WxH<TAB>scale<TAB>make model`.
    pub fn publish_outputs(&mut self) {
        use std::fmt::Write;
        let mut o = String::new();
        for out in self.outputs.list.clone() {
            let g = self.space.output_geometry(&out).unwrap_or_default();
            let mode = out
                .current_mode()
                .map(|m| format!("{}x{}", m.size.w, m.size.h))
                .unwrap_or_else(|| format!("{}x{}", g.size.w, g.size.h));
            let p = out.physical_properties();
            let hz =
                out.current_mode().map(|m| crate::backend::udev::fmt_hz(m.refresh as f64 / 1000.0)).unwrap_or_default();
            let mut modes = self.output_modes(&out);
            if modes.is_empty() {
                if let Some(m) = out.current_mode() {
                    modes.push(format!(
                        "{}x{}@{}",
                        m.size.w,
                        m.size.h,
                        crate::backend::udev::fmt_hz(m.refresh as f64 / 1000.0)
                    ));
                }
            }
            let modes = modes.join(",");
            let label = if p.make == "Aqua" { p.model.clone() } else { format!("{} {}", p.make, p.model) };
            let _ = writeln!(
                o,
                "{}\t{}\t{}\t{}\t{}\t{}",
                out.name(),
                mode,
                out.current_scale().fractional_scale(),
                label,
                hz,
                modes
            );
        }
        static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
        let mut last = LAST.lock().unwrap();
        let path = aqua_config::paths::runtime_dir().join("aqua-outputs");
        // A nested (winit) session must not overwrite the real session's display list:
        // System Settings would then save a phantom "winit" display as primary.
        let nested = !self.outputs.list.is_empty() && self.outputs.list.iter().all(|o| o.name() == "winit");
        if nested && path.exists() && !last.starts_with("winit\t") {
            return;
        }
        if *last != o || !path.exists() {
            if let Err(e) = std::fs::write(&path, &o) {
                tracing::warn!("cannot publish outputs to {}: {e}", path.display());
            }
            *last = o;
        }
    }

    pub fn on_output_resized(&mut self) {
        let (w, h) = self.output_size();
        let pw = (w as f64 * self.scale).round() as u32;
        let ph = (h as f64 * self.scale).round() as u32;
        let wp = aqua_wallpaper::wallpaper(self.cfg.wallpaper.as_deref(), pw, ph);
        self.shell.resize(w as f32, h as f32, self.scale as f32, &wp);
        self.wallpaper = std::sync::Arc::new(wp);
        self.wallpaper_serial += 1;
        self.needs_redraw = true;
    }
}

/// Keep a polkit authentication agent running: without one, pkexec and every app that asks for
/// administrator rights fails.
fn spawn_polkit_agent() {
    let bin = crate::system::env::own_bin("aqua-polkit-agent");
    if !bin.contains('/') && !aqua_sys::have(&bin) {
        tracing::warn!("aqua-polkit-agent not found: apps asking for administrator rights will fail");
        return;
    }
    std::thread::spawn(move || {
        let mut failures = 0;
        while failures < 5 {
            let started = std::time::Instant::now();
            match std::process::Command::new(&bin).stdin(std::process::Stdio::null()).status() {
                Ok(s) if s.success() => break,
                Ok(s) => tracing::warn!("aqua-polkit-agent exited ({s}), restarting"),
                Err(e) => {
                    tracing::warn!("aqua-polkit-agent: {e}");
                    break;
                }
            }
            if started.elapsed() > Duration::from_secs(60) {
                failures = 0;
            }
            failures += 1;
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}
