//! End-to-end test of the XEmbed bridge on a private Xvfb server (skipped when `Xvfb` is
//! not installed).
use aqua_tray::xembed::{Host, Sink};
use aqua_tray::Icon;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

#[derive(Default)]
struct Log(Vec<String>);

#[derive(Clone, Default)]
struct Rec(Arc<Mutex<Log>>);

impl Sink for Rec {
    fn added(&mut self, win: u32, id: &str, title: &str) {
        self.0.lock().unwrap().0.push(format!("added {win} {id} {title}"));
    }
    fn icon(&mut self, win: u32, icon: &Icon) {
        let px = &icon.rgba[..4];
        self.0.lock().unwrap().0.push(format!("icon {win} {}x{} {:?}", icon.width, icon.height, px));
    }
    fn removed(&mut self, win: u32) {
        self.0.lock().unwrap().0.push(format!("removed {win}"));
    }
}

struct Xvfb(Child);
impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn xvfb() -> Option<(Xvfb, String)> {
    for n in 90..110 {
        if std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() {
            continue;
        }
        let child = Command::new("Xvfb")
            .args([
                format!(":{n}"),
                "-screen".into(),
                "0".into(),
                "640x480x24".into(),
                "-nolisten".into(),
                "tcp".into(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let x = Xvfb(child);
        let d = format!(":{n}");
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if x11rb::connect(Some(&d)).is_ok() {
                return Some((x, d));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    None
}

fn wait_for(log: &Rec, host: &mut Host<Rec>, pat: &str) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        host.pump().unwrap();
        if log.0.lock().unwrap().0.iter().any(|l| l.starts_with(pat)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

#[test]
fn docks_paints_and_undocks_an_icon() {
    let Some((_server, display)) = xvfb() else {
        eprintln!("Xvfb not available, skipping");
        return;
    };
    let log = Rec::default();
    let l2 = log.clone();
    let mut host = Host::new(&display, move |_, _| l2).expect("tray host");

    // A tray client: find the manager, create an icon window, ask to dock.
    let (c, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = c.setup().roots[screen].root;
    let sel = c.intern_atom(false, b"_NET_SYSTEM_TRAY_S0").unwrap().reply().unwrap().atom;
    let owner = c.get_selection_owner(sel).unwrap().reply().unwrap().owner;
    assert_ne!(owner, x11rb::NONE, "the bridge owns the tray selection");
    let opcode = c.intern_atom(false, b"_NET_SYSTEM_TRAY_OPCODE").unwrap().reply().unwrap().atom;
    let icon = c.generate_id().unwrap();
    c.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        icon,
        root,
        0,
        0,
        16,
        16,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().background_pixel(0xff0000).event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE),
    )
    .unwrap();
    c.change_property8(PropMode::REPLACE, icon, AtomEnum::WM_CLASS, AtomEnum::STRING, b"testtray\0TestTray\0").unwrap();
    c.change_property8(PropMode::REPLACE, icon, AtomEnum::WM_NAME, AtomEnum::STRING, b"Test tray").unwrap();
    let ev = ClientMessageEvent::new(32, owner, opcode, [0, 0, icon, 0, 0]);
    c.send_event(false, owner, EventMask::NO_EVENT, ev).unwrap();
    c.flush().unwrap();

    assert!(wait_for(&log, &mut host, &format!("added {icon} testtray Test tray")), "{:?}", log.0.lock().unwrap().0);
    assert!(wait_for(&log, &mut host, &format!("icon {icon} 32x32 [255, 0, 0, 255]")), "{:?}", log.0.lock().unwrap().0);

    // A click arrives at the icon as a button press.
    aqua_tray::xembed::send_click(&host.conn, icon, (32, 32), aqua_tray::xembed::Click::Secondary).unwrap();
    let t = Instant::now();
    let mut pressed = None;
    while t.elapsed() < Duration::from_secs(3) && pressed.is_none() {
        while let Some(e) = c.poll_for_event().unwrap() {
            if let x11rb::protocol::Event::ButtonPress(b) = e {
                pressed = Some(b.detail);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(pressed, Some(3));

    c.destroy_window(icon).unwrap();
    c.flush().unwrap();
    assert!(wait_for(&log, &mut host, &format!("removed {icon}")), "{:?}", log.0.lock().unwrap().0);
}
