//! Lenient decoding of D-Bus values: tray items in the wild send slightly different
//! signatures (`v` wrappers, `ai` instead of `ay`, …), so values are matched by shape.
use crate::Icon;
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

pub type Props = HashMap<String, OwnedValue>;

pub fn unwrap<'a, 'b>(v: &'b Value<'a>) -> &'b Value<'a> {
    match v {
        Value::Value(inner) => unwrap(inner),
        v => v,
    }
}

pub fn as_str(v: &Value) -> Option<String> {
    match unwrap(v) {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        Value::Signature(s) => Some(s.to_string()),
        _ => None,
    }
}

pub fn as_bool(v: &Value) -> Option<bool> {
    match unwrap(v) {
        Value::Bool(b) => Some(*b),
        _ => as_i64(v).map(|n| n != 0),
    }
}

pub fn as_i64(v: &Value) -> Option<i64> {
    Some(match unwrap(v) {
        Value::I32(n) => *n as i64,
        Value::U32(n) => *n as i64,
        Value::I16(n) => *n as i64,
        Value::U16(n) => *n as i64,
        Value::I64(n) => *n,
        Value::U64(n) => *n as i64,
        Value::U8(n) => *n as i64,
        _ => return None,
    })
}

pub fn fields<'a, 'b>(v: &'b Value<'a>) -> Option<&'b [Value<'a>]> {
    match unwrap(v) {
        Value::Structure(s) => Some(s.fields()),
        _ => None,
    }
}

pub fn elements<'a, 'b>(v: &'b Value<'a>) -> Option<&'b [Value<'a>]> {
    match unwrap(v) {
        Value::Array(a) => Some(a.inner()),
        _ => None,
    }
}

pub fn bytes(v: &Value) -> Option<Vec<u8>> {
    elements(v)?.iter().map(|b| as_i64(b).map(|n| n as u8)).collect()
}

pub fn dict(v: &Value) -> Props {
    let mut out = HashMap::new();
    if let Value::Dict(d) = unwrap(v) {
        for (k, val) in d.iter() {
            if let (Some(k), Ok(val)) = (as_str(k), unwrap(val).try_to_owned()) {
                out.insert(k, val);
            }
        }
    }
    out
}

pub fn prop_str(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| as_str(v)).unwrap_or_default()
}

pub fn prop_bool(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| as_bool(v)).unwrap_or(false)
}

/// `a(iiay)`: ARGB32 in network byte order → premultiplied RGBA.
pub fn pixmaps(v: &Value) -> Vec<Icon> {
    let Some(list) = elements(v) else { return vec![] };
    let mut out = vec![];
    for e in list {
        let Some(f) = fields(e) else { continue };
        let (Some(w), Some(h), Some(data)) =
            (f.first().and_then(as_i64), f.get(1).and_then(as_i64), f.get(2).and_then(bytes))
        else {
            continue;
        };
        let (w, h) = (w.max(0) as u32, h.max(0) as u32);
        if w == 0 || h == 0 || data.len() < (w * h * 4) as usize {
            continue;
        }
        let mut rgba = Vec::with_capacity(data.len());
        for px in data.as_chunks::<4>().0.iter().take((w * h) as usize) {
            let (a, r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
            rgba.extend_from_slice(&[(r * a / 255) as u8, (g * a / 255) as u8, (b * a / 255) as u8, a as u8]);
        }
        out.push(Icon { width: w, height: h, rgba: Arc::new(rgba) });
    }
    out
}

/// `(sa(iiay)ss)` tooltip: title, else the description without markup.
pub fn tooltip(v: &Value) -> String {
    let Some(f) = fields(v) else { return as_str(v).unwrap_or_default() };
    let title = f.get(2).and_then(as_str).unwrap_or_default();
    let text = if title.trim().is_empty() { f.get(3).and_then(as_str).unwrap_or_default() } else { title };
    strip_markup(&text)
}

pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, StructureBuilder};

    fn pixmap(w: i32, h: i32, data: Vec<u8>) -> Value<'static> {
        Value::from(StructureBuilder::new().add_field(w).add_field(h).add_field(data).build().unwrap())
    }

    #[test]
    fn pixmaps_are_converted_to_premultiplied_rgba() {
        let list = Value::from(Array::from(vec![
            pixmap(1, 1, vec![255, 10, 20, 30]),
            pixmap(2, 1, vec![128, 255, 0, 0, 0, 1, 2, 3]),
            pixmap(2, 2, vec![255; 4]),
            pixmap(0, 1, vec![]),
        ]));
        let icons = pixmaps(&list);
        assert_eq!(icons.len(), 2, "short and empty pixmaps are skipped");
        assert_eq!((icons[0].width, icons[0].height), (1, 1));
        assert_eq!(*icons[0].rgba, vec![10, 20, 30, 255]);
        assert_eq!(*icons[1].rgba, vec![128, 0, 0, 128, 0, 0, 0, 0]);
        assert!(pixmaps(&Value::from(5u32)).is_empty());
    }

    #[test]
    fn tooltip_prefers_title_then_description() {
        let empty: Vec<Value> = vec![];
        let tt = |title: &str, desc: &str| {
            Value::from(
                StructureBuilder::new()
                    .add_field("icon")
                    .append_field(Value::from(Array::from(empty.clone())))
                    .add_field(title.to_string())
                    .add_field(desc.to_string())
                    .build()
                    .unwrap(),
            )
        };
        assert_eq!(tooltip(&tt("Title", "<b>desc</b>")), "Title");
        assert_eq!(tooltip(&tt(" ", "<b>desc</b> &amp; more")), "desc & more");
        assert_eq!(tooltip(&Value::from("plain")), "plain");
    }

    #[test]
    fn lenient_scalars() {
        assert_eq!(as_bool(&Value::from(true)), Some(true));
        assert_eq!(as_bool(&Value::from(0i32)), Some(false));
        assert_eq!(as_bool(&Value::from(2u32)), Some(true));
        assert_eq!(as_bool(&Value::from("x")), None);
        assert_eq!(as_i64(&Value::Value(Box::new(Value::from(7u8)))), Some(7));
        assert_eq!(as_str(&Value::Value(Box::new(Value::from("s")))).as_deref(), Some("s"));
        assert_eq!(bytes(&Value::from(vec![1i32, 2, 300])), Some(vec![1, 2, 44]));
    }

    #[test]
    fn props_helpers() {
        let mut p = Props::new();
        p.insert("Title".into(), OwnedValue::from(zbus::zvariant::Str::from("App")));
        p.insert("ItemIsMenu".into(), OwnedValue::from(true));
        assert_eq!(prop_str(&p, "Title"), "App");
        assert_eq!(prop_str(&p, "Missing"), "");
        assert!(prop_bool(&p, "ItemIsMenu"));
        assert!(!prop_bool(&p, "Missing"));
    }
}
