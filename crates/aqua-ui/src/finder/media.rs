//! Quick Look playback of audio and video (ffmpeg) and PDF pages (poppler).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}

impl Probe {
    pub fn has_video(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Parse `ffprobe -of default=nw=1` output.
pub fn parse_probe(text: &str) -> Probe {
    let mut p = Probe::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        match k.trim() {
            "duration" => {
                if let Ok(d) = v.trim().parse::<f64>() {
                    p.duration = p.duration.max(d);
                }
            }
            "width" if p.width == 0 => p.width = v.trim().parse().unwrap_or(0),
            "height" if p.height == 0 => p.height = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    p
}

pub fn probe(path: &Path) -> Option<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height:format=duration"])
        .args(["-of", "default=nw=1"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let p = parse_probe(&String::from_utf8_lossy(&out.stdout));
    (out.status.success() && (p.duration > 0.0 || p.has_video())).then_some(p)
}

/// "1:05", "1:02:03"
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Frame size that fits in `max` keeping the aspect ratio (even numbers for the scaler).
pub fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let k = (max as f64 / w.max(h) as f64).min(1.0);
    let even = |v: f64| ((v.round() as u32) / 2 * 2).max(2);
    (even(w as f64 * k), even(h as f64 * k))
}

pub struct Player {
    pub path: PathBuf,
    pub info: Probe,
    pub size: (u32, u32),
    start_pos: f64,
    started: Option<Instant>,
    paused_at: f64,
    frames: Option<Receiver<Vec<u8>>>,
    children: Vec<Child>,
}

impl Player {
    pub fn new(path: &Path, info: Probe) -> Player {
        let size = fit(info.width, info.height, 960);
        Player {
            path: path.to_path_buf(),
            info,
            size,
            start_pos: 0.0,
            started: None,
            paused_at: 0.0,
            frames: None,
            children: vec![],
        }
    }

    pub fn playing(&self) -> bool {
        self.started.is_some()
    }

    pub fn position(&self) -> f64 {
        let p = match self.started {
            Some(t) => self.start_pos + t.elapsed().as_secs_f64(),
            None => self.paused_at,
        };
        if self.info.duration > 0.0 {
            p.min(self.info.duration)
        } else {
            p
        }
    }

    pub fn finished(&self) -> bool {
        self.playing() && self.info.duration > 0.0 && self.position() >= self.info.duration - 0.05
    }

    fn stop_children(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.children.clear();
        self.frames = None;
    }

    pub fn play(&mut self, at: f64) {
        self.stop_children();
        let at = if self.info.duration > 0.0 && at >= self.info.duration - 0.1 { 0.0 } else { at.max(0.0) };
        let ss = format!("{at:.3}");
        let q = super::fs::sh_quote(&self.path);
        let audio = format!(
            "if command -v ffplay >/dev/null; then exec ffplay -v quiet -nodisp -autoexit -ss {ss} {q}; \
             elif command -v pactl >/dev/null; then exec ffmpeg -v quiet -re -ss {ss} -i {q} -vn -f pulse aqua-finder; \
             else exec ffmpeg -v quiet -re -ss {ss} -i {q} -vn -f alsa default; fi"
        );
        if let Ok(c) = Command::new("sh")
            .arg("-c")
            .arg(audio)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            self.children.push(c);
        }
        if self.info.has_video() {
            let (w, h) = self.size;
            let child = Command::new("ffmpeg")
                .args(["-v", "quiet", "-re", "-ss", &ss, "-i"])
                .arg(&self.path)
                .args(["-an", "-vf", &format!("scale={w}:{h}"), "-r", "25", "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut c) = child {
                if let Some(mut out) = c.stdout.take() {
                    let (tx, rx) = channel();
                    let n = (w * h * 4) as usize;
                    std::thread::spawn(move || loop {
                        let mut buf = vec![0u8; n];
                        if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                            return;
                        }
                    });
                    self.frames = Some(rx);
                }
                self.children.push(c);
            }
        }
        self.start_pos = at;
        self.started = Some(Instant::now());
    }

    pub fn pause(&mut self) {
        self.paused_at = self.position();
        self.started = None;
        self.stop_children();
    }

    pub fn toggle(&mut self) {
        if self.playing() {
            self.pause()
        } else {
            self.play(self.paused_at)
        }
    }

    pub fn seek(&mut self, frac: f64) {
        let at = frac.clamp(0.0, 1.0) * self.info.duration;
        if self.playing() {
            self.play(at);
        } else {
            self.paused_at = at;
        }
    }

    /// The newest decoded frame, if any arrived.
    pub fn frame(&mut self) -> Option<Vec<u8>> {
        let rx = self.frames.as_ref()?;
        let mut last = None;
        while let Ok(f) = rx.try_recv() {
            last = Some(f);
        }
        last
    }

    pub fn stop_if_done(&mut self) -> bool {
        if self.finished() {
            self.paused_at = self.info.duration;
            self.started = None;
            self.stop_children();
            return true;
        }
        false
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop_children();
    }
}

/// A still frame for the poster (first second of the video).
pub fn poster(path: &Path, size: (u32, u32)) -> Option<Vec<u8>> {
    let (w, h) = size;
    if w == 0 {
        return None;
    }
    let out = Command::new("ffmpeg")
        .args(["-v", "quiet", "-ss", "0.5", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &format!("scale={w}:{h}"), "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.stdout.len() == (w * h * 4) as usize).then_some(out.stdout)
}

pub fn pdf_pages(path: &Path) -> usize {
    Command::new("pdfinfo")
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find_map(|l| l.strip_prefix("Pages:").and_then(|v| v.trim().parse().ok()))
        })
        .unwrap_or(0)
}

/// Render one page (1-based) as RGBA.
pub fn pdf_page(path: &Path, page: usize) -> Option<(u32, u32, Vec<u8>)> {
    let n = page.to_string();
    let out = Command::new("pdftoppm")
        .args(["-f", &n, "-l", &n, "-png", "-scale-to", "1400", "-singlefile"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let img = image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_and_clock() {
        let p = parse_probe("width=1920\nheight=1080\nduration=12.5\n");
        assert_eq!(p, Probe { duration: 12.5, width: 1920, height: 1080 });
        assert!(p.has_video());
        let a = parse_probe("duration=185.2\n");
        assert!(!a.has_video());
        assert_eq!(clock(a.duration), "3:05");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(fit(1920, 1080, 960), (960, 540));
        assert_eq!(fit(101, 51, 960), (100, 50));
        assert_eq!(fit(0, 10, 960), (0, 0));
    }

    #[test]
    fn player_position_and_seek() {
        let mut p = Player::new(Path::new("/nonexistent.mp3"), Probe { duration: 100.0, width: 0, height: 0 });
        assert!(!p.playing());
        p.seek(0.25);
        assert_eq!(p.position(), 25.0);
        p.seek(2.0);
        assert_eq!(p.position(), 100.0);
        assert!(!p.finished());
    }

    #[test]
    fn real_media_when_tools_exist() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("aqua-media-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let v = d.join("clip.mp4");
        let ok = Command::new("ffmpeg")
            .args([
                "-v",
                "quiet",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=25:duration=2",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&v)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let info = probe(&v).unwrap();
        assert_eq!((info.width, info.height), (320, 240));
        assert!((info.duration - 2.0).abs() < 0.2);
        let frame = poster(&v, (160, 120)).unwrap();
        assert_eq!(frame.len(), 160 * 120 * 4);
        let mut pl = Player::new(&v, info);
        pl.play(0.0);
        let t = Instant::now();
        let mut got = None;
        while got.is_none() && t.elapsed().as_secs() < 5 {
            got = pl.frame();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(got.map(|f| f.len()), Some((pl.size.0 * pl.size.1 * 4) as usize));
        pl.pause();
        assert!(!pl.playing());
        assert!(pl.position() > 0.0);
    }
}
