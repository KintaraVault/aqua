//! Aqua login screen for greetd (Rust + Slint).
//!
//! greetd config (`/etc/greetd/config.toml`):
//! ```toml
//! [default_session]
//! command = "cage -s -- aqua-greeter"      # or: "aqua --tty --greeter"
//! user = "greeter"
//! ```
//! Talks the greetd IPC protocol over `$GREETD_SOCK` (u32 length + JSON) and starts the
//! selected session (`aqua-session` by default). `--demo` runs without greetd (any
//! password containing "fail" is rejected) for previews and tests.
use aqua_ui::*;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Extract a top-level string field from a flat JSON object (enough for greetd replies).
fn field(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = json.find(&pat)? + pat.len();
    let rest = json[i..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut it = rest.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                'u' => {
                    let h: String = it.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?).unwrap_or('?'));
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

struct Greetd {
    sock: UnixStream,
}

impl Greetd {
    fn connect() -> std::io::Result<Self> {
        let path = std::env::var("GREETD_SOCK").map_err(|_| std::io::Error::other("GREETD_SOCK not set"))?;
        Ok(Self { sock: UnixStream::connect(path)? })
    }
    fn call(&mut self, json: &str) -> std::io::Result<String> {
        self.sock.write_all(&(json.len() as u32).to_ne_bytes())?;
        self.sock.write_all(json.as_bytes())?;
        let mut len = [0u8; 4];
        self.sock.read_exact(&mut len)?;
        let mut buf = vec![0u8; u32::from_ne_bytes(len) as usize];
        self.sock.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

enum Outcome {
    Ok,
    Denied(String),
}

/// Full greetd conversation: create_session → answer prompts → start_session.
fn login(user: &str, password: &str, cmd: &[String]) -> Result<Outcome, String> {
    let mut g = Greetd::connect().map_err(|e| e.to_string())?;
    let mut reply =
        g.call(&format!("{{\"type\":\"create_session\",\"username\":{}}}", esc(user))).map_err(|e| e.to_string())?;
    let mut answered = false;
    loop {
        match field(&reply, "type").as_deref() {
            Some("auth_message") => {
                let kind = field(&reply, "auth_message_type").unwrap_or_default();
                let resp = match kind.as_str() {
                    "secret" if !answered => {
                        answered = true;
                        format!("{{\"type\":\"post_auth_message_response\",\"response\":{}}}", esc(password))
                    }
                    "secret" | "visible" => {
                        let _ = g.call("{\"type\":\"cancel_session\"}");
                        return Ok(Outcome::Denied(field(&reply, "auth_message").unwrap_or_default()));
                    }
                    _ => "{\"type\":\"post_auth_message_response\"}".into(),
                };
                reply = g.call(&resp).map_err(|e| e.to_string())?;
            }
            Some("success") => {
                let args: Vec<String> = cmd.iter().map(|c| esc(c)).collect();
                let env = "[\"XDG_SESSION_TYPE=wayland\",\"XDG_CURRENT_DESKTOP=Aqua\"]";
                let r = g
                    .call(&format!("{{\"type\":\"start_session\",\"cmd\":[{}],\"env\":{env}}}", args.join(",")))
                    .map_err(|e| e.to_string())?;
                return match field(&r, "type").as_deref() {
                    Some("success") => Ok(Outcome::Ok),
                    _ => Err(field(&r, "description").unwrap_or_else(|| "could not start session".into())),
                };
            }
            _ => {
                let _ = g.call("{\"type\":\"cancel_session\"}");
                let auth = field(&reply, "error_type").as_deref() == Some("auth_error");
                let d = field(&reply, "description").unwrap_or_default();
                return if auth { Ok(Outcome::Denied(d)) } else { Err(d) };
            }
        }
    }
}

fn users() -> Vec<(String, String)> {
    let min_uid: u32 = std::fs::read_to_string("/etc/login.defs")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("UID_MIN"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(1000);
    let mut v: Vec<(String, String)> = std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let shell = f.get(6).copied().unwrap_or("");
            if uid < min_uid || uid >= 60000 || shell.ends_with("nologin") || shell.ends_with("false") {
                return None;
            }
            let full = f
                .get(4)
                .map(|g| g.split(',').next().unwrap_or("").to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| f[0].to_string());
            Some((f[0].to_string(), full))
        })
        .collect();
    if v.is_empty() {
        let (l, f) = user_names();
        v.push((l, f));
    }
    v
}

/// Wayland sessions: (name, command); Aqua first.
fn sessions() -> Vec<(String, Vec<String>)> {
    let mut v = vec![("Aqua".to_string(), vec!["aqua-session".to_string()])];
    for dir in ["/usr/share/wayland-sessions", "/usr/local/share/wayland-sessions"] {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let t = std::fs::read_to_string(e.path()).unwrap_or_default();
            let get = |k: &str| t.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(str::to_string);
            if let (Some(n), Some(x)) = (get("Name"), get("Exec")) {
                if n != "Aqua" && !v.iter().any(|s| s.0 == n) {
                    v.push((n, x.split_whitespace().map(str::to_string).collect()));
                }
            }
        }
    }
    v
}

fn now() -> (String, String) {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        const D: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        (
            trf(
                "{weekday} {day} {month}",
                &[
                    ("weekday", &tr(D[tm.tm_wday.clamp(0, 6) as usize])),
                    ("day", &tm.tm_mday),
                    ("month", &tr(M[tm.tm_mon.clamp(0, 11) as usize])),
                ],
            ),
            format!("{}:{:02}", tm.tm_hour, tm.tm_min),
        )
    }
}

fn main() -> Result<(), slint::PlatformError> {
    aqua_ui::init("org.aqua.greeter");
    let demo = std::env::args().any(|a| a == "--demo");
    let ui = GreeterWindow::new()?;
    aqua_ui::init_translations();
    aqua_ui::set_app_id();
    let g = ui.global::<G>();
    let us = users();
    g.set_users(ModelRc::new(VecModel::from(
        us.iter()
            .map(|(l, f)| GUser {
                name: l.into(),
                full: f.into(),
                initial: f.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
            })
            .collect::<Vec<_>>(),
    )));
    let ss = sessions();
    g.set_sessions(ModelRc::new(VecModel::from(
        ss.iter().map(|s| slint::SharedString::from(s.0.as_str())).collect::<Vec<_>>(),
    )));
    let cfg = aqua_config::Config::load();
    if let Some(w) = cfg.wallpaper.as_ref().and_then(|p| slint::Image::load_from_path(p).ok()) {
        g.set_wallpaper(w);
    }
    g.set_layout(
        cfg.keyboard
            .layouts
            .first()
            .map(|l| if l == "us" { "ABC".to_string() } else { l.to_uppercase() })
            .unwrap_or("ABC".into())
            .into(),
    );
    let (d, t) = now();
    g.set_date(d.into());
    g.set_time(t.into());
    let clock = slint::Timer::default();
    clock.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), {
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                let (d, t) = now();
                ui.global::<G>().set_date(d.into());
                ui.global::<G>().set_time(t.into());
            }
        }
    });
    g.on_pick_user({
        let ui = ui.as_weak();
        move |i| {
            let ui = ui.unwrap();
            let g = ui.global::<G>();
            g.set_user_idx(i);
            g.set_password("".into());
            g.set_message("".into());
        }
    });
    g.on_power(|a| {
        let _ = match a.as_str() {
            "suspend" => aqua_sys::session::suspend(),
            "reboot" => aqua_sys::session::reboot(),
            _ => aqua_sys::session::power_off(),
        };
    });
    g.on_submit({
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            let g = ui.global::<G>();
            if g.get_busy() {
                return;
            }
            let user = us.get(g.get_user_idx().max(0) as usize).map(|u| u.0.clone()).unwrap_or_default();
            let pw = g.get_password().to_string();
            let cmd = ss
                .get(g.get_session_idx().max(0) as usize)
                .map(|s| s.1.clone())
                .unwrap_or_else(|| vec!["aqua-session".into()]);
            g.set_busy(true);
            g.set_error(false);
            g.set_message("".into());
            let weak = ui.as_weak();
            std::thread::spawn(move || {
                let r = if demo {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    if pw.contains("fail") || pw.is_empty() {
                        Ok(Outcome::Denied(String::new()))
                    } else {
                        Ok(Outcome::Ok)
                    }
                } else {
                    login(&user, &pw, &cmd)
                };
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let g = ui.global::<G>();
                    g.set_busy(false);
                    match r {
                        Ok(Outcome::Ok) => {
                            if demo {
                                g.set_message(trf("Welcome, {user}", &[("user", &user)]).into());
                            } else {
                                std::process::exit(0);
                            }
                        }
                        Ok(Outcome::Denied(msg)) => {
                            g.set_password("".into());
                            g.set_error(true);
                            let msg = msg.trim();
                            g.set_message(if msg.is_empty() {
                                tr("Incorrect password. Try again.").into()
                            } else {
                                msg.into()
                            });
                        }
                        Err(e) => {
                            g.set_error(true);
                            g.set_message(trf("Login failed: {e}", &[("e", &e)]).into());
                        }
                    }
                });
            });
        }
    });
    ui.window().set_fullscreen(true);
    aqua_ui::run(ui.run())
}
