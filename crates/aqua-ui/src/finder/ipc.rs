//! Talking to the other Finder windows (each runs as its own process).
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("aqua-finder")
}

pub struct Ipc {
    listener: UnixListener,
    path: PathBuf,
}

impl Ipc {
    pub fn bind(dir: &Path) -> Option<Ipc> {
        std::fs::create_dir_all(dir).ok()?;
        let path = dir.join(format!("{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).ok()?;
        listener.set_nonblocking(true).ok()?;
        Some(Ipc { listener, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pending requests: the connection to answer on and the request line.
    pub fn poll(&self) -> Vec<(UnixStream, String)> {
        let mut v = vec![];
        while let Ok((s, _)) = self.listener.accept() {
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut line = String::new();
            if BufReader::new(&s).read_line(&mut line).is_ok() && !line.trim().is_empty() {
                v.push((s, line.trim().to_string()));
            }
        }
        v
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Sockets of the other windows; stale ones are removed.
pub fn peers(dir: &Path, own: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "sock")).collect())
        .unwrap_or_default();
    v.retain(|p| Some(p.as_path()) != own);
    v.retain(|p| {
        let alive = UnixStream::connect(p).is_ok();
        if !alive {
            let _ = std::fs::remove_file(p);
        }
        alive
    });
    v.sort();
    v
}

/// Send one request line and read the whole answer.
pub fn request(p: &Path, msg: &str) -> Option<String> {
    let mut s = UnixStream::connect(p).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(format!("{msg}\n").as_bytes()).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

pub fn reply(mut s: UnixStream, text: &str) {
    let _ = s.write_all(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_reply() {
        let d = std::env::temp_dir().join(format!("aqua-ipc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let ipc = Ipc::bind(&d).unwrap();
        std::fs::write(d.join("999999.sock"), "").unwrap();
        assert!(peers(&d, Some(ipc.path())).is_empty());
        assert!(!d.join("999999.sock").exists());
        assert_eq!(peers(&d, None), vec![ipc.path().to_path_buf()]);
        let p = ipc.path().to_path_buf();
        let t = std::thread::spawn(move || request(&p, "merge"));
        let mut got = vec![];
        for _ in 0..200 {
            got = ipc.poll();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got.len(), 1);
        let (s, line) = got.pop().unwrap();
        assert_eq!(line, "merge");
        reply(s, "/a\n/b\n");
        assert_eq!(t.join().unwrap().as_deref(), Some("/a\n/b\n"));
        let path = ipc.path().to_path_buf();
        drop(ipc);
        assert!(!path.exists());
    }
}
