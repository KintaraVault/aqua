//! Scripted control: `aqua msg`, `$AQUA_CTL` and the test hooks of the command line.
use crate::cli::Args;
use crate::state::Aqua;
use smithay::reexports::calloop::EventLoop;
use std::time::Duration;

pub fn schedule_test_hooks(event_loop: &mut EventLoop<'static, Aqua>, args: &Args) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    if let Some((path, after)) = args.screenshot.clone() {
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_secs_f64(after)), move |_, _, st| {
                st.screenshot_request = Some(path.clone());
                st.needs_redraw = true;
                TimeoutAction::Drop
            })
            .ok();
    }
    if let Some(q) = args.quit_after {
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_secs_f64(q)), |_, _, st| {
                st.loop_signal.stop();
                TimeoutAction::Drop
            })
            .ok();
    }
    if let Ok(ctl) = std::env::var("AQUA_CTL") {
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_millis(200)), move |_, _, st| {
                if let Ok(s) = std::fs::read_to_string(&ctl) {
                    if !s.trim().is_empty() {
                        let _ = std::fs::write(&ctl, "");
                        for line in s.lines() {
                            st.control_command(line.trim());
                        }
                    }
                }
                TimeoutAction::ToDuration(Duration::from_millis(200))
            })
            .ok();
    }
}

impl Aqua {
    /// Write a short JSON-ish state summary (windows, outputs, lock) for tests.
    pub fn dump_state(&mut self, path: &str) {
        use std::fmt::Write;
        let mut o = String::new();
        let _ = writeln!(o, "locked={:?} progress={:.2}", self.lock.mode, self.lock.lock_progress());
        let _ = writeln!(o, "idle_stage={:?} dim={:.2}", self.idle.stage, self.idle.dim_amount());
        let layout = self.active_layout();
        let _ = writeln!(o, "layout={} layouts={:?}", layout, self.input_cfg.layouts);
        for out in self.outputs.list.clone() {
            let _ = writeln!(
                o,
                "output {} geo={:?} scale={}",
                out.name(),
                self.space.output_geometry(&out),
                out.current_scale().fractional_scale()
            );
        }
        let focused = self.focused_window();
        for w in self.space.elements() {
            let (id, title) = crate::state::title_of(w);
            let x11 = w.x11_surface().is_some();
            let _ = writeln!(
                o,
                "window app={id:?} title={title:?} x11={x11} geo={:?} focused={} ssd={} tiled={:?} menu={:?}",
                self.space.element_geometry(w),
                Some(w) == focused.as_ref(),
                crate::state::is_ssd(w),
                crate::state::meta(w).borrow().tiled.map(|t| t.name()),
                crate::wayland::appmenu::window_address(w)
            );
        }
        let addr = focused.as_ref().and_then(|w| self.app_menu_address(w));
        let menus = aqua_tray::appmenu::current()
            .map(|m| m.titles().iter().map(|n| n.label.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let _ = writeln!(o, "appmenu={addr:?} menus={menus:?}");
        let rows: Vec<String> = aqua_shell::tray::app_menu_entries(0)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.map(|e| format!("{}{}", e.label, if e.shortcut.is_empty() { String::new() } else { format!(" [{}]", e.shortcut) })).unwrap_or("-".into()))
            .collect();
        let _ = writeln!(o, "appmenu_first={rows:?}");
        let _ = writeln!(
            o,
            "stage on={} active={:?} strip={:?} staged={}",
            self.stage.on,
            self.stage.active,
            self.stage_slots().iter().map(|(a, _)| a.clone()).collect::<Vec<_>>(),
            self.stage.staged.len()
        );
        let _ = writeln!(o, "clipboard_entries={}", self.clip.entries.len());
        for e in self.clip.entries.iter().take(5) {
            let _ = writeln!(
                o,
                "  clip {} {:?}",
                e.id,
                e.data.iter().map(|(m, d)| (m.clone(), d.len())).collect::<Vec<_>>()
            );
        }
        let p = if path.is_empty() { "/tmp/aqua-state.txt" } else { path };
        let _ = std::fs::write(p, o);
    }

    /// Test/automation commands: `click X Y`, `move X Y`, `launchpad`, `control`,
    /// `menu`, `shot PATH`, `key TEXT`, `run CMD`, `quit`.
    pub fn control_command(&mut self, line: &str) {
        let mut p = line.splitn(2, ' ');
        let cmd = p.next().unwrap_or("");
        let rest = p.next().unwrap_or("").trim();
        let nums: Vec<f64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let t = smithay::backend::input::InputTime::now();
        match cmd {
            "move" if nums.len() >= 2 => self.on_motion((nums[0], nums[1]).into(), t),
            "click" if nums.len() >= 2 => {
                self.on_motion((nums[0], nums[1]).into(), t);
                self.inject_button(true, t);
                self.inject_button(false, smithay::backend::input::InputTime::now());
            }
            "rclick" if nums.len() >= 2 => {
                self.on_motion((nums[0], nums[1]).into(), t);
                self.on_button(0x111, smithay::backend::input::ButtonState::Pressed, t);
                self.on_button(
                    0x111,
                    smithay::backend::input::ButtonState::Released,
                    smithay::backend::input::InputTime::now(),
                );
            }
            "down" => self.inject_button(true, t),
            "up" => self.inject_button(false, t),
            "launchpad" => self.shell.toggle_launchpad(),
            "mission" => self.toggle_mission(),
            "desk" => match rest {
                "add" => self.add_space(),
                "next" => self.switch_space_rel(1),
                "prev" => self.switch_space_rel(-1),
                r if r.starts_with("move ") => {
                    if let (Some(w), Ok(i)) = (self.focused_window(), r[5..].trim().parse::<usize>()) {
                        self.move_to_space(&w, i);
                    }
                }
                r if r.starts_with("rm ") => {
                    if let Ok(i) = r[3..].trim().parse::<usize>() {
                        self.remove_space(i);
                    }
                }
                r => {
                    if let Ok(i) = r.parse::<usize>() {
                        self.switch_space(i);
                    }
                }
            },
            "control" => self.shell.control.toggle(),
            "spotlight" => self.shell.toggle_spotlight(),
            "nc" => self.shell.toggle_notification_center(),
            "notify" => {
                let (summary, body) = rest.split_once('|').unwrap_or((rest, ""));
                self.shell.notify(aqua_shell::notifications::Note {
                    id: 9000 + self.shell.notes.list.len() as u32,
                    app_id: "org.gnome.Calculator".into(),
                    app_name: String::new(),
                    icon: String::new(),
                    summary: summary.into(),
                    body: body.into(),
                    time: (0, 0),
                    timeout: 5.0,
                    ..Default::default()
                });
            }
            "switch" => {
                if self.shell.switcher.open {
                    self.shell.switcher.step(false);
                } else {
                    let o = self.app_mru();
                    self.shell.switcher.start(o, false);
                }
            }
            "switchend" => {
                if let Some(app) = self.shell.switcher.commit() {
                    self.handle_actions(vec![aqua_shell::Action::Activate(app)]);
                }
            }
            "key" => {
                let k = match rest {
                    "enter" => Some(aqua_shell::Key::Enter),
                    "down" => Some(aqua_shell::Key::Down),
                    "up" => Some(aqua_shell::Key::Up),
                    "back" => Some(aqua_shell::Key::Backspace),
                    _ => None,
                };
                let (_, a) = self.shell.key(k, None);
                self.handle_actions(a);
            }
            "type" => {
                for ch in rest.chars() {
                    let s = ch.to_string();
                    let (_, a) = self.shell.key(None, Some(&s));
                    self.handle_actions(a);
                }
            }
            "esc" => {
                let (_, a) = self.shell.key(Some(aqua_shell::Key::Escape), None);
                self.handle_actions(a);
            }
            "shot" => self.screenshot_request = Some(rest.to_string()),
            "screenshot" => self.start_screenshot(rest),
            "record" => match rest {
                "stop" => self.stop_recording(),
                "ui" => self.start_screenshot("record"),
                _ => self.queue_recording(aqua_shell::screenshot::Target::Full, false),
            },
            "lock" => self.lock_session(),
            "hotkey" => self.press_chord(rest),
            "sendkeys" => self.send_keys(rest),
            "typetext" => self.type_text(rest),
            "action" => {
                if !self.run_named_action(rest) {
                    tracing::warn!("unknown action {rest}");
                }
            }
            "clipboard" => self.shell.toggle_clipboard(),
            "cliptext" => self.clipboard_put_text(rest),
            "clipfiles" => self.clipboard_put_files(rest),
            "dragfiles" => self.start_file_drag(rest),
            "layout" => match rest {
                "next" | "" => self.next_layout(),
                r => {
                    if let Ok(i) = r.parse::<usize>() {
                        self.set_layout(i);
                    }
                }
            },
            "output" => {
                let a: Vec<&str> = rest.split_whitespace().collect();
                match a.as_slice() {
                    ["add", name, size, scale @ ..] => {
                        let (w, h) = size
                            .split_once('x')
                            .map(|(w, h)| (w.parse().unwrap_or(1280), h.parse().unwrap_or(800)))
                            .unwrap_or((1280, 800));
                        let sc = scale.first().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                        self.add_virtual_output(name, w, h, sc);
                    }
                    ["rm", name] => {
                        if let Some(o) = self.outputs.list.iter().find(|o| o.name() == *name).cloned() {
                            self.remove_output(&o);
                        }
                    }
                    ["scale", name, sc] => {
                        if let (Some(o), Ok(v)) =
                            (self.outputs.list.iter().find(|o| o.name() == *name).cloned(), sc.parse::<f64>())
                        {
                            o.change_current_state(None, None, Some(smithay::output::Scale::Fractional(v)), None);
                            self.arrange_outputs();
                        }
                    }
                    _ => tracing::warn!("output add NAME WxH [SCALE] | output rm NAME | output scale NAME S"),
                }
            }
            "idle" => {
                if let Ok(n) = rest.parse::<u64>() {
                    self.idle.last = std::time::Instant::now() - std::time::Duration::from_secs(n);
                    self.tick_idle();
                }
            }
            "wake" => self.notify_activity(),
            "dump" => self.dump_state(rest),
            "tilemenu" => self.tile_menu_at_pointer(),
            "reload" => self.reload_config(),
            "run" => {
                aqua_apps::launch(rest);
            }
            "quit" => self.loop_signal.stop(),
            _ => tracing::warn!("unknown control command: {line}"),
        }
        self.needs_redraw = true;
    }
}
