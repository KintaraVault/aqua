//! Aqua polkit authentication agent.
//!
//! `aqua-polkit-agent` registers with org.freedesktop.PolicyKit1.Authority on the system bus
//! for the current session. Each BeginAuthentication spawns `aqua-polkit-agent --dialog …`, a
//! Slint dialog that drives `polkit-agent-helper-1` (PAM) itself, so the agent
//! stays responsive to CancelAuthentication.
use aqua_ui::*;
use slint::ComponentHandle;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use zbus::zvariant::{OwnedValue, Value};

static PIDS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
static PROCESS: Mutex<Option<u32>> = Mutex::new(None);

struct Agent;

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> zbus::fdo::Result<()> {
        let me = unsafe { libc::getuid() };
        let mut uids: Vec<u32> = vec![];
        for (kind, d) in &identities {
            match kind.as_str() {
                "unix-user" => {
                    if let Some(u) = d.get("uid").and_then(|v| u32::try_from(v.clone()).ok()) {
                        uids.push(u);
                    }
                }
                "unix-group" => {
                    if let Some(g) = d.get("gid").and_then(|v| u32::try_from(v.clone()).ok()) {
                        uids.extend(group_members(g));
                    }
                }
                _ => {}
            }
        }
        uids.sort();
        uids.dedup();
        if uids.is_empty() {
            return Err(zbus::fdo::Error::Failed("no unix-user identity".into()));
        }
        uids.sort_by_key(|u| {
            if *u == me {
                0
            } else if *u == 0 {
                2
            } else {
                1
            }
        });
        let users: Vec<String> = uids.iter().filter_map(|u| user_name(*u)).collect();
        let program = details.get("polkit.caller.cmdline").or(details.get("program")).cloned().unwrap_or_default();
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "aqua-polkit-agent".into());
        let mut child = std::process::Command::new(exe)
            .args([
                "--dialog",
                "--cookie",
                &cookie,
                "--action",
                &action_id,
                "--message",
                &message,
                "--program",
                &program,
                "--users",
                &users.join(","),
            ])
            .spawn()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        PIDS.lock().unwrap().push((cookie.clone(), child.id()));
        let status = blocking::unblock(move || child.wait()).await;
        PIDS.lock().unwrap().retain(|(c, _)| *c != cookie);
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Err(zbus::fdo::Error::Failed("org.freedesktop.PolicyKit1.Error.Cancelled".into())),
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        for (c, pid) in PIDS.lock().unwrap().iter() {
            if *c == cookie {
                unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
            }
        }
    }
}

/// Members of a group: supplementary members from /etc/group plus users whose primary
/// group it is.
fn group_members(gid: u32) -> Vec<u32> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let uid_of = |name: &str| {
        passwd.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.first() == Some(&name)).then(|| f.get(2)?.parse::<u32>().ok()).flatten()
        })
    };
    let mut v = vec![];
    if let Ok(g) = std::fs::read_to_string("/etc/group") {
        for l in g.lines() {
            let f: Vec<&str> = l.split(':').collect();
            if f.get(2).and_then(|x| x.parse::<u32>().ok()) == Some(gid) {
                for m in f.get(3).unwrap_or(&"").split(',').filter(|m| !m.is_empty()) {
                    if let Some(u) = uid_of(m) {
                        v.push(u);
                    }
                }
            }
        }
    }
    for l in passwd.lines() {
        let f: Vec<&str> = l.split(':').collect();
        if f.get(3).and_then(|x| x.parse::<u32>().ok()) == Some(gid) {
            if let Some(u) = f.get(2).and_then(|x| x.parse::<u32>().ok()) {
                v.push(u);
            }
        }
    }
    v
}

fn user_name(uid: u32) -> Option<String> {
    std::fs::read_to_string("/etc/passwd").ok()?.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.get(2)?.parse::<u32>().ok()? == uid).then(|| f[0].to_string())
    })
}

fn subject() -> (String, HashMap<String, Value<'static>>) {
    let mut d = HashMap::new();
    let pid: u32 = PROCESS.lock().unwrap().unwrap_or(std::process::id());
    if PROCESS.lock().unwrap().is_some() {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let start: u64 = stat
            .rsplit(')')
            .next()
            .and_then(|r| r.split_whitespace().nth(19))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        d.insert("pid".to_string(), Value::from(pid));
        d.insert("start-time".to_string(), Value::from(start));
        return ("unix-process".into(), d);
    }
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        d.insert("session-id".to_string(), Value::from(id));
        return ("unix-session".into(), d);
    }
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let start: u64 =
        stat.rsplit(')').next().and_then(|r| r.split_whitespace().nth(19)).and_then(|s| s.parse().ok()).unwrap_or(0);
    d.insert("pid".to_string(), Value::from(std::process::id()));
    d.insert("start-time".to_string(), Value::from(start));
    ("unix-process".into(), d)
}

fn run_agent() -> zbus::Result<()> {
    let path = "/org/aqua/PolicyKit1/AuthenticationAgent";
    let conn = zbus::blocking::connection::Builder::system()?.serve_at(path, Agent)?.build()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )?;
    let locale = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into());
    let subj = subject();
    let r: zbus::Result<()> = proxy.call("RegisterAuthenticationAgent", &(subj.clone(), locale.as_str(), path));
    if let Err(e) = r {
        eprintln!("aqua-polkit-agent: session registration failed ({e}); trying process subject");
        std::env::remove_var("XDG_SESSION_ID");
        let subj = subject();
        proxy.call::<_, _, ()>("RegisterAuthenticationAgent", &(subj, locale.as_str(), path))?;
    }
    eprintln!("aqua-polkit-agent: registered");
    loop {
        std::thread::park();
    }
}

/// Socket of the socket-activated, non-setuid helper (polkit ≥ 126, used by Arch).
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Talk to polkit-agent-helper-1: send the cookie, answer PAM prompts with the password.
fn authenticate(user: &str, cookie: &str, password: &str) -> Result<(), String> {
    if std::path::Path::new(HELPER_SOCKET).exists() {
        match std::os::unix::net::UnixStream::connect(HELPER_SOCKET) {
            Ok(stream) => {
                let reader = stream.try_clone().map_err(|e| e.to_string())?;
                let mut w = stream;
                writeln!(w, "{user}").map_err(|e| e.to_string())?;
                writeln!(w, "{cookie}").map_err(|e| e.to_string())?;
                return converse(BufReader::new(reader), &mut w, password);
            }
            Err(e) => eprintln!("aqua-polkit-agent: {HELPER_SOCKET}: {e}; falling back to the setuid helper"),
        }
    }
    let helper = [
        "/usr/lib/polkit-1/polkit-agent-helper-1",
        "/usr/libexec/polkit-agent-helper-1",
        "/usr/lib/policykit-1/polkit-agent-helper-1",
        "/usr/libexec/polkit-1/polkit-agent-helper-1",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
    .ok_or("polkit-agent-helper-1 not found")?;
    let mut child = std::process::Command::new(helper)
        .arg(user)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, "{cookie}").map_err(|e| e.to_string())?;
    let r = converse(out, &mut stdin, password);
    drop(stdin);
    let _ = child.wait();
    r
}

fn converse(out: impl BufRead, input: &mut impl Write, password: &str) -> Result<(), String> {
    let mut err = String::new();
    let mut answered = false;
    for line in out.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.starts_with("PAM_PROMPT_ECHO_OFF") || line.starts_with("PAM_PROMPT_ECHO_ON") {
            if answered {
                return Err(if err.is_empty() { tr("Additional authentication required").into() } else { err });
            }
            writeln!(input, "{password}").map_err(|e| e.to_string())?;
            let _ = input.flush();
            answered = true;
        } else if let Some(m) = line.strip_prefix("PAM_ERROR_MSG ") {
            err = m.to_string();
        } else if line.starts_with("PAM_TEXT_INFO") {
        } else if line.starts_with("SUCCESS") {
            return Ok(());
        } else if line.starts_with("FAILURE") {
            break;
        }
    }
    Err(err)
}

fn run_dialog(args: &HashMap<String, String>) -> Result<(), slint::PlatformError> {
    aqua_ui::init("org.aqua.polkit");
    let ui = AuthWindow::new()?;
    aqua_ui::init_translations();
    aqua_ui::set_app_id();
    apply_theme!(ui);
    let a = ui.global::<Auth>();
    let program = args.get("program").cloned().unwrap_or_default();
    let prog = program
        .split_whitespace()
        .next()
        .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
        .filter(|s| !s.is_empty());
    let heading = match &prog {
        Some(p) => trf("“{app}” is trying to modify system settings.", &[("app", p)]),
        None => tr("Authentication Required").to_string(),
    };
    let msg = args.get("message").cloned().unwrap_or_default();
    a.set_heading(heading.into());
    a.set_body(
        format!(
            "{}{}",
            if msg.is_empty() { String::new() } else { format!("{msg}\n") },
            tr("Enter your password to allow this.")
        )
        .into(),
    );
    let users: Vec<String> = args
        .get("users")
        .map(|u| u.split(',').map(str::to_string).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    a.set_user(users.first().cloned().unwrap_or_else(|| std::env::var("USER").unwrap_or_default()).into());
    let cookie = args.get("cookie").cloned().unwrap_or_default();
    let tries = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    a.on_submit({
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            let a = ui.global::<Auth>();
            a.set_busy(true);
            let (user, pw, cookie) = (a.get_user().to_string(), a.get_password().to_string(), cookie.clone());
            let weak = ui.as_weak();
            let tries = tries.clone();
            std::thread::spawn(move || {
                let r = authenticate(&user, &cookie, &pw);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let a = ui.global::<Auth>();
                    a.set_busy(false);
                    match r {
                        Ok(()) => std::process::exit(0),
                        Err(e) => {
                            let n = tries.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if n >= 3 {
                                std::process::exit(2);
                            }
                            a.set_password("".into());
                            a.set_error(
                                if !e.is_empty() { e } else { tr("Incorrect password. Try again.").into() }.into(),
                            );
                        }
                    }
                });
            });
        }
    });
    a.on_cancel(|| std::process::exit(1));
    ui.window().on_close_requested(|| std::process::exit(1));
    aqua_ui::run(ui.run())?;
    std::process::exit(1)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(|s| s == "--dialog").unwrap_or(false) {
        let mut m = HashMap::new();
        let mut it = argv.into_iter().skip(1);
        while let Some(k) = it.next() {
            if let Some(k) = k.strip_prefix("--") {
                m.insert(k.to_string(), it.next().unwrap_or_default());
            }
        }
        if let Err(e) = run_dialog(&m) {
            eprintln!("aqua-polkit-agent: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(i) = argv.iter().position(|a| a == "--process") {
        *PROCESS.lock().unwrap() = argv.get(i + 1).and_then(|p| p.parse().ok());
    }
    if let Err(e) = run_agent() {
        eprintln!("aqua-polkit-agent: {e}");
        std::process::exit(1);
    }
}
