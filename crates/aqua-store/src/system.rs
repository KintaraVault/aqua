use crate::model::Origin;
use crate::runner::Runner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Env {
    pub distro_id: String,
    pub distro_name: String,
    pub distro_like: Vec<String>,
    pub native: Option<Origin>,
    pub flatpak: bool,
    pub pkexec: bool,
    pub aur_helper: Option<String>,
    pub checkupdates: bool,
    pub git: bool,
    pub makepkg: bool,
    pub root: bool,
}

pub fn parse_os_release(text: &str) -> (String, String, Vec<String>) {
    let mut id = String::new();
    let mut name = String::new();
    let mut pretty = String::new();
    let mut like = vec![];
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        match k.trim() {
            "ID" => id = v.to_lowercase(),
            "NAME" => name = v,
            "PRETTY_NAME" => pretty = v,
            "ID_LIKE" => like = v.split_whitespace().map(|s| s.to_lowercase()).collect(),
            _ => {}
        }
    }
    (id, if name.is_empty() { pretty } else { name }, like)
}

pub fn family(id: &str, like: &[String]) -> Option<Origin> {
    let all: Vec<&str> = std::iter::once(id).chain(like.iter().map(String::as_str)).collect();
    let any = |names: &[&str]| all.iter().any(|x| names.contains(x));
    if any(&["arch", "manjaro", "endeavouros", "cachyos", "garuda", "artix", "archarm"]) {
        Some(Origin::Pacman)
    } else if any(&["fedora", "rhel", "centos", "rocky", "almalinux", "nobara", "ultramarine", "amzn"]) {
        Some(Origin::Dnf)
    } else if any(&["debian", "ubuntu", "linuxmint", "pop", "elementary", "zorin", "kali", "raspbian", "neon"]) {
        Some(Origin::Apt)
    } else {
        None
    }
}

impl Env {
    pub fn detect(run: &dyn Runner) -> Env {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        Env::from_parts(&text, run)
    }

    pub fn from_parts(os_release: &str, run: &dyn Runner) -> Env {
        let (id, name, like) = parse_os_release(os_release);
        let mut native = family(&id, &like);
        if native.is_none() || !native.is_some_and(|n| run.have(native_program(n))) {
            native = [Origin::Pacman, Origin::Dnf, Origin::Apt].into_iter().find(|o| run.have(native_program(*o)));
        }
        Env {
            distro_id: id,
            distro_name: name,
            distro_like: like,
            native,
            flatpak: run.have("flatpak"),
            pkexec: run.have("pkexec"),
            aur_helper: crate::aur::HELPERS.iter().find(|h| run.have(h)).map(|s| s.to_string()),
            checkupdates: run.have("checkupdates"),
            git: run.have("git"),
            makepkg: run.have("makepkg"),
            root: crate::runner::is_root(),
        }
    }

    pub fn native_label(&self) -> String {
        if !self.distro_name.is_empty() {
            return self.distro_name.clone();
        }
        self.native.map(|o| o.label().to_string()).unwrap_or_default()
    }

    pub fn has_aur(&self) -> bool {
        self.native == Some(Origin::Pacman)
    }

    pub fn can_elevate(&self) -> bool {
        self.root || self.pkexec
    }
}

pub fn native_program(o: Origin) -> &'static str {
    match o {
        Origin::Pacman | Origin::Aur => "pacman",
        Origin::Dnf => "dnf",
        Origin::Apt => "apt-get",
        Origin::Flatpak => "flatpak",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;

    #[test]
    fn os_release() {
        let (id, name, like) = parse_os_release("NAME=\"EndeavourOS\"\nID=\"endeavouros\"\nID_LIKE=\"arch\"\n");
        assert_eq!((id.as_str(), name.as_str()), ("endeavouros", "EndeavourOS"));
        assert_eq!(family(&id, &like), Some(Origin::Pacman));
        assert_eq!(family("ubuntu", &["debian".into()]), Some(Origin::Apt));
        assert_eq!(family("fedora", &[]), Some(Origin::Dnf));
        assert_eq!(family("nixos", &[]), None);
    }

    #[test]
    fn detection_prefers_available_tools() {
        let r = FakeRunner::with(&["dnf", "flatpak", "pkexec"]);
        let e = Env::from_parts("ID=fedora\nNAME=Fedora Linux\n", &r);
        assert_eq!(e.native, Some(Origin::Dnf));
        assert!(e.flatpak && e.pkexec);
        assert!(!e.has_aur());
        let r = FakeRunner::with(&["pacman", "paru"]);
        let e = Env::from_parts("ID=unknown\n", &r);
        assert_eq!(e.native, Some(Origin::Pacman));
        assert_eq!(e.aur_helper.as_deref(), Some("paru"));
        assert!(!e.flatpak);
    }
}
