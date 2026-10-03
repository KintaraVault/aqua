//! Control socket `$XDG_RUNTIME_DIR/aqua.sock`: lets session tools talk to the running
//! compositor (`aqua msg screenshot area`, `aqua-screenshot`, `aqua msg reload`,
//! `aqua msg stats [reset]` for frame/damage/blur statistics).
//! One line per connection; only a safe subset of the test commands is accepted.
use crate::state::Aqua;
use smithay::reexports::calloop::{generic::Generic, EventLoop, Interest, Mode, PostAction};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// Socket path (`$AQUA_SOCKET` overrides).
pub fn path() -> PathBuf {
    aqua_config::paths::control_socket()
}

/// Commands accepted from the socket (first word).
const ALLOWED: &[&str] = &[
    "screenshot",
    "launchpad",
    "mission",
    "spotlight",
    "control",
    "nc",
    "clipboard",
    "lock",
    "reload",
    "layout",
    "action",
    "desk",
    "chars",
    "dump",
    "record",
    "clipfiles",
    "dragfiles",
    "stats",
];

/// Whether the first word of a control line is an accepted command.
fn allowed(line: &str) -> bool {
    line.split_whitespace().next().map(|c| ALLOWED.contains(&c)).unwrap_or(false)
}

pub fn listen(event_loop: &mut EventLoop<'static, Aqua>) {
    let p = path();
    if UnixStream::connect(&p).is_ok() {
        tracing::warn!("control socket {} is in use by another Aqua session", p.display());
        return;
    }
    let _ = std::fs::remove_file(&p);
    let listener = match UnixListener::bind(&p) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("cannot create control socket {}: {e}", p.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    let _ = listener.set_nonblocking(true);
    let res = event_loop.handle().insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, l, st| {
        while let Ok((mut s, _)) = l.accept() {
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(300)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            while buf.len() < 1 << 20 {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.contains(&b'\n') {
                            break;
                        }
                    }
                }
            }
            let text = String::from_utf8_lossy(&buf).to_string();
            let mut reply = String::new();
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let cmd = line.split_whitespace().next().unwrap_or("");
                if allowed(line) {
                    if cmd == "chars" {
                        st.shell_actions(vec![aqua_shell::Action::ShowChars]);
                    } else if cmd == "stats" {
                        reply.push_str(&aqua_render::stats::report());
                        if line.split_whitespace().nth(1) == Some("reset") {
                            aqua_render::stats::reset();
                        }
                        continue;
                    } else {
                        st.control_command(line);
                    }
                    st.needs_redraw = true;
                    reply.push_str("ok\n");
                } else {
                    reply.push_str(&format!("error: unknown command {cmd}\n"));
                }
            }
            let _ = s.write_all(reply.as_bytes());
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = res {
        tracing::warn!("control socket: {e}");
    } else {
        tracing::info!("control socket at {}", p.display());
    }
}

/// Client side of `aqua msg …`.
pub fn send(args: &[String]) -> i32 {
    let line = args.join(" ");
    let p = path();
    let mut s = match UnixStream::connect(&p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("aqua msg: cannot reach the compositor at {}: {e}", p.display());
            return 1;
        }
    };
    let _ = s.write_all(format!("{line}\n").as_bytes());
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    print!("{out}");
    if out.starts_with("error") {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_whitelist() {
        assert!(allowed("screenshot full"));
        assert!(allowed("  reload"));
        assert!(allowed("desk 2"));
        assert!(allowed("stats reset"));
        assert!(!allowed("spawn rm -rf ~"));
        assert!(!allowed(""));
        assert!(!allowed("Screenshot"));
    }
}
