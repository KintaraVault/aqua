//! Bounded, Unicode-aware completion and focus-bound text delivery.
pub mod accessibility;
pub mod passwords;
use fst::{automaton::Str, Automaton, IntoStreamer, Map, MapBuilder, Streamer};
use std::collections::BTreeMap;
use symspell::{SymSpell, UnicodeStringStrategy, Verbosity};
use unicode_segmentation::UnicodeSegmentation;
use zeroize::Zeroizing;

pub const MAX_TEXT: usize = 4096;
pub const MAX_WORD: usize = 64;
pub const MAX_WORDS: usize = 100_000;
pub const MAX_CANDIDATES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
}
impl Shape {
    /// A caret may have zero width. Invalid rectangles must not be invented or clamped into existence.
    pub fn validated_caret(self) -> Option<Self> {
        let Self { x, y, w, h, .. } = self;
        ([x, y, w, h].iter().all(|v| v.is_finite())
            && x.abs() <= 100_000.0
            && y.abs() <= 100_000.0
            && (0.0..=100_000.0).contains(&w)
            && h > 0.0
            && h <= 100_000.0)
            .then_some(Self { radius: 0.0, ..self })
    }

    pub fn validated(self) -> Option<Self> {
        let Self { x, y, w, h, radius } = self;
        ([x, y, w, h, radius].iter().all(|v| v.is_finite())
            && x.abs() <= 100_000.0
            && y.abs() <= 100_000.0
            && w > 0.0
            && h > 0.0
            && w <= 100_000.0
            && h <= 100_000.0
            && radius >= 0.0)
            .then_some(Self { radius: radius.min(w.min(h) / 2.0), ..self })
    }
}

/// Cursor and anchor are UTF-8 byte offsets, as required by text-input-v3.
pub fn word_before_cursor(text: &str, cursor: i32, anchor: i32) -> Option<&str> {
    if text.len() > MAX_TEXT || cursor < 0 || cursor != anchor {
        return None;
    }
    let cursor = cursor as usize;
    if !text.is_char_boundary(cursor) || cursor > text.len() {
        return None;
    }
    if text[cursor..].chars().next().is_some_and(char::is_alphabetic) {
        return None;
    }
    let before = &text[..cursor];
    let begin = before
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_alphabetic() && *c != '\'')
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let word = &before[begin..];
    (word.chars().count() >= 2 && word.len() <= MAX_WORD).then_some(word)
}

pub struct Dictionary {
    words: Map<Vec<u8>>,
    spelling: SymSpell<UnicodeStringStrategy>,
    t9: Map<Vec<u8>>,
    t9_words: Vec<Vec<(String, u64)>>,
}
impl Dictionary {
    /// Word-frequency pairs. Limits apply before indexing; no text is learned from applications.
    pub fn from_words(entries: impl IntoIterator<Item = (String, u64)>) -> Self {
        let mut words = BTreeMap::new();
        for (word, frequency) in entries.into_iter().take(MAX_WORDS) {
            let word = word.to_lowercase();
            if word.is_empty() || word.len() > MAX_WORD || !word.chars().all(char::is_alphabetic) {
                continue;
            }
            words.entry(word).and_modify(|f: &mut u64| *f = (*f).max(frequency)).or_insert(frequency.max(1));
        }
        let mut builder = MapBuilder::memory();
        let mut spelling = SymSpell::default();
        let mut groups: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
        for (word, frequency) in &words {
            builder.insert(word, *frequency).expect("sorted dictionary");
            spelling.load_dictionary_line(&format!("{word} {}", (*frequency).min(i64::MAX as u64)), 0, 1, " ");
            if let Some(digits) = t9_key(word) {
                groups.entry(digits).or_default().push((word.clone(), *frequency));
            }
        }
        let mut t9_builder = MapBuilder::memory();
        let mut t9_words = Vec::new();
        for (digits, mut group) in groups {
            group.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            group.truncate(MAX_CANDIDATES);
            t9_builder.insert(digits, t9_words.len() as u64).expect("sorted T9");
            t9_words.push(group);
        }
        Self {
            words: Map::new(builder.into_inner().unwrap()).unwrap(),
            spelling,
            t9: Map::new(t9_builder.into_inner().unwrap()).unwrap(),
            t9_words,
        }
    }
    pub fn seed() -> Self {
        Self::from_words(include_str!("seed.txt").lines().filter_map(parse_line))
    }
    pub fn bundled() -> Self {
        Self::from_words(include_str!("words.txt").lines().filter_map(parse_line))
    }
    pub fn from_text(text: &str) -> Self {
        Self::from_words(text.lines().take(MAX_WORDS).filter_map(parse_line))
    }
    pub fn complete(&self, word: &str) -> Vec<String> {
        if word.len() < 2 || word.len() > MAX_WORD {
            return vec![];
        }
        let key = word.to_lowercase();
        let mut stream = self.words.search(Str::new(&key).starts_with()).into_stream();
        let mut found = Vec::new();
        let mut scanned = 0;
        while let Some((bytes, frequency)) = stream.next() {
            scanned += 1;
            if scanned > 2048 {
                break;
            }
            let value = String::from_utf8_lossy(bytes).into_owned();
            if value != key {
                found.push((value, frequency));
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        found.truncate(MAX_CANDIDATES);
        let mut result: Vec<_> = found.into_iter().map(|(s, _)| match_case(word, s)).collect();
        if result.len() < MAX_CANDIDATES {
            for suggestion in self.spelling.lookup(&key, Verbosity::Closest, 1) {
                if suggestion.term == key {
                    continue;
                }
                let suggestion = match_case(word, suggestion.term);
                if !result.contains(&suggestion) {
                    result.push(suggestion);
                }
                if result.len() == MAX_CANDIDATES {
                    break;
                }
            }
        }
        result
    }
    pub fn predict_t9(&self, digits: &str) -> Vec<String> {
        if digits.len() < 2 || digits.len() > MAX_WORD || !digits.bytes().all(|b| (b'2'..=b'9').contains(&b)) {
            return vec![];
        }
        let mut stream = self.t9.search(Str::new(digits).starts_with()).into_stream();
        let mut result = Vec::new();
        let mut scanned = 0;
        while let Some((_, index)) = stream.next() {
            scanned += 1;
            if scanned > 2048 {
                break;
            }
            result.extend(self.t9_words[index as usize].iter().cloned());
        }
        result.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        result.truncate(MAX_CANDIDATES);
        result.into_iter().map(|(s, _)| s).collect()
    }
}
fn parse_line(line: &str) -> Option<(String, u64)> {
    let mut parts = line.split_whitespace();
    let word = parts.next()?.to_string();
    let frequency = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    Some((word, frequency))
}
fn match_case(source: &str, value: String) -> String {
    if source.chars().all(char::is_uppercase) {
        value.to_uppercase()
    } else if source.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = value.chars();
        chars.next().map(|c| c.to_uppercase().to_string() + chars.as_str()).unwrap_or_default()
    } else {
        value
    }
}
fn t9_key(word: &str) -> Option<String> {
    word.chars()
        .map(|c| {
            Some(match c {
                'a'..='c' | 'а'..='г' => '2',
                'd'..='f' | 'д'..='з' | 'ё' => '3',
                'g'..='i' | 'и'..='л' => '4',
                'j'..='l' | 'м'..='п' => '5',
                'm'..='o' | 'р'..='у' => '6',
                'p'..='s' | 'ф'..='ч' => '7',
                't'..='v' | 'ш'..='ы' => '8',
                'w'..='z' | 'ь'..='я' => '9',
                _ => return None,
            })
        })
        .collect()
}

/// Each tick yields exactly one grapheme; secrets are wiped when the queue is dropped.
pub struct Typing {
    text: Zeroizing<String>,
    offset: usize,
    pub generation: u64,
}
impl Typing {
    pub fn new(text: Zeroizing<String>, generation: u64) -> Option<Self> {
        (!text.is_empty() && text.len() <= MAX_TEXT && !text.chars().any(|c| c == '\0')).then_some(Self {
            text,
            offset: 0,
            generation,
        })
    }
    pub fn next_grapheme(&mut self) -> Option<Zeroizing<String>> {
        let value = self.text[self.offset..].graphemes(true).next()?;
        let part = Zeroizing::new(value.to_owned());
        self.offset += value.len();
        Some(part)
    }
    pub fn finished(&self) -> bool {
        self.offset == self.text.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offsets_are_bytes_not_characters() {
        assert_eq!(word_before_cursor("Привет", 12, 12), Some("Привет"));
        assert_eq!(word_before_cursor("Привет", 3, 3), None);
        assert_eq!(word_before_cursor("hello", 99, 99), None);
        assert_eq!(word_before_cursor("hello", -1, -1), None);
    }
    #[test]
    fn no_selection_or_midword_completion() {
        assert_eq!(word_before_cursor("hello", 3, 5), None);
        assert_eq!(word_before_cursor("hello", 3, 3), None);
        assert_eq!(word_before_cursor("hello ", 6, 6), None);
    }
    #[test]
    fn ranked_prefix_and_spelling() {
        let d = Dictionary::from_words([("hello".into(), 100), ("help".into(), 20), ("helmet".into(), 1)]);
        assert_eq!(d.complete("hel"), vec!["hello", "help", "helmet"]);
        assert!(d.complete("helo").contains(&"hello".into()));
        assert_eq!(d.complete("HEL")[0], "HELLO");
    }
    #[test]
    fn unicode_and_t9() {
        let d = Dictionary::seed();
        assert!(d.complete("при").contains(&"привет".into()));
        assert!(d.predict_t9("43556").contains(&"hello".into()));
        assert!(d.predict_t9("1").is_empty());
    }
    #[test]
    fn graphemes_never_split_or_arrive_in_bulk() {
        let mut q = Typing::new(Zeroizing::new("a👨‍👩‍👧‍👦е\u{301}".into()), 42).unwrap();
        assert_eq!(&*q.next_grapheme().unwrap(), "a");
        assert_eq!(&*q.next_grapheme().unwrap(), "👨‍👩‍👧‍👦");
        assert_eq!(&*q.next_grapheme().unwrap(), "е\u{301}");
        assert!(q.finished());
        assert!(q.next_grapheme().is_none());
    }
    #[test]
    fn bounded_inputs() {
        assert!(Typing::new(Zeroizing::new("a".repeat(MAX_TEXT + 1)), 0).is_none());
        assert!(Dictionary::bundled().complete(&"a".repeat(MAX_WORD + 1)).is_empty());
        assert!(word_before_cursor(&"a".repeat(MAX_TEXT + 1), 5, 5).is_none());
    }
    #[test]
    fn invalid_geometry_rejected() {
        let s = Shape { x: 0.0, y: 0.0, w: 80.0, h: 20.0, radius: 100.0 };
        assert_eq!(s.validated().unwrap().radius, 10.0);
        assert!(Shape { w: -1.0, ..s }.validated().is_none());
        assert!(Shape { x: f32::NAN, ..s }.validated().is_none());
    }
}
