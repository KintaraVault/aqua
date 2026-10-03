//! UI translations for the Rust-drawn parts of Aqua (menu bar, Dock, panels, compositor
//! messages). Catalogs are gettext `.po` files compiled into the binary; Slint apps use
//! Slint's own bundled translations (`crates/aqua-ui/translations`).
//!
//! The language comes from `AQUA_LANG`, then `LC_ALL`, `LC_MESSAGES`, `LANG`; English is the
//! source language. Untranslated strings fall back to English.
use std::collections::HashMap;
use std::sync::OnceLock;

/// Bundled catalogs: (language, `.po` source).
const CATALOGS: &[(&str, &str)] = &[("ru", include_str!("../po/ru.po"))];

static ACTIVE: OnceLock<Catalog> = OnceLock::new();

#[derive(Default)]
pub struct Catalog {
    lang: String,
    map: HashMap<String, String>,
    /// `msgid` → `msgstr[0..]` for entries with `msgid_plural`.
    plurals: HashMap<String, Vec<String>>,
}

impl Catalog {
    /// Catalog for `lang` ("ru", "ru_RU.UTF-8", …); empty for English or unknown languages.
    pub fn for_lang(lang: &str) -> Self {
        let base = base_lang(lang);
        let (map, plurals) =
            CATALOGS.iter().find(|(l, _)| *l == base).map(|(_, po)| parse_po_full(po)).unwrap_or_default();
        Self { lang: if map.is_empty() { "en".into() } else { base.to_string() }, map, plurals }
    }
    /// Plural lookup: the translated form of `one`/`other` for `n` (English rules when the
    /// catalog has no plural entry for `one`).
    pub fn nget<'a>(&'a self, one: &'a str, other: &'a str, n: i64) -> &'a str {
        if let Some(forms) = self.plurals.get(one) {
            let idx = if self.lang == "ru" { plural_ru(n, 0, 1, 2) } else { usize::from(n != 1) };
            if let Some(f) = forms.get(idx).or(forms.last()).filter(|f| !f.is_empty()) {
                return f;
            }
        }
        if n == 1 {
            one
        } else {
            other
        }
    }
    pub fn get<'a>(&'a self, s: &'a str) -> &'a str {
        self.map.get(s).map(String::as_str).unwrap_or(s)
    }
    pub fn lang(&self) -> &str {
        &self.lang
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// "ru_RU.UTF-8" → "ru"; "C"/"POSIX" → "en".
pub fn base_lang(l: &str) -> &str {
    let b = l.split(['_', '.', '@', '-']).next().unwrap_or("");
    if b.is_empty() || b == "C" || b == "POSIX" {
        "en"
    } else {
        b
    }
}

/// The language requested by the environment.
pub fn env_lang() -> String {
    for k in ["AQUA_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    "en".into()
}

fn active() -> &'static Catalog {
    ACTIVE.get_or_init(|| Catalog::for_lang(&env_lang()))
}

/// Choose the language before the first `tr` call (later calls are ignored).
pub fn init(lang: &str) {
    let _ = ACTIVE.set(Catalog::for_lang(lang));
}

/// Current UI language ("en", "ru").
pub fn lang() -> &'static str {
    active().lang()
}

/// Translate an English UI string.
pub fn tr(s: &str) -> &str {
    let c = active();
    match c.map.get(s) {
        Some(t) => t.as_str(),
        None => s,
    }
}

/// Translate a counted phrase (`msgid` / `msgid_plural` in the catalog) and fill `{n}` with
/// `n`, e.g. `ntr("{n} item", "{n} items", 3)`.
pub fn ntr(one: &str, other: &str, n: i64) -> String {
    active().nget(one, other, n).replace("{n}", &n.to_string())
}

/// Translate and fill `{name}` placeholders.
pub fn trf(s: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = tr(s).to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), &v.to_string());
    }
    out
}

/// Pick the plural form for `n`: English (one, other); Russian uses one/few/many.
pub fn plural<T>(n: i64, one: T, few: T, many: T) -> T {
    if lang() == "ru" {
        plural_ru(n, one, few, many)
    } else if n == 1 {
        one
    } else {
        many
    }
}

pub fn plural_ru<T>(n: i64, one: T, few: T, many: T) -> T {
    let n = n.abs();
    match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    }
}

/// Minimal `.po` reader: `msgid`/`msgstr` pairs with continuation lines; skips the header,
/// fuzzy entries and empty translations.
pub fn parse_po(src: &str) -> HashMap<String, String> {
    parse_po_full(src).0
}

type PluralMap = HashMap<String, Vec<String>>;

/// Like [`parse_po`], plus the plural entries (`msgid_plural` + `msgstr[N]`), keyed by `msgid`.
pub fn parse_po_full(src: &str) -> (HashMap<String, String>, PluralMap) {
    #[derive(Default)]
    struct Entry {
        id: Option<String>,
        st: Option<String>,
        forms: Vec<String>,
        plural: bool,
        fuzzy: bool,
    }
    #[derive(Clone, Copy)]
    enum Target {
        Id,
        IdPlural,
        Str,
        Form(usize),
    }
    let mut map = HashMap::new();
    let mut plurals = HashMap::new();
    let mut flush = |e: &mut Entry| {
        let e = std::mem::take(e);
        let Some(i) = e.id.filter(|i| !i.is_empty()) else { return };
        if e.fuzzy {
            return;
        }
        if e.plural {
            if e.forms.iter().all(|f| !f.is_empty()) && !e.forms.is_empty() {
                plurals.insert(i, e.forms);
            }
        } else if let Some(s) = e.st.filter(|s| !s.is_empty()) {
            map.insert(i, s);
        }
    };
    let mut e = Entry::default();
    let mut fuzzy_next = false;
    let mut target = Target::Id;
    for line in src.lines().map(str::trim) {
        if let Some(flags) = line.strip_prefix("#,") {
            fuzzy_next |= flags.split(',').any(|f| f.trim() == "fuzzy");
        } else if line.is_empty() || line.starts_with('#') {
        } else if let Some(rest) = line.strip_prefix("msgid_plural ") {
            e.plural = true;
            let _ = unquote(rest);
            target = Target::IdPlural;
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            flush(&mut e);
            e.id = Some(unquote(rest));
            e.fuzzy = std::mem::take(&mut fuzzy_next);
            target = Target::Id;
        } else if let Some(rest) = line.strip_prefix("msgstr[") {
            let (idx, val) = rest.split_once(']').unwrap_or(("0", rest));
            let idx: usize = idx.trim().parse().unwrap_or(0);
            if e.forms.len() <= idx {
                e.forms.resize(idx + 1, String::new());
            }
            e.forms[idx] = unquote(val);
            target = Target::Form(idx);
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            e.st = Some(unquote(rest));
            target = Target::Str;
        } else if line.starts_with('"') {
            let s = unquote(line);
            match target {
                Target::Id => e.id.get_or_insert_default().push_str(&s),
                Target::IdPlural => {}
                Target::Str => e.st.get_or_insert_default().push_str(&s),
                Target::Form(i) => e.forms[i].push_str(&s),
            }
        }
    }
    flush(&mut e);
    (map, plurals)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn po_parsing() {
        let po = r#"
msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

msgid "Open"
msgstr "Открыть"

#, fuzzy
msgid "Maybe"
msgstr "Может быть"

msgid "Long "
"line"
msgstr "Длинная "
"строка"

msgid "Quote \"x\""
msgstr "Кавычка «x»"

msgid "Empty"
msgstr ""
"#;
        let m = parse_po(po);
        assert_eq!(m.get("Open").map(String::as_str), Some("Открыть"));
        assert_eq!(m.get("Long line").map(String::as_str), Some("Длинная строка"));
        assert_eq!(m.get("Quote \"x\"").map(String::as_str), Some("Кавычка «x»"));
        assert!(!m.contains_key("Maybe"));
        assert!(!m.contains_key("Empty"));
        assert!(!m.contains_key(""));
    }

    #[test]
    fn language_detection() {
        assert_eq!(base_lang("ru_RU.UTF-8"), "ru");
        assert_eq!(base_lang("C.UTF-8"), "en");
        assert_eq!(base_lang("en-US"), "en");
        assert_eq!(base_lang(""), "en");
        assert_eq!(Catalog::for_lang("ru_RU.UTF-8").lang(), "ru");
        assert_eq!(Catalog::for_lang("de_DE").lang(), "en");
        assert_eq!(Catalog::for_lang("de_DE").get("Open"), "Open");
    }

    #[test]
    fn russian_catalog_is_complete_and_sane() {
        let c = Catalog::for_lang("ru");
        assert!(c.len() > 100, "catalog has {} entries", c.len());
        assert_eq!(c.get("Quit"), "Завершить");
        for (k, v) in &c.map {
            let ph = |s: &str| {
                let mut v: Vec<String> =
                    s.split('{').skip(1).filter_map(|p| p.split_once('}')).map(|p| p.0.to_string()).collect();
                v.sort();
                v
            };
            assert_eq!(ph(k), ph(v), "placeholders differ in {k:?}");
        }
    }

    #[test]
    fn slint_catalog_matches() {
        let ui = parse_po(include_str!("../../aqua-ui/translations/ru/LC_MESSAGES/aqua-ui.po"));
        assert!(ui.len() > 200, "{}", ui.len());
        let shell = Catalog::for_lang("ru");
        for (k, v) in &ui {
            assert!(!v.is_empty(), "{k}");
            if let Some(s) = shell.map.get(k) {
                assert_eq!(s, v, "Slint and shell translate {k:?} differently");
            }
        }
    }

    #[test]
    fn po_plural_entries() {
        let po = r#"
msgid "{n} item"
msgid_plural "{n} items"
msgstr[0] "{n} объект"
msgstr[1] "{n} объекта"
msgstr[2] "{n} "
"объектов"
"#;
        let (map, pl) = parse_po_full(po);
        assert!(map.is_empty());
        let c = Catalog { lang: "ru".into(), map, plurals: pl };
        assert_eq!(c.nget("{n} item", "{n} items", 1), "{n} объект");
        assert_eq!(c.nget("{n} item", "{n} items", 3), "{n} объекта");
        assert_eq!(c.nget("{n} item", "{n} items", 11), "{n} объектов");
        let en = Catalog::for_lang("en");
        assert_eq!(en.nget("{n} item", "{n} items", 1), "{n} item");
        assert_eq!(en.nget("{n} item", "{n} items", 0), "{n} items");
    }

    #[test]
    fn russian_plurals() {
        let f = |n| plural_ru(n, "файл", "файла", "файлов");
        assert_eq!(
            [1, 2, 5, 11, 12, 21, 22, 25, 111, 101].map(f),
            ["файл", "файла", "файлов", "файлов", "файлов", "файл", "файла", "файлов", "файлов", "файл"]
        );
    }
}
