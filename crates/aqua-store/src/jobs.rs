use crate::runner::{apt_status, percent, Cmd, SharedRunner};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Install,
    Remove,
    Update,
    Setup,
    Other,
}

impl Kind {
    pub fn verb(&self) -> &'static str {
        match self {
            Kind::Install => "install",
            Kind::Remove => "remove",
            Kind::Update => "update",
            Kind::Setup => "setup",
            Kind::Other => "other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub key: String,
    pub title: String,
    pub kind: Kind,
    pub steps: Vec<Cmd>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Queued { id: u64, key: String },
    Started { id: u64, key: String },
    Progress { id: u64, key: String, fraction: f32, status: String },
    Finished { id: u64, key: String, kind: Kind, ok: bool, cancelled: bool, error: String, log: String },
}

impl Event {
    pub fn key(&self) -> &str {
        match self {
            Event::Queued { key, .. }
            | Event::Started { key, .. }
            | Event::Progress { key, .. }
            | Event::Finished { key, .. } => key,
        }
    }
}

pub fn explain(code: i32, stderr: &str, cmd: &Cmd) -> String {
    if code == -2 {
        return "Cancelled.".into();
    }
    if cmd.root && code == 126 {
        return "Authentication was cancelled.".into();
    }
    if cmd.root && code == 127 && stderr.contains("Not authorized") {
        return "You are not authorized to make this change.".into();
    }
    let lower = stderr.to_lowercase();
    if lower.contains("could not resolve")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("network is unreachable")
    {
        return "Check your internet connection and try again.".into();
    }
    if lower.contains("unable to lock database")
        || lower.contains("could not get lock")
        || lower.contains("waiting for process")
    {
        return "Another package operation is running. Try again when it finishes.".into();
    }
    if lower.contains("no space left") || lower.contains("not enough disk space") {
        return "There is not enough disk space.".into();
    }
    let tail: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("warning:") && !l.starts_with("W:"))
        .collect();
    let last = tail.iter().rev().find(|l| l.to_lowercase().contains("error")).or(tail.last()).copied().unwrap_or("");
    let last = ["error:", "Error:", "E:"].iter().fold(last, |l, p| l.strip_prefix(p).unwrap_or(l)).trim();
    if last.is_empty() {
        format!("“{}” failed (exit code {code}).", cmd.program())
    } else {
        last.chars().take(300).collect()
    }
}

fn status_line(l: &str) -> String {
    let t: String = l.chars().filter(|c| !c.is_control()).collect();
    let t = t.trim().trim_matches(|c: char| c == '[' || c == ']' || c == '#' || c == '-' || c == '=');
    t.chars().take(120).collect::<String>().trim().to_string()
}

pub struct Jobs {
    tx: Sender<Job>,
    next: AtomicU64,
    cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    keys: Arc<Mutex<HashMap<u64, String>>>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
}

impl Jobs {
    pub fn spawn(run: SharedRunner, on_event: impl Fn(Event) + Send + Sync + 'static) -> Jobs {
        let (tx, rx) = channel::<Job>();
        let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::default();
        let keys: Arc<Mutex<HashMap<u64, String>>> = Arc::default();
        let on_event: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(on_event);
        {
            let cancels = cancels.clone();
            let keys = keys.clone();
            let ev = on_event.clone();
            std::thread::Builder::new()
                .name("store-jobs".into())
                .spawn(move || {
                    for job in rx {
                        let flag = cancels.lock().unwrap().get(&job.id).cloned().unwrap_or_default();
                        let res = execute(&*run, &job, &flag, &*ev);
                        cancels.lock().unwrap().remove(&job.id);
                        keys.lock().unwrap().remove(&job.id);
                        ev(res);
                    }
                })
                .expect("job thread");
        }
        Jobs { tx, next: AtomicU64::new(1), cancels, keys, on_event }
    }

    pub fn submit(&self, key: &str, title: &str, kind: Kind, steps: Vec<Cmd>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.cancels.lock().unwrap().insert(id, Arc::new(AtomicBool::new(false)));
        self.keys.lock().unwrap().insert(id, key.to_string());
        (self.on_event)(Event::Queued { id, key: key.into() });
        let _ = self.tx.send(Job { id, key: key.into(), title: title.into(), kind, steps });
        id
    }

    pub fn cancel(&self, id: u64) {
        if let Some(f) = self.cancels.lock().unwrap().get(&id) {
            f.store(true, Ordering::Relaxed);
        }
    }

    pub fn cancel_key(&self, key: &str) {
        let ids: Vec<u64> = self.keys.lock().unwrap().iter().filter(|(_, k)| *k == key).map(|(i, _)| *i).collect();
        for id in ids {
            self.cancel(id);
        }
    }

    pub fn busy(&self, key: &str) -> bool {
        self.keys.lock().unwrap().values().any(|k| k == key)
    }

    pub fn pending(&self) -> usize {
        self.keys.lock().unwrap().len()
    }
}

pub fn execute(run: &dyn crate::runner::Runner, job: &Job, cancel: &AtomicBool, ev: &dyn Fn(Event)) -> Event {
    let key = job.key.clone();
    let finished = |ok: bool, cancelled: bool, error: String, log: String| Event::Finished {
        id: job.id,
        key: key.clone(),
        kind: job.kind.clone(),
        ok,
        cancelled,
        error,
        log,
    };
    if cancel.load(Ordering::Relaxed) {
        return finished(false, true, "Cancelled.".into(), String::new());
    }
    ev(Event::Started { id: job.id, key: key.clone() });
    let n = job.steps.len().max(1) as f32;
    let mut log = String::new();
    for (i, step) in job.steps.iter().enumerate() {
        let base = i as f32 / n;
        let label = if step.label.is_empty() { job.title.clone() } else { step.label.clone() };
        ev(Event::Progress {
            id: job.id,
            key: key.clone(),
            fraction: if job.steps.len() > 1 { base } else { -1.0 },
            status: label.clone(),
        });
        log.push_str(&format!("$ {}\n", step.line()));
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut on_line = |l: &str| {
            let (frac, msg) = match apt_status(l) {
                Some((f, m)) => (Some(f), m),
                None => (percent(l), status_line(l)),
            };
            if last_emit.elapsed().as_millis() < 120 && frac.is_none() {
                return;
            }
            last_emit = std::time::Instant::now();
            let fraction = frac.map(|f| base + f / n).unwrap_or(if job.steps.len() > 1 { base } else { -1.0 });
            ev(Event::Progress {
                id: job.id,
                key: key.clone(),
                fraction,
                status: if msg.is_empty() { label.clone() } else { msg },
            });
        };
        let out = run.stream(step, &mut on_line, cancel);
        log.push_str(&out.stdout);
        log.push_str(&out.stderr);
        if out.code == -2 || cancel.load(Ordering::Relaxed) {
            return finished(false, true, "Cancelled.".into(), log);
        }
        if !step.ok_codes.contains(&out.code) {
            let combined = format!("{}\n{}", out.stdout, out.stderr);
            return finished(false, false, explain(out.code, &combined, step), log);
        }
    }
    ev(Event::Progress { id: job.id, key: key.clone(), fraction: 1.0, status: String::new() });
    finished(true, false, String::new(), log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;
    use crate::runner::Output;

    #[test]
    fn runs_steps_and_reports() {
        let r = FakeRunner::default();
        r.on("a", 0, "10%\n50%\n");
        r.on("b", 3, "");
        let job = Job {
            id: 1,
            key: "k".into(),
            title: "T".into(),
            kind: Kind::Install,
            steps: vec![Cmd::new(["a"]), Cmd::new(["b"])],
        };
        let events = Mutex::new(vec![]);
        let res = execute(&r, &job, &AtomicBool::new(false), &|e| events.lock().unwrap().push(e));
        match res {
            Event::Finished { ok, error, .. } => {
                assert!(!ok);
                assert_eq!(error, "“b” failed (exit code 3).");
            }
            _ => panic!(),
        }
        let ev = events.lock().unwrap();
        assert!(matches!(ev[0], Event::Started { .. }));
        let fr: Vec<f32> = ev
            .iter()
            .filter_map(|e| if let Event::Progress { fraction, .. } = e { Some(*fraction) } else { None })
            .collect();
        assert!(fr.contains(&0.25), "{fr:?}");
        assert!(fr.contains(&0.5));
    }

    #[test]
    fn cancelled_before_start() {
        let r = FakeRunner::default();
        let job = Job { id: 1, key: "k".into(), title: "T".into(), kind: Kind::Remove, steps: vec![Cmd::new(["a"])] };
        let res = execute(&r, &job, &AtomicBool::new(true), &|_| {});
        assert!(matches!(res, Event::Finished { cancelled: true, .. }));
        assert!(r.calls().is_empty());
    }

    #[test]
    fn explanations() {
        let c = Cmd::new(["pacman"]).root();
        assert_eq!(explain(126, "", &c), "Authentication was cancelled.");
        assert_eq!(
            explain(1, "Error: Failed to install org.x: Not enough disk space to complete this operation", &c),
            "There is not enough disk space."
        );
        assert_eq!(explain(1, "Error: Remote not found", &c), "Remote not found");
        assert_eq!(
            explain(1, "error: failed to init transaction (unable to lock database)", &c),
            "Another package operation is running. Try again when it finishes."
        );
        assert_eq!(explain(1, "warning: x\nerror: target not found: foo\n", &c), "target not found: foo");
        assert_eq!(
            explain(100, "E: Unable to locate package foo", &Cmd::new(["apt-get"])),
            "Unable to locate package foo"
        );
    }

    #[test]
    fn queue_thread() {
        let r = Arc::new(FakeRunner::default());
        r.on("ok", 0, "");
        let (tx, rx) = channel();
        let jobs = Jobs::spawn(r.clone(), move |e| {
            let _ = tx.send(e);
        });
        jobs.submit("x", "X", Kind::Update, vec![Cmd::new(["ok"])]);
        let mut done = false;
        while let Ok(e) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            if let Event::Finished { ok, .. } = e {
                done = ok;
                break;
            }
        }
        assert!(done);
        let _ = Output::default();
    }
}
