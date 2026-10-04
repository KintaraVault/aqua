use crate::flatpak::app_of_export;
use crate::model::{Icon, Origin, Package};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub versions: HashMap<String, String>,
    pub apps: Vec<Package>,
}

impl Index {
    pub fn contains(&self, key: &str) -> bool {
        self.versions.contains_key(key)
    }

    pub fn version(&self, key: &str) -> Option<&str> {
        self.versions.get(key).map(String::as_str)
    }

    pub fn app(&self, key: &str) -> Option<&Package> {
        self.apps.iter().find(|p| p.key() == key)
    }
}

pub struct Desktop {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub path: PathBuf,
    pub categories: Vec<String>,
}

pub fn desktop_entries() -> Vec<Desktop> {
    aqua_apps::scan()
        .into_iter()
        .map(|a| Desktop { id: a.id, name: a.name, icon: a.icon, path: a.path, categories: a.categories })
        .collect()
}

pub fn icon_of(s: &str) -> Icon {
    if s.is_empty() {
        Icon::None
    } else if s.starts_with('/') {
        Icon::Path(PathBuf::from(s))
    } else {
        Icon::Named(s.to_string())
    }
}

pub fn build(
    desktops: &[Desktop],
    flatpaks: &[Package],
    native: Option<(Origin, &[Package])>,
    foreign: &HashSet<String>,
    owners: &HashMap<PathBuf, String>,
) -> Index {
    let mut idx = Index::default();
    for p in flatpaks {
        idx.versions.insert(p.key(), p.installed_version.clone());
    }
    let mut native_map: HashMap<&str, &Package> = HashMap::new();
    if let Some((o, list)) = native {
        for p in list {
            let origin = if o == Origin::Pacman && foreign.contains(&p.name) { Origin::Aur } else { o };
            idx.versions.insert(crate::model::key_of(origin, &p.name), p.installed_version.clone());
            native_map.insert(p.name.as_str(), p);
        }
    }
    let fp: HashMap<&str, &Package> = flatpaks.iter().map(|p| (p.name.as_str(), p)).collect();
    let mut seen = HashSet::new();
    for d in desktops {
        let (mut pkg, origin) = if let Some(app) = app_of_export(&d.path) {
            let Some(f) = fp.get(app.as_str()) else { continue };
            ((*f).clone(), Origin::Flatpak)
        } else if let (Some(owner), Some((o, _))) = (owners.get(&d.path), native) {
            let origin = if o == Origin::Pacman && foreign.contains(owner) { Origin::Aur } else { o };
            let mut p =
                native_map.get(owner.as_str()).map(|p| (*p).clone()).unwrap_or_else(|| Package::new(origin, owner));
            p.origin = Some(origin);
            (p, origin)
        } else {
            continue;
        };
        let key = crate::model::key_of(origin, &pkg.name);
        if !seen.insert(key) {
            continue;
        }
        pkg.title = d.name.clone();
        pkg.desktop_id = d.id.clone();
        pkg.icon = icon_of(&d.icon);
        pkg.installed = true;
        pkg.is_app = true;
        if pkg.categories.is_empty() {
            pkg.categories = d.categories.clone();
        }
        idx.apps.push(pkg);
    }
    idx.apps.sort_by_key(|p| p.display_name().to_lowercase());
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_index() {
        let d = |id: &str, name: &str, path: &str| Desktop {
            id: id.into(),
            name: name.into(),
            icon: "x".into(),
            path: path.into(),
            categories: vec![],
        };
        let desktops = vec![
            d("org.gimp.GIMP", "GIMP", "/var/lib/flatpak/exports/share/applications/org.gimp.GIMP.desktop"),
            d("vim", "Vim", "/usr/share/applications/vim.desktop"),
            d("yay-app", "Yay App", "/usr/share/applications/yay-app.desktop"),
            d("orphan", "Orphan", "/home/u/.local/share/applications/orphan.desktop"),
            d("ghost", "Ghost", "/var/lib/flatpak/exports/share/applications/com.ghost.App.desktop"),
        ];
        let mut f = Package::new(Origin::Flatpak, "org.gimp.GIMP");
        f.installed_version = "3.0".into();
        let mut v = Package::new(Origin::Pacman, "vim");
        v.installed_version = "9.1".into();
        let y = Package::new(Origin::Pacman, "yay-app");
        let natives = vec![v, y];
        let foreign: HashSet<String> = ["yay-app".to_string()].into();
        let owners: HashMap<PathBuf, String> = [
            (PathBuf::from("/usr/share/applications/vim.desktop"), "vim".to_string()),
            (PathBuf::from("/usr/share/applications/yay-app.desktop"), "yay-app".to_string()),
        ]
        .into();
        let idx = build(&desktops, &[f], Some((Origin::Pacman, &natives)), &foreign, &owners);
        let keys: Vec<String> = idx.apps.iter().map(|p| p.key()).collect();
        assert_eq!(keys, vec!["flatpak:org.gimp.GIMP", "pacman:vim", "aur:yay-app"]);
        assert_eq!(idx.version("pacman:vim"), Some("9.1"));
        assert!(idx.contains("aur:yay-app"));
        assert!(!idx.contains("pacman:yay-app"));
        assert_eq!(idx.apps[1].title, "Vim");
    }
}
