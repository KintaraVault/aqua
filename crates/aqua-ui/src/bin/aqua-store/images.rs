use aqua_store::http::Http;
use aqua_store::model::Icon;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Debug)]
pub enum Req {
    Icon { key: String, id: String, name: String, icon: Icon },
    Shot { gen: u64, index: usize, url: String, max_h: u32 },
    Art { key: String, url: String },
    File { tag: String, path: std::path::PathBuf, size: u32 },
}

pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub premul: bool,
}

type Sink = Arc<dyn Fn(Req, Option<Pixels>) + Send + Sync>;

pub struct Loader {
    queue: Arc<(Mutex<Vec<Req>>, Condvar)>,
}

fn provider() -> aqua_icons::IconProvider {
    let cfg = aqua_config::Config::load();
    let fonts = Arc::new(aqua_gfx::Fonts::load(&cfg.font_dir()));
    let mut p = aqua_icons::IconProvider::new(cfg.icon_cache(), fonts, false);
    p.set_look(aqua_icons::look::Look {
        style: aqua_icons::look::Style::from_config(&cfg.icon_style, cfg.dark),
        dark: cfg.dark,
        tint: cfg.accent_rgb(),
        glass: cfg.icon_glass,
    });
    p.set_policy(aqua_icons::equiv::Policy { mode: "off".into(), apps: vec![], owners: vec![] });
    p
}

pub fn scale_to(img: image::DynamicImage, max_w: u32, max_h: u32) -> Pixels {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let k = (max_w as f32 / w as f32).min(max_h as f32 / h as f32).min(1.0);
    let img = if k < 1.0 {
        img.resize(
            (w as f32 * k).round().max(1.0) as u32,
            (h as f32 * k).round().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Pixels { w: rgba.width(), h: rgba.height(), rgba: rgba.into_raw(), premul: false }
}

fn decode(path: &std::path::Path) -> Option<image::DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    image::load_from_memory(&bytes).ok()
}

fn icon_source(http: &Http, icon: &Icon) -> Option<String> {
    match icon {
        Icon::None => None,
        Icon::Named(n) => Some(n.clone()),
        Icon::Path(p) => Some(p.to_string_lossy().into_owned()),
        Icon::Url(u) => http.file(u).ok().map(|p| p.to_string_lossy().into_owned()),
    }
}

fn work(req: &Req, http: &Http, prov: &mut Option<aqua_icons::IconProvider>) -> Option<Pixels> {
    match req {
        Req::Icon { id, name, icon, .. } => {
            let src = icon_source(http, icon)?;
            let p = prov.get_or_insert_with(provider);
            let pm = p.get(&aqua_icons::IconRequest { id: id.clone(), name: name.clone(), icon: src }, 128);
            if pm.width() == 0 {
                return None;
            }
            Some(Pixels { w: pm.width(), h: pm.height(), rgba: pm.data().to_vec(), premul: true })
        }
        Req::Shot { url, max_h, .. } => {
            let path = http.file(url).ok()?;
            Some(scale_to(decode(&path)?, max_h * 3, *max_h))
        }
        Req::Art { url, .. } => {
            let path = http.file(url).ok()?;
            Some(scale_to(decode(&path)?, 1100, 760))
        }
        Req::File { path, size, .. } => Some(scale_to(decode(path)?, *size, *size)),
    }
}

impl Loader {
    pub fn spawn(
        http: Arc<Http>,
        threads: usize,
        sink: impl Fn(Req, Option<Pixels>) + Send + Sync + 'static,
    ) -> Loader {
        let queue: Arc<(Mutex<Vec<Req>>, Condvar)> = Arc::default();
        let sink: Sink = Arc::new(sink);
        for n in 0..threads.max(1) {
            let q = queue.clone();
            let sink = sink.clone();
            let http = http.clone();
            let _ = std::thread::Builder::new().name(format!("store-img-{n}")).spawn(move || {
                let mut prov = None;
                loop {
                    let req = {
                        let (m, cv) = &*q;
                        let mut g = m.lock().unwrap();
                        while g.is_empty() {
                            g = cv.wait(g).unwrap();
                        }
                        g.remove(0)
                    };
                    let px = work(&req, &http, &mut prov);
                    sink(req, px);
                }
            });
        }
        Loader { queue }
    }

    pub fn push(&self, r: Req) {
        let (m, cv) = &*self.queue;
        m.lock().unwrap().push(r);
        cv.notify_one();
    }

    pub fn push_front(&self, r: Req) {
        let (m, cv) = &*self.queue;
        m.lock().unwrap().insert(0, r);
        cv.notify_one();
    }

    pub fn drop_shots(&self, keep_gen: u64) {
        let (m, _) = &*self.queue;
        m.lock().unwrap().retain(|r| !matches!(r, Req::Shot { gen, .. } if *gen != keep_gen));
    }
}

pub fn to_image(p: &Pixels) -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(p.w, p.h);
    buf.make_mut_bytes().copy_from_slice(&p.rgba);
    if p.premul {
        slint::Image::from_rgba8_premultiplied(buf)
    } else {
        slint::Image::from_rgba8(buf)
    }
}
