use crate::http::{urlencode, Http};
use crate::manager::{rank, Manager};
use crate::markup;
use crate::model::{Details, Link, Origin, Package, Update};
use crate::runner::{Cmd, SharedRunner};
use crate::vercmp::newer;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const RPC: &str = "https://aur.archlinux.org/rpc/v5";
pub const HELPERS: [&str; 2] = ["paru", "yay"];

pub struct Aur {
    pub run: SharedRunner,
    pub http: Arc<Http>,
}

pub fn from_rpc(v: &Value) -> Option<Package> {
    let name = v.get("Name")?.as_str()?;
    let mut p = Package::new(Origin::Aur, name);
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    p.version = s("Version");
    p.summary = s("Description");
    p.homepage = s("URL");
    p.developer = s("Maintainer");
    p.repo = "aur".into();
    p.votes = v.get("NumVotes").and_then(|x| x.as_u64()).unwrap_or(0);
    p.popularity = v.get("Popularity").and_then(|x| x.as_f64()).unwrap_or(0.0);
    p.out_of_date = v.get("OutOfDate").is_some_and(|x| !x.is_null());
    p.license = v
        .get("License")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|l| l.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    p.keywords = v
        .get("Keywords")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Some(p)
}

pub fn details_from_rpc(v: &Value) -> Option<Details> {
    let pkg = from_rpc(v)?;
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let mut d = Details { description: markup::plain(&pkg.summary), ..Default::default() };
    if !pkg.homepage.is_empty() {
        d.links.push(Link { kind: "homepage".into(), url: pkg.homepage.clone() });
    }
    d.links.push(Link { kind: "aur".into(), url: format!("https://aur.archlinux.org/packages/{}", pkg.name) });
    d.depends = list("Depends");
    d.maintainer = pkg.developer.clone();
    d.updated = v.get("LastModified").and_then(|x| x.as_i64());
    d.pkg = pkg;
    Some(d)
}

pub fn helper_install(helper: &str, names: &[String]) -> Cmd {
    let mut c = Cmd::new([helper, "-S", "--noconfirm", "--needed", "--sudo", "pkexec"]);
    if helper == "yay" {
        c = c.args(["--answerdiff", "None", "--answerclean", "None", "--answeredit", "None", "--removemake"]);
    } else {
        c = c.args(["--skipreview", "--removemake"]);
    }
    c.args(names.iter().cloned())
}

pub fn makepkg_steps(name: &str, workdir: &std::path::Path) -> Vec<Cmd> {
    let dir = workdir.join(name);
    vec![
        Cmd::new(["rm", "-rf", &dir.to_string_lossy()]),
        Cmd::new([
            "git",
            "clone",
            "--depth=1",
            &format!("https://aur.archlinux.org/{name}.git"),
            &dir.to_string_lossy(),
        ])
        .label(format!("Downloading {name}")),
        Cmd::new(["makepkg", "-si", "--noconfirm", "--needed"])
            .cwd(dir)
            .env("PACMAN_AUTH", "pkexec")
            .label(format!("Building {name}")),
    ]
}

impl Aur {
    pub fn helper(&self) -> Option<&'static str> {
        HELPERS.into_iter().find(|h| self.run.have(h))
    }

    pub fn rpc_info(&self, names: &[String]) -> Vec<Value> {
        let mut out = vec![];
        for chunk in names.chunks(100) {
            let q: Vec<String> = chunk.iter().map(|n| format!("arg[]={}", urlencode(n))).collect();
            let url = format!("{RPC}/info?{}", q.join("&"));
            if let Ok(v) = self.http.get_json(&url, Duration::from_secs(600)) {
                if let Some(a) = v.get("results").and_then(|r| r.as_array()) {
                    out.extend(a.iter().cloned());
                }
            }
        }
        out
    }

    fn foreign(&self) -> Vec<Package> {
        let o = self.run.run(&Cmd::new(["pacman", "-Qm"]));
        crate::pacman::parse_query(&o.stdout)
            .into_iter()
            .map(|mut p| {
                p.origin = Some(Origin::Aur);
                p.repo = "aur".into();
                p
            })
            .collect()
    }
}

impl Manager for Aur {
    fn origin(&self) -> Origin {
        Origin::Aur
    }

    fn available(&self) -> bool {
        self.run.have("pacman")
    }

    fn installed(&self) -> Vec<Package> {
        self.foreign()
    }

    fn search(&self, query: &str) -> Vec<Package> {
        let q = query.trim();
        if q.len() < 2 {
            return vec![];
        }
        let url = format!("{RPC}/search/{}?by=name-desc", urlencode(q));
        let Ok(v) = self.http.get_json(&url, Duration::from_secs(1800)) else { return vec![] };
        let mut res: Vec<Package> = v
            .get("results")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(from_rpc).collect())
            .unwrap_or_default();
        let inst: HashMap<String, String> = self.foreign().into_iter().map(|p| (p.name, p.installed_version)).collect();
        for p in &mut res {
            if let Some(v) = inst.get(&p.name) {
                p.installed = true;
                p.installed_version = v.clone();
            }
        }
        rank(&mut res, q);
        res.truncate(40);
        res
    }

    fn info(&self, name: &str) -> Option<Details> {
        let v = self.rpc_info(&[name.to_string()]);
        let mut d = details_from_rpc(v.first()?)?;
        if let Some(p) = self.foreign().into_iter().find(|p| p.name == name) {
            d.pkg.installed = true;
            d.pkg.installed_version = p.installed_version;
        }
        Some(d)
    }

    fn updates(&self) -> Vec<Update> {
        let local = self.foreign();
        if local.is_empty() {
            return vec![];
        }
        let names: Vec<String> = local.iter().map(|p| p.name.clone()).collect();
        let remote: HashMap<String, String> =
            self.rpc_info(&names).iter().filter_map(from_rpc).map(|p| (p.name, p.version)).collect();
        local
            .into_iter()
            .filter_map(|p| {
                let r = remote.get(&p.name)?;
                newer(r, &p.installed_version).then(|| Update {
                    origin: Some(Origin::Aur),
                    name: p.name,
                    from: p.installed_version,
                    to: r.clone(),
                    repo: "aur".into(),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn owners(&self, _files: &[PathBuf]) -> HashMap<PathBuf, String> {
        HashMap::new()
    }

    fn install(&self, pkg: &Package) -> Vec<Cmd> {
        match self.helper() {
            Some(h) => vec![helper_install(h, std::slice::from_ref(&pkg.name)).label(format!("Building {}", pkg.name))],
            None => makepkg_steps(&pkg.name, &crate::http::cache_root().join("build")),
        }
    }

    fn remove(&self, pkg: &Package, purge: bool) -> Vec<Cmd> {
        let flag = if purge { "-Rns" } else { "-Rs" };
        vec![Cmd::new(["pacman", flag, "--noconfirm", &pkg.name]).root().label(format!("Removing {}", pkg.name))]
    }

    fn update(&self, names: &[String]) -> Vec<Cmd> {
        match self.helper() {
            Some(h) if names.is_empty() => {
                let mut c = helper_install(h, &[]);
                c.argv[1] = "-Sua".into();
                vec![c.label("Updating AUR packages")]
            }
            Some(h) => vec![helper_install(h, names).label("Updating AUR packages")],
            None => names.iter().flat_map(|n| makepkg_steps(n, &crate::http::cache_root().join("build"))).collect(),
        }
    }

    fn refresh(&self) -> Vec<Cmd> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_parsing() {
        let v: Value = serde_json::from_str(
            r#"{"Name":"paru-bin","Version":"2.1.0-1","Description":"Feature packed AUR helper","NumVotes":325,"Popularity":1.27,"OutOfDate":1765668822,"License":["GPL-3.0-or-later"],"Depends":["git","pacman"],"Maintainer":null,"URL":"https://github.com/morganamilo/paru","LastModified":1751964686}"#,
        )
        .unwrap();
        let d = details_from_rpc(&v).unwrap();
        assert_eq!(d.pkg.name, "paru-bin");
        assert!(d.pkg.out_of_date);
        assert_eq!(d.depends, vec!["git", "pacman"]);
        assert_eq!(d.link("aur"), Some("https://aur.archlinux.org/packages/paru-bin"));
        assert_eq!(d.pkg.developer, "");
    }

    #[test]
    fn helper_commands() {
        let c = helper_install("paru", &["foo".into()]);
        assert_eq!(c.argv[..6], ["paru", "-S", "--noconfirm", "--needed", "--sudo", "pkexec"].map(String::from));
        assert!(!c.root);
        let y = helper_install("yay", &["foo".into()]);
        assert!(y.argv.contains(&"--answerdiff".to_string()));
        let steps = makepkg_steps("paru-bin", std::path::Path::new("/tmp/b"));
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[2].env, vec![("PACMAN_AUTH".to_string(), "pkexec".to_string())]);
        assert_eq!(steps[2].cwd.as_deref(), Some(std::path::Path::new("/tmp/b/paru-bin")));
    }
}
