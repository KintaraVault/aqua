//! Secret Service access. Call only from a worker, and fetch only after an explicit selection.
use secret_service::{blocking::SecretService, EncryptionType};
use std::collections::HashMap;
use zeroize::Zeroizing;

pub struct Entry {
    pub path: String,
    pub label: String,
}
fn attributes(app: &str) -> HashMap<&str, &str> {
    HashMap::from([("aqua-app-id", app), ("xdg:schema", "org.aqua.Password")])
}
pub fn list(app: &str) -> Result<Vec<Entry>, secret_service::Error> {
    let service = SecretService::connect(EncryptionType::Dh)?;
    let items = service.search_items(attributes(app))?;
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
    let service = SecretService::connect(EncryptionType::Dh)?;
    let items = service.search_items(attributes(app))?;
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
