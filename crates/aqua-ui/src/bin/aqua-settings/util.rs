//! Small helpers shared by the panes.
use super::*;

pub fn apply_theme_from(ui: &SettingsWindow, cfg: &Config) {
    let t = ui.global::<Theme>();
    t.set_dark(is_dark(cfg));
    t.set_accent(accent_color(&cfg.accent));
    t.set_solid_sidebar(cfg.solid_sidebar());
    t.set_glass_controls(cfg.glass_controls);
    t.set_glass_lights(cfg.glass_traffic_lights);
    t.set_motion(!cfg.reduce_motion);
}

#[derive(Clone)]
pub enum LoginSrc {
    Cfg(String),
    Xdg(String),
}

/// Run `work` on a thread and deliver its result to `done` on the UI thread.
pub fn run_bg<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = std::sync::mpsc::channel::<T>();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    let timer = Rc::new(slint::Timer::default());
    let t2 = timer.clone();
    let done = RefCell::new(Some(done));
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(150), move || {
        if let Ok(v) = rx.try_recv() {
            t2.stop();
            if let Some(f) = done.borrow_mut().take() {
                f(v);
            }
        }
    });
}
