use crate::manager::{get, key_values, rank, regex_escape, words, Manager};
use crate::markup;
use crate::model::{Details, Link, Origin, Package, Update};
use crate::runner::{Cmd, SharedRunner};
use crate::units::{parse_date, parse_size};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Pacman {
    pub run: SharedRunner,
}

pub fn parse_query(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let ver = it.next()?;
            let mut p = Package::new(Origin::Pacman, name);
            p.version = ver.into();
            p.installed = true;
            p.installed_version = ver.into();
            Some(p)
        })
        .collect()
}

pub fn parse_search(out: &str, origin: Origin) -> Vec<Package> {
    let mut v: Vec<Package> = vec![];
    for line in out.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(p) = v.last_mut() {
                if !p.summary.is_empty() {
                    p.summary.push(' ');
                }
                p.summary.push_str(line.trim());
            }
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(rn) = it.next() else { continue };
        let Some(ver) = it.next() else { continue };
        let (repo, name) = rn.split_once('/').unwrap_or(("", rn));
        let mut p = Package::new(origin, name);
        p.repo = repo.into();
        p.version = ver.into();
        let rest: Vec<&str> = it.collect();
        let rest = rest.join(" ");
        if let Some(i) = rest.find("[installed") {
            p.installed = true;
            let tag = &rest[i + 1..];
            let tag = tag.split(']').next().unwrap_or("");
            p.installed_version = tag.strip_prefix("installed: ").unwrap_or(ver).trim().to_string();
            if p.installed_version.is_empty() || tag == "installed" {
                p.installed_version = ver.into();
            }
        }
        v.push(p);
    }
    v
}

pub fn parse_info(out: &str, origin: Origin) -> Option<Details> {
    let kv = key_values(out);
    let name = get(&kv, "Name");
    if name.is_empty() {
        return None;
    }
    let mut p = Package::new(origin, name);
    p.version = get(&kv, "Version").into();
    p.summary = get(&kv, "Description").into();
    p.repo = get(&kv, "Repository").into();
    p.homepage = get(&kv, "URL").into();
    p.license = words(get(&kv, "Licenses")).join(", ");
    p.size = parse_size(get(&kv, "Installed Size"));
    p.download_size = parse_size(get(&kv, "Download Size"));
    let mut d = Details { pkg: p, ..Default::default() };
    d.description = markup::plain(&d.pkg.summary);
    d.depends = words(get(&kv, "Depends On"));
    d.packager = get(&kv, "Packager").into();
    d.updated = parse_pacman_date(get(&kv, "Build Date"));
    if !d.pkg.homepage.is_empty() {
        d.links.push(Link { kind: "homepage".into(), url: d.pkg.homepage.clone() });
    }
    Some(d)
}

pub fn parse_pacman_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let month = |s: &str| crate::units::MONTHS.iter().position(|x| *x == s).map(|i| i as u32 + 1);
    if parts.len() >= 5 {
        let (m, d, y) = if let Some(m) = month(parts[1]) {
            (m, parts[2].parse().ok()?, parts[4].parse().ok()?)
        } else {
            (month(parts[2])?, parts[1].parse().ok()?, parts[3].parse().ok()?)
        };
        return Some(crate::units::days_from_civil(y, m, d) * 86400);
    }
    parse_date(s)
}

pub fn parse_checkupdates(out: &str, origin: Origin) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let from = it.next()?;
            if it.next()? != "->" {
                return None;
            }
            let to = it.next()?;
            Some(Update {
                origin: Some(origin),
                name: name.into(),
                from: from.into(),
                to: to.into(),
                ..Default::default()
            })
        })
        .collect()
}

pub fn parse_owners(out: &str, files: &[PathBuf]) -> HashMap<PathBuf, String> {
    let mut m = HashMap::new();
    for line in out.lines() {
        if let Some((path, rest)) = line.split_once(" is owned by ") {
            if let Some(name) = rest.split_whitespace().next() {
                m.insert(PathBuf::from(path.trim()), name.to_string());
            }
        }
    }
    if m.is_empty() {
        let names: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if names.len() == files.len() {
            for (f, n) in files.iter().zip(names) {
                m.insert(f.clone(), n.to_string());
            }
        }
    }
    m
}

impl Manager for Pacman {
    fn origin(&self) -> Origin {
        Origin::Pacman
    }

    fn available(&self) -> bool {
        self.run.have("pacman")
    }

    fn installed(&self) -> Vec<Package> {
        let o = self.run.run(&Cmd::new(["pacman", "-Q"]));
        parse_query(&o.stdout)
    }

    fn search(&self, query: &str) -> Vec<Package> {
        let terms: Vec<String> = query.split_whitespace().map(regex_escape).collect();
        if terms.is_empty() {
            return vec![];
        }
        let o = self.run.run(&Cmd::new(["pacman", "-Ss"]).args(terms));
        let mut v = parse_search(&o.stdout, Origin::Pacman);
        rank(&mut v, query);
        v.truncate(60);
        v
    }

    fn info(&self, name: &str) -> Option<Details> {
        let local = self.run.run(&Cmd::new(["pacman", "-Qi", name]));
        let sync = self.run.run(&Cmd::new(["pacman", "-Si", name]));
        let mut d = parse_info(&sync.stdout, Origin::Pacman);
        if let Some(l) = parse_info(&local.stdout, Origin::Pacman) {
            match &mut d {
                Some(d) => {
                    d.pkg.installed = true;
                    d.pkg.installed_version = l.pkg.version.clone();
                    d.pkg.size = l.pkg.size.or(d.pkg.size);
                }
                None => {
                    let mut l = l;
                    l.pkg.installed = true;
                    l.pkg.installed_version = l.pkg.version.clone();
                    d = Some(l);
                }
            }
        }
        d
    }

    fn updates(&self) -> Vec<Update> {
        if self.run.have("checkupdates") {
            let o = self.run.run(&Cmd::new(["checkupdates", "--nocolor"]).ok(2));
            return parse_checkupdates(&o.stdout, Origin::Pacman);
        }
        let o = self.run.run(&Cmd::new(["pacman", "-Qu"]).ok(1));
        parse_checkupdates(&o.stdout, Origin::Pacman)
    }

    fn owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        if files.is_empty() {
            return HashMap::new();
        }
        let o = self.run.run(&Cmd::new(["pacman", "-Qo"]).args(files.iter().map(|f| f.to_string_lossy().into_owned())));
        parse_owners(&o.stdout, files)
    }

    fn install(&self, pkg: &Package) -> Vec<Cmd> {
        vec![Cmd::new(["pacman", "-S", "--noconfirm", "--needed", &pkg.name])
            .root()
            .label(format!("Installing {}", pkg.name))]
    }

    fn remove(&self, pkg: &Package, purge: bool) -> Vec<Cmd> {
        let flag = if purge { "-Rns" } else { "-Rs" };
        vec![Cmd::new(["pacman", flag, "--noconfirm", &pkg.name]).root().label(format!("Removing {}", pkg.name))]
    }

    fn update(&self, _names: &[String]) -> Vec<Cmd> {
        vec![Cmd::new(["pacman", "-Syu", "--noconfirm"]).root().label("Updating system packages")]
    }

    fn refresh(&self) -> Vec<Cmd> {
        vec![Cmd::new(["pacman", "-Sy"]).root().label("Refreshing package databases")]
    }

    fn upgrade_is_whole_system(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;
    use std::sync::Arc;

    const SS: &str = "extra/gimp 3.0.4-1 [installed]\n    GNU Image Manipulation Program\nextra/gimp-help 2.10.34-1 (gimp)\n    Help files\n    for GIMP\ncore/glibc 2.40-1 [installed: 2.39-1]\n    GNU C Library\n";

    #[test]
    fn search_output() {
        let v = parse_search(SS, Origin::Pacman);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].repo.as_str(), v[0].name.as_str(), v[0].installed), ("extra", "gimp", true));
        assert_eq!(v[0].installed_version, "3.0.4-1");
        assert_eq!(v[1].summary, "Help files for GIMP");
        assert!(!v[1].installed);
        assert_eq!(v[2].installed_version, "2.39-1");
    }

    #[test]
    fn info_output() {
        let txt = "Repository      : extra\nName            : gimp\nVersion         : 3.0.4-1\nDescription     : GNU Image Manipulation Program\nURL             : https://www.gimp.org/\nLicenses        : GPL-3.0-or-later  LGPL-3.0-or-later\nDepends On      : babl  gegl\n                  gtk3\nDownload Size   : 21.27 MiB\nInstalled Size  : 124.50 MiB\nPackager        : Someone <a@b>\nBuild Date      : Mon May 12 10:00:00 2025\n";
        let d = parse_info(txt, Origin::Pacman).unwrap();
        assert_eq!(d.pkg.name, "gimp");
        assert_eq!(d.pkg.license, "GPL-3.0-or-later, LGPL-3.0-or-later");
        assert_eq!(d.depends, vec!["babl", "gegl", "gtk3"]);
        assert_eq!(d.pkg.size, Some(130_547_712));
        assert_eq!(d.link("homepage"), Some("https://www.gimp.org/"));
        assert_eq!(d.updated, Some(crate::units::days_from_civil(2025, 5, 12) * 86400));
        assert_eq!(parse_pacman_date("Mon 12 May 2025 10:00:00 AM UTC"), d.updated);
    }

    #[test]
    fn updates_and_owners() {
        let u = parse_checkupdates("linux 6.9.1-1 -> 6.9.2-1\nbad line\n", Origin::Pacman);
        assert_eq!(u.len(), 1);
        assert_eq!((u[0].from.as_str(), u[0].to.as_str()), ("6.9.1-1", "6.9.2-1"));
        let files = vec![PathBuf::from("/usr/share/applications/gimp.desktop")];
        let o = parse_owners("/usr/share/applications/gimp.desktop is owned by gimp 3.0.4-1\n", &files);
        assert_eq!(o.get(&files[0]).map(String::as_str), Some("gimp"));
    }

    #[test]
    fn commands_need_root() {
        let r = Arc::new(FakeRunner::with(&["pacman"]));
        r.on("pacman -Ss", 0, SS);
        let p = Pacman { run: r.clone() };
        let found = p.search("gimp");
        assert_eq!(found[0].name, "gimp");
        let c = p.install(&found[0]);
        assert!(c[0].root);
        assert_eq!(c[0].argv, vec!["pacman", "-S", "--noconfirm", "--needed", "gimp"]);
        assert!(p.update(&[]).iter().all(|c| c.argv.contains(&"-Syu".to_string())));
        assert_eq!(p.remove(&found[0], true)[0].argv[1], "-Rns");
    }

    #[test]
    fn search_escapes_regex() {
        let r = Arc::new(FakeRunner::with(&["pacman"]));
        let p = Pacman { run: r.clone() };
        p.search("c++ (x)");
        assert_eq!(r.calls()[0], "pacman -Ss c\\+\\+ \\(x\\)");
    }
}
