use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct Http {
    agent: ureq::Agent,
    pub cache_dir: PathBuf,
}

pub fn hash(s: &str) -> String {
    sha1_smol::Sha1::from(s).digest().to_string()
}

pub fn cache_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("aqua/store")
}

fn fresh(p: &Path, ttl: Duration) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < ttl)
}

impl Default for Http {
    fn default() -> Self {
        Http::new(cache_root())
    }
}

impl Http {
    pub fn new(cache_dir: PathBuf) -> Http {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(8))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("aqua-store/", env!("CARGO_PKG_VERSION")))
            .build();
        Http { agent, cache_dir }
    }

    fn path(&self, kind: &str, url: &str, ext: &str) -> PathBuf {
        self.cache_dir.join(kind).join(format!("{}{ext}", hash(url)))
    }

    pub fn get_json(&self, url: &str, ttl: Duration) -> Result<Value, String> {
        let p = self.path("json", url, ".json");
        if fresh(&p, ttl) {
            if let Some(v) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                return Ok(v);
            }
        }
        match self.fetch(url) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                write_atomic(&p, &bytes);
                Ok(v)
            }
            Err(e) => std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(e),
        }
    }

    pub fn post_json(&self, url: &str, body: &Value) -> Result<Value, String> {
        let r = self.agent.post(url).set("Content-Type", "application/json").send_string(&body.to_string());
        match r {
            Ok(resp) => resp.into_json::<Value>().map_err(|e| e.to_string()),
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Err(format!("HTTP {code}: {}", text.chars().take(200).collect::<String>()))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn post_json_cached(&self, url: &str, body: &Value, ttl: Duration) -> Result<Value, String> {
        let key = format!("{url}#{body}");
        let p = self.path("json", &key, ".json");
        if fresh(&p, ttl) {
            if let Some(v) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                return Ok(v);
            }
        }
        match self.post_json(url, body) {
            Ok(v) => {
                write_atomic(&p, v.to_string().as_bytes());
                Ok(v)
            }
            Err(e) => std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(e),
        }
    }

    pub fn expire_json(&self) {
        let Ok(rd) = std::fs::read_dir(self.cache_dir.join("json")) else { return };
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86400);
        for e in rd.flatten() {
            if let Ok(f) = std::fs::File::options().write(true).open(e.path()) {
                let _ = f.set_modified(old);
            }
        }
    }

    pub fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        match self.agent.get(url).call() {
            Ok(resp) => {
                let mut buf = vec![];
                resp.into_reader().take(64 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf)
            }
            Err(ureq::Error::Status(code, _)) => Err(format!("HTTP {code}")),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn file(&self, url: &str) -> Result<PathBuf, String> {
        let ext = Path::new(url.split('?').next().unwrap_or(url))
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .filter(|e| e.len() <= 5)
            .unwrap_or_default();
        let p = self.path("files", url, &ext);
        if p.exists() {
            return Ok(p);
        }
        let bytes = self.fetch(url)?;
        write_atomic(&p, &bytes);
        Ok(p)
    }
}

pub fn write_atomic(p: &Path, bytes: &[u8]) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

pub fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode() {
        assert_eq!(urlencode("The GIMP team"), "The%20GIMP%20team");
        assert_eq!(urlencode("a/b&c"), "a%2Fb%26c");
        assert_eq!(hash("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn stale_cache_is_used_when_offline() {
        let dir = std::env::temp_dir().join(format!("aqua-store-http-{}", std::process::id()));
        let h = Http::new(dir.clone());
        let url = "http://127.0.0.1:9/never";
        write_atomic(&h.path("json", url, ".json"), br#"{"ok":1}"#);
        let v = h.get_json(url, Duration::from_secs(0)).unwrap();
        assert_eq!(v["ok"], 1);
        assert!(h.get_json("http://127.0.0.1:9/other", Duration::from_secs(0)).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
