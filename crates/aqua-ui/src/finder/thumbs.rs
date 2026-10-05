//! Background previews: image thumbnails, PDF / video first frames (poppler, ffmpeg),
//! text snippets and application icons (rendered like the Dock does).
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};

#[derive(Clone)]
pub enum Job {
    /// path, kind, mtime
    File(PathBuf, i32, i64),
    /// key path, desktop id, name, Icon=
    App(PathBuf, String, String, String),
}

pub struct Done {
    pub key: PathBuf,
    /// RGBA8 (premultiplied when `premul`)
    pub img: Option<(u32, u32, Vec<u8>, bool)>,
    pub snippet: String,
}

pub struct Worker {
    queue: Arc<(Mutex<Vec<Job>>, Condvar)>,
    pub rx: mpsc::Receiver<Done>,
}

impl Worker {
    pub fn spawn(threads: usize) -> Self {
        let queue: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::new((Mutex::new(vec![]), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        for n in 0..threads.max(1) {
            let q = queue.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name(format!("finder-thumbs-{n}"))
                .spawn(move || {
                    let mut icons: Option<aqua_icons::IconProvider> = None;
                    loop {
                        let job = {
                            let (m, cv) = &*q;
                            let mut g = m.lock().unwrap();
                            while g.is_empty() {
                                g = cv.wait(g).unwrap();
                            }
                            g.pop().unwrap()
                        };
                        let done = match job {
                            Job::File(p, kind, mtime) => file_preview(&p, kind, mtime),
                            Job::App(key, id, name, icon) => {
                                let prov = icons.get_or_insert_with(|| {
                                    let cfg = aqua_config::Config::load();
                                    let fonts = Arc::new(aqua_gfx::Fonts::load(&cfg.font_dir()));
                                    let mut p = aqua_icons::IconProvider::new(cfg.icon_cache(), fonts, false);
                                    p.set_look(aqua_icons::look::Look {
                                        style: aqua_icons::look::Style::from_config(&cfg.icon_style, cfg.dark),
                                        dark: cfg.dark,
                                        tint: cfg.accent_rgb(),
                                        glass: cfg.icon_glass,
                                    });
                                    p.set_policy(aqua_icons::equiv::Policy {
                                        mode: cfg.apple_icons.clone(),
                                        apps: cfg.apple_icon_apps.clone(),
                                        owners: vec![],
                                    });
                                    p
                                });
                                let pm = prov.get(&aqua_icons::IconRequest { id, name, icon }, 128);
                                Done {
                                    key,
                                    img: Some((pm.width(), pm.height(), pm.data().to_vec(), true)),
                                    snippet: String::new(),
                                }
                            }
                        };
                        if tx.send(done).is_err() {
                            return;
                        }
                    }
                })
                .ok();
        }
        Self { queue, rx }
    }

    pub fn push(&self, j: Job) {
        let (m, cv) = &*self.queue;
        m.lock().unwrap().push(j);
        cv.notify_one();
    }

    /// Forget queued work (navigated away).
    pub fn clear(&self) {
        self.queue.0.lock().unwrap().clear();
    }
}

fn cache_path(p: &Path, mtime: i64) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    mtime.hash(&mut h);
    dirs::cache_dir().unwrap_or_else(|| "/tmp".into()).join("aqua/thumbs").join(format!("{:016x}.png", h.finish()))
}

fn load_png(p: &Path) -> Option<(u32, u32, Vec<u8>, bool)> {
    let i = image::open(p).ok()?.to_rgba8();
    Some((i.width(), i.height(), i.into_raw(), false))
}

fn file_preview(p: &Path, kind: i32, mtime: i64) -> Done {
    let key = p.to_path_buf();
    // FIFOs, sockets and devices: opening one for a preview blocks (or reads forever).
    let Some(size) = std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len()) else {
        return Done { key, img: None, snippet: String::new() };
    };
    if kind == 6 || (kind == 1 && size < 512 * 1024) {
        return Done { key, img: None, snippet: snippet(p).unwrap_or_default() };
    }
    let cached = cache_path(p, mtime);
    if let Some(i) = load_png(&cached) {
        return Done { key, img: Some(i), snippet: String::new() };
    }
    let img = match kind {
        2 if size < 80_000_000 => decode_image(p),
        8 if aqua_sys::have("pdftoppm") => {
            let stem = cached.with_extension("");
            let _ = std::fs::create_dir_all(cached.parent().unwrap());
            let ok = std::process::Command::new("pdftoppm")
                .args(["-png", "-singlefile", "-f", "1", "-scale-to", "256"])
                .arg(p)
                .arg(&stem)
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return Done { key, img: load_png(&cached), snippet: String::new() };
            }
            None
        }
        4 if aqua_sys::have("ffmpegthumbnailer") || aqua_sys::have("ffmpeg") => {
            let _ = std::fs::create_dir_all(cached.parent().unwrap());
            let ok = if aqua_sys::have("ffmpegthumbnailer") {
                std::process::Command::new("ffmpegthumbnailer")
                    .arg("-i")
                    .arg(p)
                    .arg("-o")
                    .arg(&cached)
                    .args(["-s", "256"])
                    .stderr(std::process::Stdio::null())
                    .status()
            } else {
                std::process::Command::new("ffmpeg")
                    .args(["-v", "quiet", "-y", "-ss", "1", "-i"])
                    .arg(p)
                    .args(["-frames:v", "1", "-vf", "scale=256:-1"])
                    .arg(&cached)
                    .stdin(std::process::Stdio::null())
                    .status()
            }
            .map(|s| s.success())
            .unwrap_or(false);
            if ok {
                return Done { key, img: load_png(&cached), snippet: String::new() };
            }
            None
        }
        _ => None,
    };
    if let Some((w, h, data, _)) = &img {
        if let Some(buf) = image::RgbaImage::from_raw(*w, *h, data.clone()) {
            let _ = std::fs::create_dir_all(cached.parent().unwrap());
            let _ = buf.save(&cached);
        }
    }
    Done { key, img, snippet: String::new() }
}

fn decode_image(p: &Path) -> Option<(u32, u32, Vec<u8>, bool)> {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "svg" {
        let data = aqua_gfx::read_regular(p, 32 << 20)?;
        let tree = resvg::usvg::Tree::from_data(&data, &aqua_icons::theme::svg_options()).ok()?;
        let s = tree.size();
        let k = 256.0 / s.width().max(s.height()).max(1.0);
        let (w, h) = (((s.width() * k).ceil() as u32).max(1), ((s.height() * k).ceil() as u32).max(1));
        let mut pm = resvg::tiny_skia::Pixmap::new(w, h)?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
        return Some((w, h, pm.data().to_vec(), true));
    }
    let i = aqua_gfx::decode_limited(&aqua_gfx::read_regular(p, 512 << 20)?)?.thumbnail(256, 256).to_rgba8();
    Some((i.width(), i.height(), i.into_raw(), false))
}

/// Open `p` for reading only if it is a regular file: a FIFO would block the open (and the
/// thread with it) until some writer appears, a device can stream forever.
pub fn open_regular(p: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    if !std::fs::metadata(p).ok()?.is_file() {
        return None;
    }
    // O_NONBLOCK closes the race where the path is swapped for a FIFO after the check.
    let f = std::fs::File::options().read(true).custom_flags(0o4000).open(p).ok()?;
    f.metadata().ok()?.is_file().then_some(f)
}

/// First ~40 lines of a text file (None for binaries).
pub fn snippet(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = open_regular(p)?;
    let mut buf = vec![0u8; 3000];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.is_empty() || buf.iter().take(1024).any(|&b| b == 0) {
        return None;
    }
    let s = String::from_utf8_lossy(&buf);
    if s.chars().filter(|c| *c == '\u{fffd}').count() > 4 {
        return None;
    }
    Some(s.lines().take(40).map(|l| l.replace('\t', "    ")).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Previews of special files must return at once instead of hanging the worker.
    #[test]
    fn special_files_do_not_block() {
        let d = std::env::temp_dir().join(format!("aqua-thumb-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let fifo = d.join("pipe.txt");
        let img = d.join("pipe.png");
        for p in [&fifo, &img] {
            assert!(std::process::Command::new("mkfifo").arg(p).status().unwrap().success());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (f2, i2) = (fifo.clone(), img.clone());
        std::thread::spawn(move || {
            let a = snippet(&f2).is_none();
            let b = file_preview(&f2, 6, 0).snippet.is_empty();
            let c = file_preview(&i2, 2, 0).img.is_none();
            let e = snippet(Path::new("/dev/zero")).is_none();
            tx.send(a && b && c && e).unwrap();
        });
        let ok = rx.recv_timeout(std::time::Duration::from_secs(5));
        // Unblock a thread stuck in open() before failing.
        let _ = std::fs::OpenOptions::new().write(true).custom_flags_nonblock().open(&fifo);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(ok, Ok(true), "preview of a FIFO or device blocked or produced data");
    }

    trait NonBlock {
        fn custom_flags_nonblock(&mut self) -> &mut Self;
    }
    impl NonBlock for std::fs::OpenOptions {
        fn custom_flags_nonblock(&mut self) -> &mut Self {
            use std::os::unix::fs::OpenOptionsExt;
            self.custom_flags(0o4000)
        }
    }

    #[test]
    fn regular_files_still_preview() {
        let p = std::env::temp_dir().join(format!("aqua-thumb-txt-{}.txt", std::process::id()));
        std::fs::write(&p, "line one\n\tline two\n").unwrap();
        assert_eq!(snippet(&p).as_deref(), Some("line one\n    line two"));
        let _ = std::fs::remove_file(&p);
    }
}
