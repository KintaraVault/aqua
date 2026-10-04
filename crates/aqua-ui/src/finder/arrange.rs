//! Icon positions for folders whose items are placed by hand.
use super::layout::Grid;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folder {
    pub snap: bool,
    pub pos: HashMap<String, (f32, f32)>,
}

#[derive(Default)]
pub struct Arrangements {
    pub map: HashMap<String, Folder>,
    file: Option<PathBuf>,
}

pub fn default_file() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| super::fs::home().join(".local/share")).join("aqua/positions.json")
}

impl Arrangements {
    pub fn load(file: PathBuf) -> Self {
        let mut map = HashMap::new();
        let v: Value =
            std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for (dir, f) in v.as_object().into_iter().flatten() {
            let mut folder = Folder { snap: f["snap"].as_bool().unwrap_or(false), pos: HashMap::new() };
            for (name, p) in f["pos"].as_object().into_iter().flatten() {
                if let (Some(x), Some(y)) = (p[0].as_f64(), p[1].as_f64()) {
                    folder.pos.insert(name.clone(), (x as f32, y as f32));
                }
            }
            map.insert(dir.clone(), folder);
        }
        Arrangements { map, file: Some(file) }
    }

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        let v: serde_json::Map<String, Value> = self
            .map
            .iter()
            .map(|(d, f)| {
                let pos: serde_json::Map<String, Value> =
                    f.pos.iter().map(|(n, p)| (n.clone(), json!([p.0.round(), p.1.round()]))).collect();
                (d.clone(), json!({"snap": f.snap, "pos": pos}))
            })
            .collect();
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(file, Value::Object(v).to_string());
    }

    pub fn get(&self, dir: &Path) -> Option<&Folder> {
        self.map.get(dir.to_string_lossy().as_ref())
    }

    pub fn set(&mut self, dir: &Path, f: Option<Folder>) {
        let k = dir.to_string_lossy().into_owned();
        match f {
            Some(f) => self.map.insert(k, f),
            None => self.map.remove(&k),
        };
        self.save();
    }

    /// Keep positions when an item is renamed or moved inside the same folder.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        let (Some(fd), Some(fname), Some(td), Some(tname)) =
            (from.parent(), from.file_name(), to.parent(), to.file_name())
        else {
            return;
        };
        if fd != td {
            return;
        }
        if let Some(f) = self.map.get_mut(fd.to_string_lossy().as_ref()) {
            if let Some(p) = f.pos.remove(fname.to_string_lossy().as_ref()) {
                f.pos.insert(tname.to_string_lossy().into_owned(), p);
                self.save();
            }
        }
    }
}

pub fn slot(g: &Grid, i: usize) -> (f32, f32) {
    let cols = g.cols.max(1);
    (g.pad + (i % cols) as f32 * g.cell_w, (i / cols) as f32 * g.cell_h)
}

pub fn snap(g: &Grid, p: (f32, f32)) -> (f32, f32) {
    let c = ((p.0 - g.pad) / g.cell_w).round().max(0.0);
    let r = (p.1 / g.cell_h).round().max(0.0);
    (g.pad + c * g.cell_w, r * g.cell_h)
}

fn overlaps(g: &Grid, a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < g.cell_w * 0.6 && (a.1 - b.1).abs() < g.cell_h * 0.6
}

fn first_free(g: &Grid, taken: &[(f32, f32)], from: usize) -> (usize, (f32, f32)) {
    let mut k = from;
    loop {
        let s = slot(g, k);
        if !taken.iter().any(|t| overlaps(g, *t, s)) {
            return (k, s);
        }
        k += 1;
    }
}

/// Positions for `names` (in display order): saved ones first, the rest in the first free grid slots.
pub fn place(g: &Grid, names: &[String], f: &Folder) -> Vec<(f32, f32)> {
    let mut out: Vec<Option<(f32, f32)>> = names
        .iter()
        .map(|n| f.pos.get(n).map(|&p| if f.snap { snap(g, p) } else { (p.0.max(0.0), p.1.max(0.0)) }))
        .collect();
    let mut taken: Vec<(f32, f32)> = out.iter().flatten().copied().collect();
    let mut k = 0;
    for o in out.iter_mut().filter(|o| o.is_none()) {
        let (next, s) = first_free(g, &taken, k);
        k = next + 1;
        taken.push(s);
        *o = Some(s);
    }
    out.into_iter().flatten().collect()
}

/// Grid positions for `names` in the given order.
pub fn in_order(g: &Grid, names: &[String]) -> HashMap<String, (f32, f32)> {
    names.iter().enumerate().map(|(i, n)| (n.clone(), slot(g, i))).collect()
}

/// Snap every item to the nearest free grid cell, keeping the overall arrangement.
pub fn clean_up(g: &Grid, names: &[String], pos: &[(f32, f32)]) -> HashMap<String, (f32, f32)> {
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by(|&a, &b| pos[a].1.total_cmp(&pos[b].1).then(pos[a].0.total_cmp(&pos[b].0)));
    let mut taken: Vec<(f32, f32)> = vec![];
    let mut out = HashMap::new();
    for i in order {
        let mut s = snap(g, pos[i]);
        if taken.iter().any(|t| overlaps(g, *t, s)) {
            s = first_free(g, &taken, 0).1;
        }
        taken.push(s);
        out.insert(names[i].clone(), s);
    }
    out
}

/// Move the named items by (dx, dy).
pub fn shift(g: &Grid, f: &mut Folder, names: &[String], pos: &[(f32, f32)], moving: &[usize], d: (f32, f32)) {
    for (i, n) in names.iter().enumerate() {
        f.pos.entry(n.clone()).or_insert(pos[i]);
    }
    for &i in moving {
        let p = pos[i];
        let np = ((p.0 + d.0).max(0.0), (p.1 + d.1).max(0.0));
        f.pos.insert(names[i].clone(), if f.snap { snap(g, np) } else { np });
    }
}

/// Content height needed to show all positions.
pub fn extent(g: &Grid, pos: &[(f32, f32)]) -> f32 {
    pos.iter().map(|p| p.1 + g.cell_h).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::new(600.0, 64.0, 0.0, 12.0, false)
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unplaced_items_fill_free_slots() {
        let g = grid();
        let mut f = Folder::default();
        f.pos.insert("b".into(), slot(&g, 0));
        let p = place(&g, &names(&["a", "b", "c"]), &f);
        assert_eq!(p[1], slot(&g, 0));
        assert_eq!(p[0], slot(&g, 1));
        assert_eq!(p[2], slot(&g, 2));
    }

    #[test]
    fn snapping_and_cleanup() {
        let g = grid();
        let s = snap(&g, (g.pad + g.cell_w * 1.4, g.cell_h * 0.6));
        assert_eq!(s, (g.pad + g.cell_w, g.cell_h));
        let n = names(&["a", "b"]);
        let pos = [(g.pad + 3.0, 2.0), (g.pad + 10.0, 5.0)];
        let c = clean_up(&g, &n, &pos);
        assert_ne!(c["a"], c["b"]);
        assert!(c.values().all(|p| *p == snap(&g, *p)));
    }

    #[test]
    fn moving_and_persisting() {
        let g = grid();
        let n = names(&["a", "b"]);
        let pos = place(&g, &n, &Folder::default());
        let mut f = Folder::default();
        shift(&g, &mut f, &n, &pos, &[1], (40.0, 300.0));
        assert_eq!(f.pos["b"], (pos[1].0 + 40.0, pos[1].1 + 300.0));
        assert_eq!(f.pos["a"], pos[0]);
        assert!(extent(&g, &place(&g, &n, &f)) >= pos[1].1 + 300.0);
        let file = std::env::temp_dir().join(format!("aqua-arrange-{}.json", std::process::id()));
        let mut a = Arrangements::load(file.clone());
        a.set(Path::new("/x"), Some(f.clone()));
        a.renamed(Path::new("/x/b"), Path::new("/x/c"));
        let b = Arrangements::load(file.clone());
        assert!(b.get(Path::new("/x")).unwrap().pos.contains_key("c"));
        let _ = std::fs::remove_file(file);
    }
}
