//! PipeWire video sources for the `org.freedesktop.impl.portal.ScreenCast` backend.
//!
//! Each [`Cast`] owns a PipeWire stream (a `Video/Source` node driven by Aqua) on its own
//! thread. The compositor renders the shared output offscreen and hands frames over with
//! [`Cast::push`]; xdg-desktop-portal gives the requesting app access to the node id.
//! Frames are only wanted while a consumer is connected ([`Cast::streaming`]).
use pipewire as pw;
use pw::spa;
use spa::pod::{ChoiceValue, Object, Pod, Property, Value};
use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle, SpaTypes};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Upper bound of the frame rate offered to consumers.
pub const MAX_FPS: u32 = 60;

enum Msg {
    Frame(Vec<u8>),
    Stop,
}

#[derive(Default)]
struct Shared {
    streaming: AtomicBool,
    ended: AtomicBool,
}

/// A running screen cast stream.
pub struct Cast {
    tx: pw::channel::Sender<Msg>,
    node: u32,
    width: u32,
    height: u32,
    shared: Arc<Shared>,
    last: Option<Instant>,
}

/// Pixel layouts we can produce from the compositor's RGBA read-back.
pub const FORMATS: [spa::param::video::VideoFormat; 4] = [
    spa::param::video::VideoFormat::BGRx,
    spa::param::video::VideoFormat::RGBx,
    spa::param::video::VideoFormat::BGRA,
    spa::param::video::VideoFormat::RGBA,
];

/// Does `format` want blue first (so RGBA input must be swizzled)?
pub fn blue_first(format: spa::param::video::VideoFormat) -> bool {
    matches!(format, spa::param::video::VideoFormat::BGRx | spa::param::video::VideoFormat::BGRA)
}

/// Copy a tightly packed RGBA frame into a buffer with `stride`, swapping R/B if asked and
/// forcing opaque alpha.
pub fn copy_frame(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize, swap: bool) -> bool {
    let row = width * 4;
    if src.len() < row * height || stride < row || dst.len() < stride * (height - 1) + row {
        return false;
    }
    for y in 0..height {
        let s = &src[y * row..(y + 1) * row];
        let d = &mut dst[y * stride..y * stride + row];
        if swap {
            for (d, s) in d.as_chunks_mut::<4>().0.iter_mut().zip(s.as_chunks::<4>().0) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        } else {
            d.copy_from_slice(s);
            for p in d.as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
        }
    }
    true
}

fn serialize(obj: Object) -> Result<Vec<u8>, String> {
    pw::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .map(|(c, _)| c.into_inner())
        .map_err(|e| format!("{e:?}"))
}

fn prop(key: u32, value: Value) -> Property {
    Property::new(key, value)
}

/// `EnumFormat`: raw video of the output size in one of [`FORMATS`], variable frame rate.
pub fn format_pod(width: u32, height: u32) -> Result<Vec<u8>, String> {
    use spa::param::format::{FormatProperties as F, MediaSubtype, MediaType};
    let formats: Vec<Id> = FORMATS.iter().map(|f| Id(f.as_raw())).collect();
    serialize(Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: vec![
            prop(F::MediaType.as_raw(), Value::Id(Id(MediaType::Video.as_raw()))),
            prop(F::MediaSubtype.as_raw(), Value::Id(Id(MediaSubtype::Raw.as_raw()))),
            prop(
                F::VideoFormat.as_raw(),
                Value::Choice(ChoiceValue::Id(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum { default: formats[0], alternatives: formats.clone() },
                ))),
            ),
            prop(F::VideoSize.as_raw(), Value::Rectangle(Rectangle { width, height })),
            prop(F::VideoFramerate.as_raw(), Value::Fraction(Fraction { num: 0, denom: 1 })),
            prop(
                F::VideoMaxFramerate.as_raw(),
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Fraction { num: 30, denom: 1 },
                        min: Fraction { num: 1, denom: 1 },
                        max: Fraction { num: MAX_FPS, denom: 1 },
                    },
                ))),
            ),
        ],
    })
}

/// `Buffers`: memfd/memptr buffers of one tightly packed frame.
pub fn buffers_pod(width: u32, height: u32) -> Result<Vec<u8>, String> {
    use spa::sys;
    let stride = width as i32 * 4;
    serialize(Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![
            prop(
                sys::SPA_PARAM_BUFFERS_buffers,
                Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range { default: 4, min: 2, max: 8 },
                ))),
            ),
            prop(sys::SPA_PARAM_BUFFERS_blocks, Value::Int(1)),
            prop(sys::SPA_PARAM_BUFFERS_size, Value::Int(stride * height as i32)),
            prop(sys::SPA_PARAM_BUFFERS_stride, Value::Int(stride)),
            prop(sys::SPA_PARAM_BUFFERS_dataType, Value::Int((1 << sys::SPA_DATA_MemFd) | (1 << sys::SPA_DATA_MemPtr))),
        ],
    })
}

impl Cast {
    /// Create the stream and wait (up to 5 s) until PipeWire assigned its node id.
    pub fn start(name: &str, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("empty output".into());
        }
        let (tx, rx) = pw::channel::channel::<Msg>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let shared = Arc::new(Shared::default());
        let sh = shared.clone();
        let name = name.to_string();
        std::thread::Builder::new()
            .name("aqua-screencast".into())
            .spawn(move || {
                let r2 = ready_tx.clone();
                if let Err(e) = run(&name, width, height, rx, ready_tx, sh.clone()) {
                    tracing::warn!("screencast stream failed: {e}");
                    let _ = r2.send(Err(e));
                }
                sh.streaming.store(false, Ordering::Relaxed);
                sh.ended.store(true, Ordering::Relaxed);
            })
            .map_err(|e| e.to_string())?;
        let node =
            ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "PipeWire did not answer".to_string())??;
        Ok(Self { tx, node, width, height, shared, last: None })
    }

    pub fn node_id(&self) -> u32 {
        self.node
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// A consumer is connected and frames are wanted.
    pub fn streaming(&self) -> bool {
        self.shared.streaming.load(Ordering::Relaxed)
    }

    /// The stream died (PipeWire went away or errored).
    pub fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::Relaxed)
    }

    /// Is it time for another frame (rate limit `fps`)?
    pub fn due(&mut self, fps: u32) -> bool {
        if !self.streaming() {
            return false;
        }
        let now = Instant::now();
        let iv = Duration::from_micros(1_000_000 / fps.clamp(1, MAX_FPS) as u64);
        if self.last.is_some_and(|l| now.duration_since(l) < iv) {
            return false;
        }
        self.last = Some(now);
        true
    }

    /// Queue a tightly packed RGBA frame of [`Cast::size`].
    pub fn push(&self, rgba: Vec<u8>) {
        let _ = self.tx.send(Msg::Frame(rgba));
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

struct Data {
    swap: bool,
    reported: bool,
}

fn run(
    name: &str,
    width: u32,
    height: u32,
    rx: pw::channel::Receiver<Msg>,
    ready: std::sync::mpsc::Sender<Result<u32, String>>,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let e = |e: pw::Error| e.to_string();
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(e)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(e)?;
    let core = context.connect_rc(None).map_err(e)?;
    let props = pw::properties::properties! {
        *pw::keys::MEDIA_CLASS => "Video/Source",
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
        *pw::keys::NODE_NAME => name,
        *pw::keys::NODE_DESCRIPTION => "Aqua screen cast",
    };
    let stream = pw::stream::StreamRc::new(core, name, props).map_err(e)?;
    let frame: Rc<RefCell<Option<Vec<u8>>>> = Rc::default();

    let (f2, sh2, ml2) = (frame.clone(), shared.clone(), mainloop.clone());
    let _listener = stream
        .add_local_listener_with_user_data(Data { swap: true, reported: false })
        .state_changed(move |st, data, _old, new| {
            use pw::stream::StreamState as S;
            if !data.reported && st.node_id() != u32::MAX && !matches!(new, S::Error(_) | S::Unconnected) {
                data.reported = true;
                let _ = ready.send(Ok(st.node_id()));
            }
            sh2.streaming.store(matches!(new, S::Streaming), Ordering::Relaxed);
            match new {
                S::Error(msg) => {
                    if !data.reported {
                        let _ = ready.send(Err(msg.clone()));
                    }
                    tracing::warn!("screencast: {msg}");
                    ml2.quit();
                }
                S::Unconnected => ml2.quit(),
                _ => {}
            }
        })
        .param_changed(move |st, data, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            data.swap = blue_first(info.format());
            tracing::info!("screencast: negotiated {:?} {}x{}", info.format(), info.size().width, info.size().height);
            if let Ok(b) = buffers_pod(width, height) {
                if let Some(pod) = Pod::from_bytes(&b) {
                    let _ = st.update_params(&mut [pod]);
                }
            }
        })
        .process(move |st, data| {
            let Some(rgba) = f2.borrow_mut().take() else { return };
            let Some(mut buf) = st.dequeue_buffer() else { return };
            let datas = buf.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let stride = width as usize * 4;
            let ok =
                d.data().is_some_and(|dst| copy_frame(&rgba, dst, width as usize, height as usize, stride, data.swap));
            let chunk = d.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = if ok { (stride * height as usize) as u32 } else { 0 };
        })
        .register()
        .map_err(e)?;

    let fmt = format_pod(width, height)?;
    let mut params = [Pod::from_bytes(&fmt).ok_or("bad format pod")?];
    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            // PipeWire allocates (MAP_BUFFERS maps them); ALLOC_BUFFERS would mean we allocate.
            pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(e)?;

    let (st3, ml3, f3) = (stream.clone(), mainloop.clone(), frame.clone());
    let _rx = rx.attach(mainloop.loop_(), move |m| match m {
        Msg::Frame(rgba) => {
            *f3.borrow_mut() = Some(rgba);
            if st3.is_driving() {
                let _ = st3.trigger_process();
            }
        }
        Msg::Stop => ml3.quit(),
    });
    mainloop.run();
    let _ = stream.disconnect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_copy_swizzles_and_pads() {
        let src = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = [0u8; 12];
        assert!(copy_frame(&src, &mut dst, 1, 2, 6, true));
        assert_eq!(dst, [3, 2, 1, 255, 0, 0, 7, 6, 5, 255, 0, 0]);
        let mut dst = [0u8; 8];
        assert!(copy_frame(&src, &mut dst, 2, 1, 8, false));
        assert_eq!(dst, [1, 2, 3, 255, 5, 6, 7, 255]);
        assert!(!copy_frame(&src, &mut [0u8; 4], 2, 1, 8, false), "too small destination");
    }

    #[test]
    fn pods_serialize() {
        let f = format_pod(1920, 1080).unwrap();
        assert!(Pod::from_bytes(&f).is_some());
        let b = buffers_pod(1920, 1080).unwrap();
        assert!(Pod::from_bytes(&b).is_some());
        assert!(blue_first(spa::param::video::VideoFormat::BGRx));
        assert!(!blue_first(spa::param::video::VideoFormat::RGBA));
    }
}
