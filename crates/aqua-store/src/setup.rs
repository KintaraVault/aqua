use crate::flatpak::Flatpak;
use crate::model::{Origin, Scope};
use crate::runner::Cmd;
use crate::system::Env;

#[derive(Clone, Debug, PartialEq)]
pub struct Requirement {
    pub id: &'static str,
    pub title: String,
    pub detail: String,
    pub action: String,
    pub steps: Vec<Cmd>,
    pub terminal: bool,
    pub essential: bool,
}

pub fn native_install(native: Origin, pkgs: &[&str]) -> Cmd {
    let pk: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
    match native {
        Origin::Pacman | Origin::Aur => Cmd::new(["pacman", "-S", "--noconfirm", "--needed"]).args(pk).root(),
        Origin::Dnf => Cmd::new(["dnf", "install", "-y"]).args(pk).root(),
        Origin::Apt => Cmd::new(["apt-get", "install", "-y"]).args(pk).root().env("DEBIAN_FRONTEND", "noninteractive"),
        Origin::Flatpak => Cmd::new(["flatpak", "install", "-y", "--noninteractive"]).args(pk),
    }
}

pub fn polkit_package(native: Origin) -> &'static str {
    match native {
        Origin::Apt => "pkexec",
        _ => "polkit",
    }
}

pub fn appstream_packages(native: Origin) -> &'static [&'static str] {
    match native {
        Origin::Pacman | Origin::Aur => &["archlinux-appstream-data"],
        Origin::Dnf => &["appstream-data"],
        Origin::Apt => &["appstream"],
        Origin::Flatpak => &[],
    }
}

pub struct Facts {
    pub flathub_user: bool,
    pub flathub_system: bool,
    pub native_catalog: bool,
    pub scope: Scope,
}

pub fn requirements(env: &Env, facts: &Facts) -> Vec<Requirement> {
    let mut v = vec![];
    let Some(native) = env.native else {
        if !env.flatpak {
            v.push(Requirement {
                id: "flatpak",
                title: "Flatpak".into(),
                detail: "No supported package manager was found. Install Flatpak with your system tools to get apps."
                    .into(),
                action: String::new(),
                steps: vec![],
                terminal: false,
                essential: true,
            });
        }
        return v;
    };
    if !env.can_elevate() {
        let pk = polkit_package(native);
        let cmd = native_install(native, &[pk]);
        v.push(Requirement {
            id: "polkit",
            title: "Administrator Access".into(),
            detail: format!(
                "Installing and removing system apps needs “{pk}”, which asks for your password. It will be installed in a terminal window."
            ),
            action: "Install".into(),
            steps: vec![Cmd::new(["sudo"]).args(cmd.argv)],
            terminal: true,
            essential: true,
        });
    }
    if !env.flatpak {
        v.push(Requirement {
            id: "flatpak",
            title: "Flatpak".into(),
            detail: "Get thousands of sandboxed apps from Flathub that work on every system.".into(),
            action: "Install".into(),
            steps: vec![native_install(native, &["flatpak"]).label("Installing Flatpak")],
            terminal: false,
            essential: true,
        });
    }
    let has_remote = match facts.scope {
        Scope::User => facts.flathub_user || facts.flathub_system,
        Scope::System => facts.flathub_system,
    };
    if !has_remote {
        v.push(Requirement {
            id: "flathub",
            title: "Flathub".into(),
            detail: "Add the Flathub catalog so apps from it can be installed and updated.".into(),
            action: "Add".into(),
            steps: vec![Flatpak::add_flathub(facts.scope)],
            terminal: false,
            essential: true,
        });
    }
    if !facts.native_catalog {
        let pk = appstream_packages(native);
        if !pk.is_empty() {
            let mut steps = vec![native_install(native, pk).label("Installing app catalog")];
            if native == Origin::Apt {
                steps.push(Cmd::new(["apt-get", "update"]).root().label("Refreshing package lists"));
            }
            v.push(Requirement {
                id: "appstream",
                title: format!("{} App Catalog", env.native_label()),
                detail: "Shows icons, screenshots and descriptions for apps from your system repositories.".into(),
                action: "Install".into(),
                steps,
                terminal: false,
                essential: false,
            });
        }
    }
    if native == Origin::Pacman {
        if env.aur_helper.is_none() {
            let dir = crate::http::cache_root().join("build");
            let mut steps = vec![native_install(native, &["base-devel", "git"]).label("Installing build tools")];
            steps.extend(crate::aur::makepkg_steps("paru-bin", &dir));
            v.push(Requirement {
                id: "aur",
                title: "AUR Support".into(),
                detail: "Install paru to get community packages from the Arch User Repository.".into(),
                action: "Install".into(),
                steps,
                terminal: false,
                essential: false,
            });
        }
        if !env.checkupdates {
            v.push(Requirement {
                id: "checkupdates",
                title: "Safe Update Checks".into(),
                detail: "Install pacman-contrib to check for system updates without touching the package database."
                    .into(),
                action: "Install".into(),
                steps: vec![native_install(native, &["pacman-contrib"]).label("Installing pacman-contrib")],
                terminal: false,
                essential: false,
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(native: Origin) -> Env {
        Env { native: Some(native), pkexec: true, ..Default::default() }
    }

    fn facts() -> Facts {
        Facts { flathub_user: false, flathub_system: false, native_catalog: false, scope: Scope::User }
    }

    #[test]
    fn arch_without_anything() {
        let r = requirements(&env(Origin::Pacman), &facts());
        let ids: Vec<&str> = r.iter().map(|x| x.id).collect();
        assert_eq!(ids, vec!["flatpak", "flathub", "appstream", "aur", "checkupdates"]);
        assert_eq!(r[0].steps[0].argv, vec!["pacman", "-S", "--noconfirm", "--needed", "flatpak"]);
        assert!(r[0].steps[0].root);
        assert_eq!(r[3].steps.len(), 4);
        assert_eq!(r[3].steps[3].argv[0], "makepkg");
    }

    #[test]
    fn debian_needs_polkit_in_terminal() {
        let mut e = env(Origin::Apt);
        e.pkexec = false;
        e.flatpak = true;
        let f = Facts { flathub_system: true, native_catalog: true, ..facts() };
        let r = requirements(&e, &f);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "polkit");
        assert!(r[0].terminal);
        assert_eq!(r[0].steps[0].argv, vec!["sudo", "apt-get", "install", "-y", "pkexec"]);
    }

    #[test]
    fn fedora_complete() {
        let mut e = env(Origin::Dnf);
        e.flatpak = true;
        let f = Facts { flathub_user: true, native_catalog: true, ..facts() };
        assert!(requirements(&e, &f).is_empty());
        let f = Facts { flathub_user: true, native_catalog: true, scope: Scope::System, ..facts() };
        assert_eq!(requirements(&e, &f)[0].steps[0].argv[3], "--system");
    }
}
