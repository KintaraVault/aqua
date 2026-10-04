use crate::model::{Details, Origin, Package, Update};
use crate::runner::Cmd;
use std::collections::HashMap;
use std::path::PathBuf;

pub trait Manager: Send + Sync {
    fn origin(&self) -> Origin;
    fn available(&self) -> bool;
    fn installed(&self) -> Vec<Package>;
    fn search(&self, query: &str) -> Vec<Package>;
    fn info(&self, name: &str) -> Option<Details>;
    fn updates(&self) -> Vec<Update>;
    fn owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String>;
    fn install(&self, pkg: &Package) -> Vec<Cmd>;
    fn remove(&self, pkg: &Package, purge: bool) -> Vec<Cmd>;
    fn update(&self, names: &[String]) -> Vec<Cmd>;
    fn refresh(&self) -> Vec<Cmd>;
    fn upgrade_is_whole_system(&self) -> bool {
        false
    }
}

pub fn key_values(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let continuation = line.starts_with(' ') || line.starts_with('\t');
        let split = if continuation { None } else { line.split_once(':') };
        match split {
            Some((k, v)) if !k.trim().is_empty() => out.push((k.trim().to_string(), v.trim().to_string())),
            _ => {
                let t = line.trim();
                let t = t.strip_prefix(':').map(str::trim).unwrap_or(t);
                if let Some(last) = out.last_mut() {
                    if !last.1.is_empty() {
                        last.1.push('\n');
                    }
                    last.1.push_str(t);
                }
            }
        }
    }
    out
}

pub fn get<'a>(kv: &'a [(String, String)], key: &str) -> &'a str {
    kv.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()).unwrap_or("")
}

pub fn words(v: &str) -> Vec<String> {
    if v == "None" {
        return vec![];
    }
    v.split_whitespace().map(str::to_string).collect()
}

pub fn regex_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

pub fn rank(pkgs: &mut [Package], query: &str) {
    let q = query.trim().to_lowercase();
    let score = |p: &Package| -> i64 {
        let n = p.name.to_lowercase();
        let t = p.title.to_lowercase();
        let mut s = 0i64;
        if n == q || t == q {
            s += 1000;
        } else if n.starts_with(&q) || t.starts_with(&q) {
            s += 400;
        } else if n.contains(&q) || t.contains(&q) {
            s += 200;
        } else if p.summary.to_lowercase().contains(&q) {
            s += 50;
        }
        if p.is_app {
            s += 120;
        }
        if n.starts_with("lib") || n.ends_with("-dev") || n.ends_with("-devel") || n.ends_with("-doc") {
            s -= 150;
        }
        s += (p.popularity * 10.0).min(100.0) as i64;
        s += (p.votes as f64).log10().max(0.0) as i64 * 10;
        s -= n.len() as i64;
        s
    };
    pkgs.sort_by_cached_key(|p| std::cmp::Reverse(score(p)));
}
