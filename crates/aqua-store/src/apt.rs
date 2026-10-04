use crate::manager::{get, key_values, rank, Manager};
use crate::markup;
use crate::model::{Details, Link, Origin, Package, Update};
use crate::runner::{Cmd, SharedRunner};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Apt {
    pub run: SharedRunner,
}

pub fn parse_dpkg(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 4 || !f[3].starts_with("ii") {
                return None;
            }
            let mut p = Package::new(Origin::Apt, f[0]);
            p.version = f[1].into();
            p.installed = true;
            p.installed_version = f[1].into();
            p.size = f[2].trim().parse::<u64>().ok().map(|k| k * 1024);
            p.summary = f.get(4).map(|s| s.trim().to_string()).unwrap_or_default();
            Some(p)
        })
        .collect()
}

pub fn parse_search(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let (n, s) = l.split_once(" - ")?;
            let mut p = Package::new(Origin::Apt, n.trim());
            p.summary = s.trim().into();
            Some(p)
        })
        .collect()
}

pub fn parse_show(out: &str) -> Option<Details> {
    let stanza = out.split("\n\n").find(|s| !s.trim().is_empty())?;
    let kv = key_values(stanza);
    let name = get(&kv, "Package");
    if name.is_empty() {
        return None;
    }
    let mut p = Package::new(Origin::Apt, name);
    p.version = get(&kv, "Version").into();
    let desc = {
        let d = get(&kv, "Description");
        if d.is_empty() {
            get(&kv, "Description-en")
        } else {
            d
        }
    };
    let (summary, long) = desc.split_once('\n').unwrap_or((desc, ""));
    p.summary = summary.trim().into();
    p.homepage = get(&kv, "Homepage").into();
    p.size = get(&kv, "Installed-Size").parse::<u64>().ok().map(|k| k * 1024);
    p.download_size = get(&kv, "Size").parse().ok();
    p.repo = get(&kv, "Section").into();
    p.categories = vec![get(&kv, "Section").to_string()];
    let mut d = Details { pkg: p, ..Default::default() };
    d.description = markup::plain(long);
    d.maintainer = get(&kv, "Maintainer").into();
    d.depends = get(&kv, "Depends")
        .split(',')
        .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !d.pkg.homepage.is_empty() {
        d.links.push(Link { kind: "homepage".into(), url: d.pkg.homepage.clone() });
    }
    Some(d)
}

pub fn parse_upgradable(out: &str) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once('/')?;
            let mut it = rest.split_whitespace();
            let suite = it.next()?;
            let to = it.next()?;
            let from = l.split("upgradable from: ").nth(1)?.trim_end_matches(']').trim();
            Some(Update {
                origin: Some(Origin::Apt),
                name: name.into(),
                from: from.into(),
                to: to.into(),
                repo: suite.split(',').next().unwrap_or("").into(),
                ..Default::default()
            })
        })
        .collect()
}

pub fn parse_dpkg_s(out: &str) -> HashMap<PathBuf, String> {
    let mut m = HashMap::new();
    for l in out.lines() {
        if let Some((pkgs, path)) = l.split_once(": ") {
            if let Some(first) = pkgs.split(',').next() {
                let n = first.trim().split(':').next().unwrap_or("").to_string();
                if !n.is_empty() && !n.contains(' ') {
                    m.insert(PathBuf::from(path.trim()), n);
                }
            }
        }
    }
    m
}

fn apt_get() -> Cmd {
    Cmd::new([
        "apt-get",
        "-y",
        "-o",
        "APT::Status-Fd=1",
        "-o",
        "Dpkg::Options::=--force-confdef",
        "-o",
        "Dpkg::Options::=--force-confold",
    ])
    .root()
    .env("DEBIAN_FRONTEND", "noninteractive")
}

impl Manager for Apt {
    fn origin(&self) -> Origin {
        Origin::Apt
    }

    fn available(&self) -> bool {
        self.run.have("apt-get") && self.run.have("dpkg-query")
    }

    fn installed(&self) -> Vec<Package> {
        let o = self.run.run(&Cmd::new([
            "dpkg-query",
            "-W",
            "-f=${Package}\\t${Version}\\t${Installed-Size}\\t${db:Status-Abbrev}\\t${binary:Summary}\\n",
        ]));
        parse_dpkg(&o.stdout)
    }

    fn search(&self, query: &str) -> Vec<Package> {
        if query.trim().is_empty() {
            return vec![];
        }
        let o = self.run.run(&Cmd::new(["apt-cache", "search"]).args(query.split_whitespace()));
        let mut v = parse_search(&o.stdout);
        let inst: HashMap<String, String> =
            self.installed().into_iter().map(|p| (p.name, p.installed_version)).collect();
        for p in &mut v {
            if let Some(ver) = inst.get(&p.name) {
                p.installed = true;
                p.installed_version = ver.clone();
            }
        }
        rank(&mut v, query);
        v.truncate(60);
        v
    }

    fn info(&self, name: &str) -> Option<Details> {
        let o = self.run.run(&Cmd::new(["apt-cache", "show", "--no-all-versions", name]));
        let mut d = parse_show(&o.stdout)?;
        let i = self.run.run(&Cmd::new(["dpkg-query", "-W", "-f=${db:Status-Abbrev}\\t${Version}", name]));
        if let Some((st, ver)) = i.stdout.split_once('\t') {
            if st.starts_with("ii") {
                d.pkg.installed = true;
                d.pkg.installed_version = ver.trim().into();
            }
        }
        Some(d)
    }

    fn updates(&self) -> Vec<Update> {
        let o = self.run.run(&Cmd::new(["apt", "list", "--upgradable"]));
        parse_upgradable(&o.stdout)
    }

    fn owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        if files.is_empty() {
            return HashMap::new();
        }
        let o =
            self.run.run(&Cmd::new(["dpkg-query", "-S"]).args(files.iter().map(|f| f.to_string_lossy().into_owned())));
        parse_dpkg_s(&o.stdout)
    }

    fn install(&self, pkg: &Package) -> Vec<Cmd> {
        vec![apt_get().args(["install", &pkg.name]).label(format!("Installing {}", pkg.name))]
    }

    fn remove(&self, pkg: &Package, purge: bool) -> Vec<Cmd> {
        let verb = if purge { "purge" } else { "remove" };
        vec![apt_get().args([verb, &pkg.name]).label(format!("Removing {}", pkg.name))]
    }

    fn update(&self, names: &[String]) -> Vec<Cmd> {
        if names.is_empty() {
            vec![apt_get().arg("dist-upgrade").label("Updating packages")]
        } else {
            vec![apt_get().args(["install", "--only-upgrade"]).args(names.iter().cloned()).label("Updating packages")]
        }
    }

    fn refresh(&self) -> Vec<Cmd> {
        vec![Cmd::new(["apt-get", "update"]).root().label("Refreshing package lists")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_list() {
        let v = parse_dpkg("gimp\t3.0.4-1\t120000\tii \tGNU Image Manipulation Program\nold\t1\t1\trc \tgone\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].size, Some(120_000 * 1024));
    }

    #[test]
    fn search_and_show() {
        let v = parse_search("gimp - GNU Image Manipulation Program\ngimp-data - Data files for GIMP\n");
        assert_eq!(v[1].name, "gimp-data");
        let show = "Package: gimp\nVersion: 3.0.4-1\nInstalled-Size: 20480\nMaintainer: Debian <x@y>\nDepends: libc6 (>= 2.34), libgtk-3-0\nSection: graphics\nHomepage: https://www.gimp.org/\nDescription: GNU Image Manipulation Program\n GIMP is an advanced picture editor.\n .\n  - layers\n\nPackage: gimp\nVersion: 2.10\n";
        let d = parse_show(show).unwrap();
        assert_eq!(d.pkg.version, "3.0.4-1");
        assert_eq!(d.pkg.summary, "GNU Image Manipulation Program");
        assert_eq!(d.depends, vec!["libc6", "libgtk-3-0"]);
        assert_eq!(d.description.len(), 2);
        assert_eq!(d.pkg.size, Some(20480 * 1024));
    }

    #[test]
    fn upgradable() {
        let v = parse_upgradable(
            "Listing... Done\nfirefox-esr/stable-security 128.3.0esr-1~deb12u1 amd64 [upgradable from: 128.2.0esr-1~deb12u1]\n",
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "firefox-esr");
        assert_eq!(v[0].from, "128.2.0esr-1~deb12u1");
        assert_eq!(v[0].repo, "stable-security");
    }

    #[test]
    fn owners() {
        let m = parse_dpkg_s("gimp: /usr/share/applications/gimp.desktop\nlibfoo:amd64, libfoo2: /usr/lib/x\n");
        assert_eq!(m.get(&PathBuf::from("/usr/share/applications/gimp.desktop")).unwrap(), "gimp");
        assert_eq!(m.get(&PathBuf::from("/usr/lib/x")).unwrap(), "libfoo");
    }

    #[test]
    fn noninteractive_root() {
        let a = Apt { run: std::sync::Arc::new(crate::runner::fake::FakeRunner::default()) };
        let c = &a.install(&Package::new(Origin::Apt, "vim"))[0];
        assert!(c.root);
        assert_eq!(
            c.final_argv(false, "pkexec")[..3],
            ["pkexec".to_string(), "env".into(), "DEBIAN_FRONTEND=noninteractive".into()]
        );
        assert_eq!(c.argv.last().unwrap(), "vim");
    }
}
