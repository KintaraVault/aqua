use crate::manager::{get, key_values, rank, Manager};
use crate::markup;
use crate::model::{Details, Link, Origin, Package, Update};
use crate::runner::{Cmd, SharedRunner};
use crate::units::parse_size;
use std::collections::HashMap;
use std::path::PathBuf;

const ARCHES: [&str; 9] = ["x86_64", "noarch", "i686", "i386", "aarch64", "ppc64le", "s390x", "armv7hl", "src"];

pub struct Dnf {
    pub run: SharedRunner,
}

pub fn strip_arch(s: &str) -> &str {
    match s.rsplit_once('.') {
        Some((n, a)) if ARCHES.contains(&a) => n,
        _ => s,
    }
}

pub fn parse_rpm_qa(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 2 || f[0].is_empty() {
                return None;
            }
            let mut p = Package::new(Origin::Dnf, f[0]);
            let ver = f[1].strip_prefix("0:").unwrap_or(f[1]);
            p.version = ver.into();
            p.installed = true;
            p.installed_version = ver.into();
            p.size = f.get(2).and_then(|s| s.trim().parse().ok());
            p.summary = f.get(3).map(|s| s.trim().to_string()).unwrap_or_default();
            Some(p)
        })
        .collect()
}

pub fn parse_search(out: &str) -> Vec<Package> {
    let mut v: Vec<Package> = vec![];
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with('=')
            || t.starts_with("Matched fields")
            || t.starts_with("Last metadata")
            || t.starts_with("Updating and loading")
            || t.starts_with("Repositories loaded")
        {
            continue;
        }
        let (head, summary) = match t.split_once(" : ") {
            Some((h, s)) => (h.trim(), s.trim()),
            None => match t.split_once(|c: char| c.is_whitespace()) {
                Some((h, s)) => (h.trim(), s.trim()),
                None => (t, ""),
            },
        };
        if !head.contains('.') {
            continue;
        }
        let name = strip_arch(head);
        if v.iter().any(|p| p.name == name) {
            continue;
        }
        let mut p = Package::new(Origin::Dnf, name);
        p.summary = summary.into();
        v.push(p);
    }
    v
}

pub fn parse_info(out: &str) -> Vec<Details> {
    let mut res = vec![];
    let mut section_installed = false;
    let mut chunk = String::new();
    let flush = |chunk: &mut String, installed: bool, res: &mut Vec<Details>| {
        let kv = key_values(chunk);
        chunk.clear();
        let name = get(&kv, "Name");
        if name.is_empty() {
            return;
        }
        let mut p = Package::new(Origin::Dnf, name);
        let epoch = get(&kv, "Epoch");
        let ver = get(&kv, "Version");
        let rel = get(&kv, "Release");
        p.version = match (epoch, rel) {
            ("" | "0", "") => ver.to_string(),
            ("" | "0", r) => format!("{ver}-{r}"),
            (e, "") => format!("{e}:{ver}"),
            (e, r) => format!("{e}:{ver}-{r}"),
        };
        p.summary = get(&kv, "Summary").into();
        p.repo = {
            let r = get(&kv, "Repository");
            let r = if r.is_empty() { get(&kv, "From repository") } else { r };
            if r.is_empty() { get(&kv, "Repo") } else { r }.to_string()
        };
        p.homepage = get(&kv, "URL").into();
        p.license = get(&kv, "License").into();
        let size = get(&kv, "Installed size");
        p.size = parse_size(if size.is_empty() { get(&kv, "Size") } else { size });
        p.download_size = parse_size(get(&kv, "Download size"));
        p.installed = installed || p.repo == "@System" || p.repo == "installed";
        if p.installed {
            p.installed_version = p.version.clone();
        }
        let mut d = Details { pkg: p, ..Default::default() };
        d.description = markup::plain(get(&kv, "Description"));
        if !d.pkg.homepage.is_empty() {
            d.links.push(Link { kind: "homepage".into(), url: d.pkg.homepage.clone() });
        }
        d.packager = get(&kv, "Packager").into();
        res.push(d);
    };
    for line in out.lines() {
        let t = line.trim();
        if t == "Installed Packages" || t == "Installed packages" {
            flush(&mut chunk, section_installed, &mut res);
            section_installed = true;
            continue;
        }
        if t == "Available Packages" || t == "Available packages" {
            flush(&mut chunk, section_installed, &mut res);
            section_installed = false;
            continue;
        }
        if t.is_empty() {
            flush(&mut chunk, section_installed, &mut res);
            continue;
        }
        if line.starts_with("Name") && !chunk.is_empty() {
            flush(&mut chunk, section_installed, &mut res);
        }
        chunk.push_str(line);
        chunk.push('\n');
    }
    flush(&mut chunk, section_installed, &mut res);
    res
}

pub fn parse_check_update(out: &str) -> Vec<Update> {
    let mut v = vec![];
    for line in out.lines() {
        let t = line.trim();
        if t.starts_with("Obsoleting") || t.starts_with("Security:") {
            break;
        }
        let f: Vec<&str> = t.split_whitespace().collect();
        if f.len() != 3 || !f[0].contains('.') || !f[1].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        v.push(Update {
            origin: Some(Origin::Dnf),
            name: strip_arch(f[0]).into(),
            to: f[1].into(),
            repo: f[2].into(),
            ..Default::default()
        });
    }
    v
}

impl Manager for Dnf {
    fn origin(&self) -> Origin {
        Origin::Dnf
    }

    fn available(&self) -> bool {
        self.run.have("dnf") && self.run.have("rpm")
    }

    fn installed(&self) -> Vec<Package> {
        let o = self.run.run(&Cmd::new([
            "rpm",
            "-qa",
            "--qf",
            "%{NAME}\\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\\t%{SIZE}\\t%{SUMMARY}\\n",
        ]));
        parse_rpm_qa(&o.stdout)
    }

    fn search(&self, query: &str) -> Vec<Package> {
        if query.trim().is_empty() {
            return vec![];
        }
        let o = self.run.run(&Cmd::new(["dnf", "search", "-q"]).args(query.split_whitespace()));
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
        let o = self.run.run(&Cmd::new(["dnf", "info", "-q", name]));
        let all = parse_info(&o.stdout);
        let inst = all.iter().find(|d| d.pkg.installed).cloned();
        let mut best = all.iter().find(|d| !d.pkg.installed).cloned().or_else(|| inst.clone())?;
        if let Some(i) = inst {
            best.pkg.installed = true;
            best.pkg.installed_version = i.pkg.version.clone();
            best.pkg.size = i.pkg.size.or(best.pkg.size);
        }
        Some(best)
    }

    fn updates(&self) -> Vec<Update> {
        let inst: HashMap<String, String> =
            self.installed().into_iter().map(|p| (p.name, p.installed_version)).collect();
        let o = self.run.run(&Cmd::new(["dnf", "check-update", "-q"]).ok(100));
        let mut v = parse_check_update(&o.stdout);
        for u in &mut v {
            u.from = inst.get(&u.name).cloned().unwrap_or_default();
        }
        v
    }

    fn owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        let mut m = HashMap::new();
        if files.is_empty() {
            return m;
        }
        let o = self.run.run(
            &Cmd::new(["rpm", "-qf", "--qf", "%{NAME}\\n"])
                .args(files.iter().map(|f| f.to_string_lossy().into_owned())),
        );
        let lines: Vec<&str> = o.stdout.lines().collect();
        if lines.len() == files.len() {
            for (f, l) in files.iter().zip(lines) {
                if !l.contains(' ') && !l.is_empty() {
                    m.insert(f.clone(), l.to_string());
                }
            }
        }
        m
    }

    fn install(&self, pkg: &Package) -> Vec<Cmd> {
        vec![Cmd::new(["dnf", "install", "-y", &pkg.name]).root().label(format!("Installing {}", pkg.name))]
    }

    fn remove(&self, pkg: &Package, _purge: bool) -> Vec<Cmd> {
        vec![Cmd::new(["dnf", "remove", "-y", &pkg.name]).root().label(format!("Removing {}", pkg.name))]
    }

    fn update(&self, names: &[String]) -> Vec<Cmd> {
        vec![Cmd::new(["dnf", "upgrade", "-y"]).args(names.iter().cloned()).root().label("Updating packages")]
    }

    fn refresh(&self) -> Vec<Cmd> {
        vec![Cmd::new(["dnf", "makecache", "-q"]).label("Refreshing package metadata")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_list() {
        let v = parse_rpm_qa(
            "gimp\t2:3.0.4-1.fc42\t130000000\tGNU Image Manipulation Program\nbash\t0:5.2-1\t100\tShell\n",
        );
        assert_eq!(v[0].version, "2:3.0.4-1.fc42");
        assert_eq!(v[1].version, "5.2-1");
        assert_eq!(v[0].size, Some(130_000_000));
    }

    #[test]
    fn search_dnf4_and_dnf5() {
        let d4 = "========== Name Exactly Matched: gimp ==========\ngimp.x86_64 : GNU Image Manipulation Program\n===== Name & Summary Matched =====\ngimp-libs.i686 : GIMP libraries\ngimp-libs.x86_64 : GIMP libraries\n";
        let v = parse_search(d4);
        assert_eq!(v.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["gimp", "gimp-libs"]);
        let d5 = "Matched fields: name (exact)\n gimp.x86_64\tGNU Image Manipulation Program\nMatched fields: name, summary\n gimp-help.noarch\tHelp files for GIMP\n";
        let v = parse_search(d5);
        assert_eq!(v[1].name, "gimp-help");
        assert_eq!(v[1].summary, "Help files for GIMP");
    }

    #[test]
    fn info_sections() {
        let txt = "Installed Packages\nName         : gimp\nEpoch        : 2\nVersion      : 3.0.2\nRelease      : 1.fc42\nArchitecture : x86_64\nSize         : 120 M\nSource       : gimp.src.rpm\nRepository   : @System\nSummary      : GNU Image Manipulation Program\nURL          : https://www.gimp.org/\nLicense      : GPL-3.0-or-later\nDescription  : GIMP is a powerful\n             : image editor.\n\nAvailable Packages\nName         : gimp\nEpoch        : 2\nVersion      : 3.0.4\nRelease      : 1.fc42\nSize         : 25 M\nRepository   : updates\nSummary      : GNU Image Manipulation Program\nDescription  : GIMP is a powerful\n             : image editor.\n";
        let all = parse_info(txt);
        assert_eq!(all.len(), 2);
        assert!(all[0].pkg.installed);
        assert_eq!(all[0].pkg.version, "2:3.0.2-1.fc42");
        assert_eq!(all[1].pkg.repo, "updates");
        assert_eq!(crate::model::blocks_to_text(&all[1].description), "GIMP is a powerful image editor.");
    }

    #[test]
    fn check_update() {
        let v = parse_check_update(
            "\nkernel.x86_64    6.10.3-200.fc40    updates\nfirefox.x86_64   130.0-1.fc40       updates\nObsoleting Packages\nfoo.x86_64 1-1 updates\n",
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "kernel");
        assert_eq!(v[1].to, "130.0-1.fc40");
    }
}
