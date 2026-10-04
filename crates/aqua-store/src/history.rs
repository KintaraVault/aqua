use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub time: i64,
    pub action: String,
    pub key: String,
    pub name: String,
    pub version: String,
    pub source: String,
}

pub fn path() -> PathBuf {
    std::env::var_os("AQUA_STORE_HISTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("aqua/store-history.json"))
}

pub fn load() -> Vec<Entry> {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn record(e: Entry) {
    let mut v = load();
    v.insert(0, e);
    v.truncate(1000);
    if let Ok(s) = serde_json::to_vec_pretty(&v) {
        crate::http::write_atomic(&path(), &s);
    }
}

pub fn filter<'a>(v: &'a [Entry], query: &str, action: &str, since: i64) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    v.iter()
        .filter(|e| e.time >= since)
        .filter(|e| action.is_empty() || e.action == action)
        .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering() {
        let mk = |t, a: &str, n: &str| Entry {
            time: t,
            action: a.into(),
            key: format!("x:{n}"),
            name: n.into(),
            version: "1".into(),
            source: "s".into(),
        };
        let v = vec![mk(10, "install", "Gimp"), mk(20, "remove", "Inkscape"), mk(30, "update", "Gimp")];
        assert_eq!(filter(&v, "gimp", "", 0).len(), 2);
        assert_eq!(filter(&v, "", "remove", 0).len(), 1);
        assert_eq!(filter(&v, "", "", 15).len(), 2);
    }
}
