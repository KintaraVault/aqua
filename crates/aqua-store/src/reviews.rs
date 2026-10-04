use crate::http::{urlencode, Http};
use crate::model::{Ratings, Review};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub const ODRS: &str = "https://odrs.gnome.org/1.0/reviews/api";

pub struct Reviews {
    pub http: Arc<Http>,
    pub user_hash: String,
    pub distro: String,
    pub locale: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetched {
    pub reviews: Vec<Review>,
    pub user_skey: String,
}

pub fn machine_hash(machine_id: &str, user: &str) -> String {
    sha1_smol::Sha1::from(format!("gnome-software[{user}:{}]", machine_id.trim())).digest().to_string()
}

pub fn parse_ratings(v: &Value) -> Ratings {
    let mut r = Ratings::default();
    for i in 0..5 {
        r.stars[i] = v.get(format!("star{}", i + 1)).and_then(|x| x.as_u64()).unwrap_or(0);
    }
    r
}

pub fn parse_reviews(v: &Value, user_hash: &str) -> Fetched {
    let mut f = Fetched::default();
    let Some(a) = v.as_array() else { return f };
    for r in a {
        let s = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        if f.user_skey.is_empty() {
            f.user_skey = s("user_skey");
        }
        let Some(id) = r.get("review_id").and_then(|x| x.as_i64()) else { continue };
        let author = s("user_display");
        f.reviews.push(Review {
            id,
            author: if author.is_empty() { "Anonymous".into() } else { author },
            summary: s("summary"),
            body: s("description"),
            rating: r.get("rating").and_then(|x| x.as_u64()).unwrap_or(0).min(100) as u8,
            date: r.get("date_created").and_then(|x| x.as_f64()).unwrap_or(0.0) as i64,
            version: s("version"),
            distro: s("distro"),
            karma_up: r.get("karma_up").and_then(|x| x.as_i64()).unwrap_or(0),
            karma_down: r.get("karma_down").and_then(|x| x.as_i64()).unwrap_or(0),
            mine: !user_hash.is_empty() && s("user_hash") == user_hash,
        });
    }
    f
}

pub fn current_locale() -> String {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            let base = v.split('.').next().unwrap_or("").to_string();
            if !base.is_empty() && base != "C" && base != "POSIX" {
                return base;
            }
        }
    }
    "en_US".into()
}

impl Reviews {
    pub fn new(http: Arc<Http>, distro: &str) -> Reviews {
        let mid = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
        let user = std::env::var("USER").unwrap_or_default();
        Reviews { http, user_hash: machine_hash(&mid, &user), distro: distro.into(), locale: current_locale() }
    }

    pub fn ratings(&self, app_id: &str) -> Result<Ratings, String> {
        self.http
            .get_json(&format!("{ODRS}/ratings/{}", urlencode(app_id)), Duration::from_secs(6 * 3600))
            .map(|v| parse_ratings(&v))
    }

    pub fn fetch(&self, app_id: &str, version: &str, limit: u32) -> Result<Fetched, String> {
        let body = json!({
            "app_id": app_id,
            "locale": self.locale,
            "distro": self.distro,
            "version": if version.is_empty() { "unknown" } else { version },
            "user_hash": self.user_hash,
            "limit": limit,
        });
        self.http
            .post_json_cached(&format!("{ODRS}/fetch"), &body, Duration::from_secs(3600))
            .map(|v| parse_reviews(&v, &self.user_hash))
    }

    pub fn submit(
        &self,
        app_id: &str,
        skey: &str,
        version: &str,
        author: &str,
        summary: &str,
        body: &str,
        stars: u8,
    ) -> Result<(), String> {
        let v = json!({
            "user_hash": self.user_hash,
            "user_skey": skey,
            "app_id": app_id,
            "locale": self.locale,
            "summary": summary,
            "description": body,
            "user_display": author,
            "version": if version.is_empty() { "unknown" } else { version },
            "distro": self.distro,
            "rating": (stars.clamp(1, 5) as u32) * 20,
        });
        let r = self.http.post_json(&format!("{ODRS}/submit"), &v)?;
        check(&r)
    }

    pub fn vote(&self, app_id: &str, skey: &str, review_id: i64, kind: &str) -> Result<(), String> {
        let v = json!({ "user_hash": self.user_hash, "user_skey": skey, "app_id": app_id, "review_id": review_id });
        let r = self.http.post_json(&format!("{ODRS}/{kind}"), &v)?;
        check(&r)
    }
}

fn check(r: &Value) -> Result<(), String> {
    if r.get("success").and_then(|x| x.as_bool()) == Some(false) {
        return Err(r.get("msg").and_then(|x| x.as_str()).unwrap_or("rejected").to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_and_reviews() {
        let v: Value =
            serde_json::from_str(r#"{"star0":0,"star1":143,"star2":49,"star3":39,"star4":83,"star5":472,"total":786}"#)
                .unwrap();
        let r = parse_ratings(&v);
        assert_eq!(r.total(), 786);
        assert!((r.average() - 3.88).abs() < 0.01);
        let f = parse_reviews(
            &serde_json::from_str(r#"[{"app_id":"a","review_id":5,"rating":80,"summary":"Good","description":"Nice","user_display":"","user_hash":"me","user_skey":"k","date_created":1789812238.0,"karma_up":2}]"#).unwrap(),
            "me",
        );
        assert_eq!(f.user_skey, "k");
        assert_eq!(f.reviews[0].stars(), 4);
        assert_eq!(f.reviews[0].author, "Anonymous");
        assert!(f.reviews[0].mine);
        let empty =
            parse_reviews(&serde_json::from_str(r#"[{"app_id":"a","user_skey":"z","user_hash":"me"}]"#).unwrap(), "me");
        assert!(empty.reviews.is_empty());
        assert_eq!(empty.user_skey, "z");
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(machine_hash("abc\n", "u"), machine_hash("abc", "u"));
        assert_eq!(machine_hash("abc", "u").len(), 40);
    }
}
