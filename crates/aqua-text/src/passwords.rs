//! Secret Service access. Call only from a worker, and fetch only after an explicit selection.
use secret_service::{blocking::SecretService, EncryptionType};
use std::collections::HashMap;
use zeroize::Zeroizing;

pub struct Entry {
    pub path: String,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    App(String),
    Origin(String),
}
impl Scope {
    fn attributes(&self) -> HashMap<&str, &str> {
        let (key, value) = match self {
            Self::App(value) => ("aqua-app-id", value),
            Self::Origin(value) => ("aqua-origin", value),
        };
        HashMap::from([(key, value.as_str()), ("xdg:schema", "org.aqua.Password")])
    }
}

pub fn list(app: &str) -> Result<Vec<Entry>, secret_service::Error> {
    list_scoped(&Scope::App(app.into()))
}
pub fn list_scoped(scope: &Scope) -> Result<Vec<Entry>, secret_service::Error> {
    let service = SecretService::connect(EncryptionType::Dh)?;
    let items = service.search_items(scope.attributes())?;
    items
        .unlocked
        .into_iter()
        .chain(items.locked)
        .take(32)
        .map(|item| {
            Ok(Entry {
                path: item.item_path.to_string(),
                label: item.get_label()?.chars().filter(|c| !c.is_control()).take(80).collect(),
            })
        })
        .collect()
}
pub fn fetch(app: &str, path: &str) -> Result<Option<Zeroizing<String>>, secret_service::Error> {
    fetch_scoped(&Scope::App(app.into()), path)
}
pub fn fetch_scoped(scope: &Scope, path: &str) -> Result<Option<Zeroizing<String>>, secret_service::Error> {
    let service = SecretService::connect(EncryptionType::Dh)?;
    let items = service.search_items(scope.attributes())?;
    for item in items.unlocked.into_iter().chain(items.locked).take(32) {
        if item.item_path.as_str() != path {
            continue;
        }
        item.unlock()?;
        let bytes = Zeroizing::new(item.get_secret()?);
        if bytes.len() > crate::MAX_TEXT {
            return Ok(None);
        }
        return Ok(std::str::from_utf8(&bytes).ok().map(|s| Zeroizing::new(s.to_owned())));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_and_origin_never_share_a_search_scope() {
        let app = Scope::App("firefox".into());
        let origin = Scope::Origin("https://example.org".into());
        assert!(!origin.attributes().contains_key("aqua-app-id"));
        assert!(!app.attributes().contains_key("aqua-origin"));
        assert_eq!(origin.attributes()["aqua-origin"], "https://example.org");
    }
}
