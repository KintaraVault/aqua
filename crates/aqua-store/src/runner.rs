use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub root: bool,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub ok_codes: Vec<i32>,
    pub label: String,
}

impl Cmd {
    pub fn new<I, S>(argv: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Cmd { argv: argv.into_iter().map(Into::into).collect(), ok_codes: vec![0], ..Default::default() }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Cmd {
        self.argv.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, a: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.argv.extend(a.into_iter().map(Into::into));
        self
    }

    pub fn root(mut self) -> Cmd {
        self.root = true;
        self
    }

    pub fn env(mut self, k: &str, v: &str) -> Cmd {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn cwd(mut self, p: impl Into<PathBuf>) -> Cmd {
        self.cwd = Some(p.into());
        self
    }

    pub fn ok(mut self, code: i32) -> Cmd {
        self.ok_codes.push(code);
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Cmd {
        self.label = l.into();
        self
    }

    pub fn program(&self) -> &str {
        self.argv.first().map(String::as_str).unwrap_or("")
    }

    pub fn line(&self) -> String {
        aqua_apps::shell_join(&self.argv)
    }

    pub fn final_argv(&self, is_root: bool, elevate: &str) -> Vec<String> {
        if !self.root || is_root {
            return self.argv.clone();
        }
        let mut v = vec![elevate.to_string()];
        if !self.env.is_empty() {
            v.push("env".into());
            v.extend(self.env.iter().map(|(k, val)| format!("{k}={val}")));
        }
        v.extend(self.argv.iter().cloned());
        v
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    pub fn failed(msg: &str) -> Output {
        Output { code: 127, stdout: String::new(), stderr: msg.into() }
    }
}

pub trait Runner: Send + Sync {
    fn run(&self, cmd: &Cmd) -> Output;
    fn have(&self, prog: &str) -> bool;
    fn stream(&self, cmd: &Cmd, on_line: &mut dyn FnMut(&str), cancel: &AtomicBool) -> Output {
        if cancel.load(Ordering::Relaxed) {
            return Output::failed("cancelled");
        }
        let o = self.run(cmd);
        for l in o.stdout.lines().chain(o.stderr.lines()) {
            on_line(l);
        }
        o
    }
}

pub struct SystemRunner {
    pub elevate: String,
}

impl Default for SystemRunner {
    fn default() -> Self {
        SystemRunner { elevate: "pkexec".into() }
    }
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

impl SystemRunner {
    fn command(&self, cmd: &Cmd) -> Option<Command> {
        let argv = cmd.final_argv(is_root(), &self.elevate);
        let (prog, rest) = argv.split_first()?;
        let mut c = Command::new(prog);
        c.args(rest).stdin(Stdio::null()).env("LC_ALL", "C").env("LANG", "C");
        for (k, v) in &cmd.env {
            c.env(k, v);
        }
        if let Some(d) = &cmd.cwd {
            c.current_dir(d);
        }
        Some(c)
    }
}

impl Runner for SystemRunner {
    fn run(&self, cmd: &Cmd) -> Output {
        let Some(mut c) = self.command(cmd) else { return Output::failed("empty command") };
        match c.output() {
            Ok(o) => Output {
                code: o.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Output::failed(&format!("{}: {e}", c.get_program().to_string_lossy())),
        }
    }

    fn have(&self, prog: &str) -> bool {
        aqua_apps::find_in_path(prog).is_some()
    }

    fn stream(&self, cmd: &Cmd, on_line: &mut dyn FnMut(&str), cancel: &AtomicBool) -> Output {
        let Some(mut c) = self.command(cmd) else { return Output::failed("empty command") };
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => return Output::failed(&format!("{}: {e}", c.get_program().to_string_lossy())),
        };
        let (tx, rx) = mpsc::channel::<(bool, String)>();
        let mut readers = vec![];
        for (is_err, pipe) in [
            (false, child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>)),
            (true, child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)),
        ] {
            let Some(mut pipe) = pipe else { continue };
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut acc = Vec::new();
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            for &b in &buf[..n] {
                                if b == b'\n' || b == b'\r' {
                                    if !acc.is_empty() {
                                        let _ = tx.send((is_err, String::from_utf8_lossy(&acc).into_owned()));
                                        acc.clear();
                                    }
                                } else {
                                    acc.push(b);
                                }
                            }
                        }
                    }
                }
                if !acc.is_empty() {
                    let _ = tx.send((is_err, String::from_utf8_lossy(&acc).into_owned()));
                }
            }));
        }
        drop(tx);
        let mut out = Output::default();
        let mut cancelled = false;
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok((is_err, line)) => {
                    on_line(&line);
                    let dst = if is_err { &mut out.stderr } else { &mut out.stdout };
                    dst.push_str(&line);
                    dst.push('\n');
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !cancelled && cancel.load(Ordering::Relaxed) {
                cancelled = true;
                unsafe {
                    libc::kill(child.id() as i32, libc::SIGTERM);
                }
            }
        }
        for r in readers {
            let _ = r.join();
        }
        out.code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        if cancelled {
            out.code = -2;
            out.stderr.push_str("cancelled\n");
        }
        out
    }
}

pub type SharedRunner = Arc<dyn Runner>;

pub fn percent(line: &str) -> Option<f32> {
    let b = line.as_bytes();
    let mut best = None;
    for (i, &c) in b.iter().enumerate() {
        if c != b'%' {
            continue;
        }
        let mut j = i;
        while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == b'.') {
            j -= 1;
        }
        if j < i {
            if let Ok(v) = line[j..i].parse::<f32>() {
                if (0.0..=100.0).contains(&v) {
                    best = Some(v / 100.0);
                }
            }
        }
    }
    best
}

pub fn apt_status(line: &str) -> Option<(f32, String)> {
    let mut it = line.splitn(4, ':');
    let kind = it.next()?;
    if kind != "pmstatus" && kind != "dlstatus" {
        return None;
    }
    let _pkg = it.next()?;
    let pct: f32 = it.next()?.parse().ok()?;
    let msg = it.next().unwrap_or("").to_string();
    let p = (pct / 100.0).clamp(0.0, 1.0);
    Some((if kind == "dlstatus" { p * 0.5 } else { 0.5 + p * 0.5 }, msg))
}

pub mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeRunner {
        pub responses: Mutex<Vec<(String, Output)>>,
        pub programs: Vec<String>,
        pub log: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        pub fn with(programs: &[&str]) -> FakeRunner {
            FakeRunner { programs: programs.iter().map(|s| s.to_string()).collect(), ..Default::default() }
        }

        pub fn on(&self, prefix: &str, code: i32, stdout: &str) -> &Self {
            self.responses
                .lock()
                .unwrap()
                .push((prefix.into(), Output { code, stdout: stdout.into(), stderr: String::new() }));
            self
        }

        pub fn calls(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, cmd: &Cmd) -> Output {
            let line = cmd.argv.join(" ");
            self.log.lock().unwrap().push(if cmd.root { format!("# {line}") } else { line.clone() });
            let r = self.responses.lock().unwrap();
            r.iter()
                .filter(|(p, _)| line.starts_with(p.as_str()))
                .max_by_key(|(p, _)| p.len())
                .map(|(_, o)| o.clone())
                .unwrap_or_else(|| Output::failed("no fake response"))
        }

        fn have(&self, prog: &str) -> bool {
            self.programs.iter().any(|p| p == prog)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation() {
        let c = Cmd::new(["apt-get", "install", "-y", "x"]).root().env("DEBIAN_FRONTEND", "noninteractive");
        assert_eq!(
            c.final_argv(false, "pkexec"),
            vec!["pkexec", "env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "x"]
        );
        assert_eq!(c.final_argv(true, "pkexec")[0], "apt-get");
        assert_eq!(Cmd::new(["ls"]).final_argv(false, "pkexec"), vec!["ls"]);
    }

    #[test]
    fn percentages() {
        assert_eq!(percent("Installing 2/3… ████  45%"), Some(0.45));
        assert_eq!(percent("(1/2) 12.5% and 80%"), Some(0.8));
        assert_eq!(percent("no progress"), None);
        assert_eq!(percent("150%"), None);
        assert_eq!(apt_status("pmstatus:vim:50.0:Installing vim").map(|x| x.0), Some(0.75));
        assert_eq!(apt_status("dlstatus:1:20:Retrieving").map(|x| x.0), Some(0.1));
    }

    #[test]
    fn streams_real_process() {
        let r = SystemRunner::default();
        let mut lines = vec![];
        let o = r.stream(
            &Cmd::new(["sh", "-c", "echo one; printf 'two\\rthree\\n'; echo err >&2"]),
            &mut |l| lines.push(l.to_string()),
            &AtomicBool::new(false),
        );
        assert!(o.ok());
        lines.sort();
        assert_eq!(lines, vec!["err", "one", "three", "two"]);
    }

    #[test]
    fn cancel_kills() {
        let r = SystemRunner::default();
        let cancel = AtomicBool::new(true);
        let t = std::time::Instant::now();
        let o = r.stream(&Cmd::new(["sleep", "5"]), &mut |_| {}, &cancel);
        assert_eq!(o.code, -2);
        assert!(t.elapsed().as_secs() < 3);
    }
}
