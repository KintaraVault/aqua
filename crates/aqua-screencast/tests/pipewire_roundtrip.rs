//! Cast frames through a private PipeWire daemon (+ WirePlumber) to a consumer stream.
//! Skipped when `pipewire`/`wireplumber` are not installed.
use aqua_screencast::Cast;
use pipewire as pw;
use pw::spa;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Daemons(Vec<Child>, std::path::PathBuf);
impl Drop for Daemons {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn which(b: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(b).is_file())).unwrap_or(false)
}

fn daemons() -> Option<Daemons> {
    if !which("pipewire") || !which("wireplumber") {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("aqua-pw-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    // SAFETY: tests in this file run in one thread (single test).
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        std::env::remove_var("PIPEWIRE_REMOTE");
        std::env::set_var("PIPEWIRE_RUNTIME_DIR", &dir);
    }
    let spawn = |b: &str| {
        Command::new(b)
            .env("XDG_RUNTIME_DIR", &dir)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    };
    let mut d = Daemons(vec![spawn("pipewire")?], dir.clone());
    let t = Instant::now();
    while !dir.join("pipewire-0").exists() {
        if t.elapsed() > Duration::from_secs(5) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    d.0.push(spawn("wireplumber")?);
    std::thread::sleep(Duration::from_millis(800));
    Some(d)
}

#[test]
fn frames_reach_a_consumer() {
    let Some(_d) = daemons() else {
        eprintln!("pipewire/wireplumber not available, skipping");
        return;
    };
    let (w, h) = (64u32, 32u32);
    let mut cast = Cast::start("aqua-test-cast", w, h).expect("stream");
    let node = cast.node_id();
    assert_ne!(node, u32::MAX);

    // Consumer on its own thread.
    let got: Arc<Mutex<Option<(u32, Vec<u8>)>>> = Arc::default();
    let g2 = got.clone();
    std::thread::spawn(move || {
        pw::init();
        let ml = pw::main_loop::MainLoopRc::new(None).unwrap();
        let ctx = pw::context::ContextRc::new(&ml, None).unwrap();
        let core = ctx.connect_rc(None).unwrap();
        let stream = pw::stream::StreamRc::new(
            core,
            "consumer",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
                *pw::keys::TARGET_OBJECT => node.to_string(),
            },
        )
        .unwrap();
        let fmt = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let f2 = fmt.clone();
        let _l = stream
            .add_local_listener_with_user_data(())
            .param_changed(move |_, _, id, p| {
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                if let Some(p) = p {
                    let mut i = spa::param::video::VideoInfoRaw::default();
                    if i.parse(p).is_ok() {
                        f2.set(i.format().as_raw());
                    }
                }
            })
            .process(move |s, _| {
                if let Some(mut b) = s.dequeue_buffer() {
                    let d = &mut b.datas_mut()[0];
                    let size = d.chunk().size() as usize;
                    if size > 0 {
                        if let Some(px) = d.data() {
                            *g2.lock().unwrap() = Some((fmt.get(), px[..size].to_vec()));
                        }
                    }
                }
            })
            .register()
            .unwrap();
        let pod = aqua_screencast::format_pod(w, h).unwrap();
        let mut params = [spa::pod::Pod::from_bytes(&pod).unwrap()];
        stream
            .connect(
                spa::utils::Direction::Input,
                Some(node),
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .unwrap();
        ml.run();
    });

    let mut frame = vec![0u8; (w * h * 4) as usize];
    for p in frame.as_chunks_mut::<4>().0 {
        p.copy_from_slice(&[10, 20, 30, 0]);
    }
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) && got.lock().unwrap().is_none() {
        if cast.due(30) {
            cast.push(frame.clone());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (fmt, px) = got.lock().unwrap().clone().expect("consumer received a frame");
    assert_eq!(px.len(), frame.len());
    let expect = if aqua_screencast::blue_first(spa::param::video::VideoFormat::from_raw(fmt)) {
        [30, 20, 10, 255]
    } else {
        [10, 20, 30, 255]
    };
    assert_eq!(&px[..4], &expect);
    assert!(cast.streaming());
}
