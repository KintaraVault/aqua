//! Screen recording: frames rendered by the compositor are cropped and piped as raw
//! RGBA into `ffmpeg`, which encodes an MP4 (H.264 when available) in the background.
//!
//! Frames are only produced when the screen changes; ffmpeg timestamps them with the
//! wall clock and repeats them to a constant 30 fps, so a static screen costs nothing.
//! A bounded queue drops frames instead of stalling the compositor when the encoder
//! can't keep up.
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const FPS: u32 = 30;
const QUEUE: usize = 6;

type Finished = Result<(String, Option<aqua_gfx::Pixmap>), String>;
static FINISHED: Mutex<Vec<Finished>> = Mutex::new(Vec::new());

/// Recordings finalised since the last call (path + thumbnail, or an error).
pub fn finished() -> Vec<Finished> {
    std::mem::take(&mut *FINISHED.lock().unwrap_or_else(|e| e.into_inner()))
}

struct Caps {
    video: Vec<&'static str>,
    audio_in: Option<&'static str>,
}

/// What the installed ffmpeg can do (probed once).
fn caps() -> Option<&'static Caps> {
    static C: OnceLock<Option<Caps>> = OnceLock::new();
    C.get_or_init(|| {
        let run = |a: &str| {
            Command::new("ffmpeg")
                .args(["-hide_banner", a])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        let enc = run("-encoders")?;
        let dev = run("-devices").unwrap_or_default();
        let has = |name: &str| enc.lines().any(|l| l.split_whitespace().nth(1) == Some(name));
        let video = if has("libx264") {
            vec!["-c:v", "libx264", "-preset", "superfast", "-crf", "21"]
        } else if has("libopenh264") {
            vec!["-c:v", "libopenh264", "-b:v", "8M"]
        } else {
            vec!["-c:v", "mpeg4", "-q:v", "3"]
        };
        let input = |name: &str| {
            dev.lines().any(|l| {
                l.split_whitespace().next().is_some_and(|f| f.contains('D'))
                    && l.split_whitespace().nth(1).is_some_and(|n| n.split(',').any(|n| n == name))
            })
        };
        let audio_in = if input("pulse") {
            Some("pulse")
        } else if input("alsa") {
            Some("alsa")
        } else {
            None
        };
        Some(Caps { video, audio_in })
    })
    .as_ref()
}

pub struct Recorder {
    /// Physical crop of the output frame: x, y, w, h (even sizes).
    pub crop: (usize, usize, usize, usize),
    tx: Option<SyncSender<Vec<u8>>>,
    last: Option<Instant>,
    /// The screen changed after the last grab (another frame is wanted).
    pending: bool,
    err: Arc<Mutex<Option<String>>>,
}

impl Recorder {
    pub fn start(path: String, crop: (usize, usize, usize, usize), mic: bool) -> Result<Self, String> {
        let caps = caps().ok_or("ffmpeg is not installed")?;
        let (_, _, w, h) = crop;
        let log = std::env::temp_dir().join(format!("aqua-record-{}.log", std::process::id()));
        let logf = std::fs::File::create(&log).map_err(|e| e.to_string())?;
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        cmd.args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-video_size",
            &format!("{w}x{h}"),
            "-use_wallclock_as_timestamps",
            "1",
            "-thread_queue_size",
            "64",
            "-i",
            "pipe:0",
        ]);
        let mic = mic && caps.audio_in.is_some();
        if mic {
            cmd.args(["-f", caps.audio_in.unwrap_or("pulse"), "-thread_queue_size", "1024", "-i", "default"]);
        }
        cmd.args(&caps.video);
        cmd.args(["-pix_fmt", "yuv420p", "-fps_mode", "cfr", "-r", &FPS.to_string()]);
        if mic {
            cmd.args(["-c:a", "aac", "-b:a", "160k"]);
        }
        cmd.args(["-movflags", "+faststart", &path]);
        // The encoder runs at a lower priority: it must never starve the compositor
        // (and the apps being recorded) of CPU.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                Ok(())
            });
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(logf))
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
        let err = Arc::new(Mutex::new(None));
        let err2 = err.clone();
        std::thread::Builder::new()
            .name("aqua-record".into())
            .spawn(move || {
                let mut last: Option<Vec<u8>> = None;
                let mut broken = false;
                for f in rx.iter() {
                    if stdin.write_all(&f).is_err() {
                        broken = true;
                        break;
                    }
                    last = Some(f);
                }
                if broken {
                    *err2.lock().unwrap_or_else(|e| e.into_inner()) = Some(tail(&log));
                } else if let Some(f) = &last {
                    let _ = stdin.write_all(f);
                }
                drop(stdin);
                if mic {
                    std::thread::sleep(Duration::from_millis(250));
                    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
                }
                let t0 = Instant::now();
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(s)) => break Some(s),
                        Ok(None) if t0.elapsed() < Duration::from_secs(30) => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        _ => {
                            let _ = child.kill();
                            break child.wait().ok();
                        }
                    }
                };
                let ok = std::path::Path::new(&path).metadata().map(|m| m.len() > 0).unwrap_or(false)
                    && status.is_some_and(|s| s.success() || mic);
                let res = if ok {
                    let thumb = last.and_then(|f| {
                        let img = image::RgbaImage::from_raw(w as u32, h as u32, f)?;
                        let t = image::DynamicImage::ImageRgba8(img).thumbnail(480, 480).to_rgba8();
                        aqua_gfx::from_rgba(t.width(), t.height(), t.as_raw())
                    });
                    Ok((path, thumb))
                } else {
                    let _ = std::fs::remove_file(&path);
                    Err(tail(&log))
                };
                let _ = std::fs::remove_file(&log);
                FINISHED.lock().unwrap_or_else(|e| e.into_inner()).push(res);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { crop, tx: Some(tx), last: None, pending: false, err })
    }

    fn interval() -> Duration {
        Duration::from_micros(1_000_000 / FPS as u64)
    }

    /// Should the frame just rendered be captured? (≤ 30 fps)
    pub fn due(&mut self) -> bool {
        let now = Instant::now();
        if self.last.is_some_and(|l| now.duration_since(l) < Self::interval()) {
            self.pending = true;
            return false;
        }
        self.last = Some(now);
        self.pending = false;
        true
    }

    /// A change was skipped by the rate limit and its time has come: render again.
    pub fn wants_frame(&self) -> bool {
        self.pending && self.last.is_none_or(|l| l.elapsed() >= Self::interval())
    }

    pub fn push(&mut self, frame: Vec<u8>) {
        if let Some(tx) = &self.tx {
            match tx.try_send(frame) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => self.tx = None,
            }
        }
    }

    pub fn failed(&self) -> Option<String> {
        self.err.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Finish the file (in the background).
    pub fn stop(mut self) {
        self.tx = None;
    }
}

fn tail(log: &std::path::Path) -> String {
    let s = std::fs::read_to_string(log).unwrap_or_default();
    let l = s.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("ffmpeg stopped unexpectedly").to_string();
    l.chars().take(200).collect()
}
