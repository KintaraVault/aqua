//! Renaming several items at once: replace text, add text, or format with an index.
use super::fs::split_ext;

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Replace { find: String, with: String },
    Add { text: String, after: bool },
    Format { style: Style, base: String, start: u64, after: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Index,
    Counter,
    Date,
}

/// New names for `names` (in order). Extensions are kept for Add and Format; `date` is the
/// text used for Style::Date (e.g. "2025-06-18 at 12.57.03").
pub fn apply(names: &[String], rule: &Rule, date: &dyn Fn(usize) -> String) -> Vec<String> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (stem, ext) = split_ext(n);
            match rule {
                Rule::Replace { find, with } if !find.is_empty() => n.replace(find.as_str(), with),
                Rule::Replace { .. } => n.clone(),
                Rule::Add { text, after: true } => format!("{stem}{text}{ext}"),
                Rule::Add { text, after: false } => format!("{text}{n}"),
                Rule::Format { style, base, start, after } => {
                    let k = start + i as u64;
                    let tag = match style {
                        Style::Index => k.to_string(),
                        Style::Counter => format!("{k:05}"),
                        Style::Date => date(i),
                    };
                    let base = if base.trim().is_empty() { stem.to_string() } else { base.trim().to_string() };
                    if *after {
                        format!("{base} {tag}{ext}")
                    } else {
                        format!("{tag} {base}{ext}")
                    }
                }
            }
        })
        .collect()
}

/// Why a set of new names can't be used (first problem), or None.
pub fn problem(old: &[String], new: &[String], exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for (o, n) in old.iter().zip(new) {
        let t = n.trim();
        if t.is_empty() || t == "." || t == ".." {
            return Some(crate::tr("A name can't be empty.").into());
        }
        if t.contains('/') {
            return Some(crate::tr("The name can't contain “/”.").into());
        }
        if !seen.insert(t.to_string()) {
            return Some(crate::trf("The name “{name}” is already taken.", &[("name", &t)]));
        }
        if t != o && !old.iter().any(|x| x == t) && exists(t) {
            return Some(crate::trf("The name “{name}” is already taken.", &[("name", &t)]));
        }
    }
    None
}

/// Order renames so that no step overwrites a name another step still has to free.
pub fn order(pairs: &[(String, String)]) -> Option<Vec<(String, String)>> {
    let mut left: Vec<(String, String)> = pairs.iter().filter(|(a, b)| a != b).cloned().collect();
    let mut out = vec![];
    while !left.is_empty() {
        let pos = left.iter().position(|(_, to)| !left.iter().any(|(from, _)| from == to))?;
        out.push(left.remove(pos));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn replace_add_format() {
        let names = s(&["IMG_001.jpg", "IMG_002.jpg", "notes"]);
        let d = |i: usize| format!("D{i}");
        assert_eq!(
            apply(&names, &Rule::Replace { find: "IMG_".into(), with: "Trip ".into() }, &d),
            s(&["Trip 001.jpg", "Trip 002.jpg", "notes"])
        );
        assert_eq!(apply(&names, &Rule::Replace { find: "".into(), with: "x".into() }, &d), names);
        assert_eq!(
            apply(&names, &Rule::Add { text: " (old)".into(), after: true }, &d),
            s(&["IMG_001 (old).jpg", "IMG_002 (old).jpg", "notes (old)"])
        );
        assert_eq!(
            apply(&names, &Rule::Add { text: "x-".into(), after: false }, &d),
            s(&["x-IMG_001.jpg", "x-IMG_002.jpg", "x-notes"])
        );
        assert_eq!(
            apply(&names, &Rule::Format { style: Style::Index, base: "Beach".into(), start: 1, after: true }, &d),
            s(&["Beach 1.jpg", "Beach 2.jpg", "Beach 3"])
        );
        assert_eq!(
            apply(&names, &Rule::Format { style: Style::Counter, base: "B".into(), start: 9, after: false }, &d),
            s(&["00009 B.jpg", "00010 B.jpg", "00011 B"])
        );
        assert_eq!(
            apply(&names[..1], &Rule::Format { style: Style::Date, base: "".into(), start: 1, after: true }, &d),
            s(&["IMG_001 D0.jpg"])
        );
    }

    #[test]
    fn problems() {
        let old = s(&["a", "b"]);
        assert!(problem(&old, &s(&["x", "x"]), &|_| false).is_some());
        assert!(problem(&old, &s(&["x", ""]), &|_| false).is_some());
        assert!(problem(&old, &s(&["x/y", "z"]), &|_| false).is_some());
        assert!(problem(&old, &s(&["taken", "z"]), &|n| n == "taken").is_some());
        assert!(problem(&old, &s(&["b", "a"]), &|_| true).is_none());
        assert!(problem(&old, &s(&["x", "y"]), &|_| false).is_none());
    }

    #[test]
    fn ordering_avoids_overwrites() {
        let p = vec![("a".to_string(), "b".to_string()), ("b".to_string(), "c".to_string())];
        assert_eq!(order(&p).unwrap(), vec![("b".to_string(), "c".to_string()), ("a".to_string(), "b".to_string())]);
        let swap = vec![("a".to_string(), "b".to_string()), ("b".to_string(), "a".to_string())];
        assert!(order(&swap).is_none());
        assert!(order(&[("a".to_string(), "a".to_string())]).unwrap().is_empty());
    }
}
