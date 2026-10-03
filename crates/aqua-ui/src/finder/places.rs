//! Sidebar places.
use super::*;

impl App {
    pub(super) fn place_y(&self, path: &str) -> f32 {
        self.places.iter().find(|p| p.2 == path).map(|p| p.0).unwrap_or(0.0)
    }

    pub(super) fn places(&mut self) {
        let home = fs::home();
        let mut v: Vec<FPlace> = vec![];
        let mut hdr = String::new();
        let put = |v: &mut Vec<FPlace>, hdr: &mut String, label: &str, path: String, icon: &str, eject: bool| {
            let h = std::mem::take(hdr);
            v.push(FPlace {
                label: label.into(),
                path: path.into(),
                icon: icon.into(),
                header: crate::tr(&h).into(),
                eject,
                dot: Color::default(),
                is_tag: false,
            });
        };
        if let Some(p) = dirs::public_dir().filter(|p| p.is_dir() && p != &home) {
            put(&mut v, &mut hdr, crate::tr("Shared"), p.to_string_lossy().into_owned(), "shared", false);
        }
        put(&mut v, &mut hdr, crate::tr("Recents"), "recents:".into(), "recents", false);
        hdr = "Favorites".into();
        put(&mut v, &mut hdr, crate::tr("Applications"), "apps:".into(), "apps", false);
        for (label, d, icon) in [
            ("Desktop", dirs::desktop_dir(), "desktop"),
            ("Documents", dirs::document_dir(), "docs"),
            ("Downloads", dirs::download_dir(), "downloads"),
            ("Pictures", dirs::picture_dir(), "pictures"),
            ("Music", dirs::audio_dir(), "music"),
            ("Movies", dirs::video_dir().or_else(|| Some(home.join("Videos")).filter(|p| p.is_dir())), "movies"),
        ] {
            let d = d.or_else(|| Some(home.join(label)));
            if let Some(d) = d.filter(|d| d.is_dir() && d != &home) {
                put(&mut v, &mut hdr, crate::tr(label), d.to_string_lossy().into_owned(), icon, false);
            }
        }
        let bm = dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("gtk-3.0/bookmarks");
        for l in std::fs::read_to_string(bm).unwrap_or_default().lines().take(12) {
            let (uri, label) = l.split_once(' ').unwrap_or((l, ""));
            if let Some(p) =
                uri.strip_prefix("file://").map(fs::percent_decode).map(PathBuf::from).filter(|p| p.is_dir())
            {
                if v.iter().any(|x| x.path.as_str() == p.to_string_lossy()) {
                    continue;
                }
                let name = if label.is_empty() {
                    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    label.to_string()
                };
                put(&mut v, &mut hdr, &name, p.to_string_lossy().into_owned(), "folder", false);
            }
        }
        hdr = "Locations".into();
        let user = std::env::var("USER").unwrap_or_else(|_| crate::tr("Home").into());
        put(&mut v, &mut hdr, &user, home.to_string_lossy().into_owned(), "home", false);
        put(&mut v, &mut hdr, &hostname(), "/".into(), "computer", false);
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        for l in mounts.lines() {
            let mut it = l.split_whitespace();
            let (dev, mp) = (it.next().unwrap_or(""), it.next().unwrap_or("").replace("\\040", " "));
            if (mp.starts_with("/media/") || mp.starts_with("/run/media/") || mp.starts_with("/mnt/"))
                && dev.starts_with('/')
            {
                let name = Path::new(&mp).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(mp.clone());
                put(&mut v, &mut hdr, &name, mp.clone(), "disk", true);
            }
        }
        put(&mut v, &mut hdr, crate::tr("Trash"), "trash:".into(), "trash", false);
        hdr = "Tags".into();
        for (name, c) in fs::TAGS {
            let h = std::mem::take(&mut hdr);
            v.push(FPlace {
                label: crate::tr(name).into(),
                path: format!("tag:{name}").into(),
                icon: "".into(),
                header: crate::tr(&h).into(),
                eject: false,
                dot: rgb(c),
                is_tag: true,
            });
        }
        let mut y = 52.0;
        self.places.clear();
        for p in &v {
            if !p.header.is_empty() {
                y += 28.0;
            }
            self.places.push((y, y + 28.0, p.path.to_string()));
            y += 28.0;
        }
        self.ui().global::<F>().set_places(ModelRc::new(VecModel::from(v)));
    }
}
