use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Flatpak,
    Pacman,
    Aur,
    Dnf,
    Apt,
}

impl Origin {
    pub const ALL: [Origin; 5] = [Origin::Flatpak, Origin::Pacman, Origin::Aur, Origin::Dnf, Origin::Apt];

    pub fn id(self) -> &'static str {
        match self {
            Origin::Flatpak => "flatpak",
            Origin::Pacman => "pacman",
            Origin::Aur => "aur",
            Origin::Dnf => "dnf",
            Origin::Apt => "apt",
        }
    }

    pub fn parse(s: &str) -> Option<Origin> {
        Origin::ALL.into_iter().find(|o| o.id() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Origin::Flatpak => "Flathub",
            Origin::Pacman => "Arch Linux",
            Origin::Aur => "AUR",
            Origin::Dnf => "Fedora",
            Origin::Apt => "Debian",
        }
    }

    pub fn is_native(self) -> bool {
        matches!(self, Origin::Pacman | Origin::Dnf | Origin::Apt)
    }

    pub fn sandboxed(self) -> bool {
        self == Origin::Flatpak
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    User,
    System,
}

impl Scope {
    pub fn flag(self) -> &'static str {
        match self {
            Scope::User => "--user",
            Scope::System => "--system",
        }
    }

    pub fn parse(s: &str) -> Scope {
        if s.trim().eq_ignore_ascii_case("system") {
            Scope::System
        } else {
            Scope::User
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Icon {
    #[default]
    None,
    Url(String),
    Path(PathBuf),
    Named(String),
}

impl Icon {
    pub fn is_none(&self) -> bool {
        matches!(self, Icon::None)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub origin: Option<Origin>,
    pub name: String,
    pub appstream_id: String,
    pub title: String,
    pub summary: String,
    pub version: String,
    pub repo: String,
    pub installed: bool,
    pub installed_version: String,
    pub update_version: String,
    pub size: Option<u64>,
    pub download_size: Option<u64>,
    pub icon: Icon,
    pub developer: String,
    pub categories: Vec<String>,
    pub verified: bool,
    pub installs: Option<u64>,
    pub license: String,
    pub homepage: String,
    pub scope: Option<Scope>,
    pub branch: String,
    pub desktop_id: String,
    pub is_app: bool,
    pub keywords: Vec<String>,
    pub popularity: f64,
    pub votes: u64,
    pub out_of_date: bool,
}

impl Package {
    pub fn new(origin: Origin, name: &str) -> Package {
        Package { origin: Some(origin), name: name.to_string(), title: name.to_string(), ..Default::default() }
    }

    pub fn origin(&self) -> Origin {
        self.origin.unwrap_or(Origin::Flatpak)
    }

    pub fn key(&self) -> String {
        key_of(self.origin(), &self.name)
    }

    pub fn display_name(&self) -> &str {
        if self.title.is_empty() {
            &self.name
        } else {
            &self.title
        }
    }

    pub fn has_update(&self) -> bool {
        self.installed && !self.update_version.is_empty()
    }

    pub fn source_label(&self) -> String {
        let o = self.origin();
        match o {
            Origin::Flatpak if !self.repo.is_empty() && self.repo != "flathub" => self.repo.clone(),
            Origin::Pacman | Origin::Dnf | Origin::Apt if !self.repo.is_empty() => {
                format!("{} ({})", o.label(), self.repo)
            }
            _ => o.label().to_string(),
        }
    }

    pub fn merge_from(&mut self, o: &Package) {
        macro_rules! fill {
            ($($f:ident),*) => {$( if self.$f.is_empty() { self.$f = o.$f.clone(); } )*};
        }
        fill!(appstream_id, summary, version, repo, developer, license, homepage, branch, desktop_id);
        if (self.title.is_empty() || self.title == self.name) && !o.title.is_empty() {
            self.title = o.title.clone();
        }
        if self.icon.is_none() {
            self.icon = o.icon.clone();
        }
        if self.categories.is_empty() {
            self.categories = o.categories.clone();
        }
        if self.keywords.is_empty() {
            self.keywords = o.keywords.clone();
        }
        self.size = self.size.or(o.size);
        self.download_size = self.download_size.or(o.download_size);
        self.installs = self.installs.or(o.installs);
        self.scope = self.scope.or(o.scope);
        self.verified |= o.verified;
        self.is_app |= o.is_app;
        if self.popularity == 0.0 {
            self.popularity = o.popularity;
        }
    }
}

pub fn key_of(origin: Origin, name: &str) -> String {
    format!("{}:{}", origin.id(), name)
}

pub fn parse_key(key: &str) -> Option<(Origin, &str)> {
    let (o, n) = key.split_once(':')?;
    Some((Origin::parse(o)?, n))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Para(String),
    Bullet(String),
    Number(usize, String),
}

impl Block {
    pub fn text(&self) -> &str {
        match self {
            Block::Para(s) | Block::Bullet(s) | Block::Number(_, s) => s,
        }
    }
}

pub fn blocks_to_text(b: &[Block]) -> String {
    let mut out = String::new();
    for (i, x) in b.iter().enumerate() {
        let line = match x {
            Block::Para(s) => s.clone(),
            Block::Bullet(s) => format!("• {s}"),
            Block::Number(n, s) => format!("{n}. {s}"),
        };
        if i > 0 {
            let prev_list = matches!(b[i - 1], Block::Bullet(_) | Block::Number(..));
            let list = !matches!(x, Block::Para(_));
            out.push_str(if prev_list && list { "\n" } else { "\n\n" });
        }
        out.push_str(&line);
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screenshot {
    pub thumb: String,
    pub full: String,
    pub caption: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub version: String,
    pub timestamp: Option<i64>,
    pub notes: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permission {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub risky: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub kind: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub pkg: Package,
    pub description: Vec<Block>,
    pub screenshots: Vec<Screenshot>,
    pub releases: Vec<Release>,
    pub links: Vec<Link>,
    pub age: Option<u8>,
    pub content: Vec<String>,
    pub permissions: Vec<Permission>,
    pub runtime: String,
    pub depends: Vec<String>,
    pub brand_light: String,
    pub brand_dark: String,
    pub maintainer: String,
    pub packager: String,
    pub updated: Option<i64>,
    pub languages: Vec<String>,
}

impl Details {
    pub fn link(&self, kind: &str) -> Option<&str> {
        self.links.iter().find(|l| l.kind == kind).map(|l| l.url.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub origin: Option<Origin>,
    pub name: String,
    pub from: String,
    pub to: String,
    pub repo: String,
    pub download_size: Option<u64>,
    pub scope: Option<Scope>,
    pub is_app: bool,
}

impl Update {
    pub fn origin(&self) -> Origin {
        self.origin.unwrap_or(Origin::Flatpak)
    }

    pub fn key(&self) -> String {
        key_of(self.origin(), &self.name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ratings {
    pub stars: [u64; 5],
}

impl Ratings {
    pub fn total(&self) -> u64 {
        self.stars.iter().sum()
    }

    pub fn average(&self) -> f32 {
        let t = self.total();
        if t == 0 {
            return 0.0;
        }
        let s: u64 = self.stars.iter().enumerate().map(|(i, n)| (i as u64 + 1) * n).sum();
        s as f32 / t as f32
    }

    pub fn fraction(&self, star: usize) -> f32 {
        let t = self.total().max(1) as f32;
        self.stars.get(star.saturating_sub(1)).copied().unwrap_or(0) as f32 / t
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Review {
    pub id: i64,
    pub author: String,
    pub summary: String,
    pub body: String,
    pub rating: u8,
    pub date: i64,
    pub version: String,
    pub distro: String,
    pub karma_up: i64,
    pub karma_down: i64,
    pub mine: bool,
}

impl Review {
    pub fn stars(&self) -> u8 {
        ((self.rating as u32 + 10) / 20).clamp(0, 5) as u8
    }
}
