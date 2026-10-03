//! `com.canonical.dbusmenu` client side: layout decoding.
use crate::value::{as_bool, as_i64, as_str, dict, elements, fields};
use zbus::zvariant::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuToggle {
    #[default]
    None,
    Check(bool),
    Radio(bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub toggle: MenuToggle,
    pub icon_name: String,
    pub children: Vec<MenuNode>,
}

/// Remove the `_` access-key markers (`__` is a literal underscore).
fn strip_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '_' {
            if it.peek() == Some(&'_') {
                out.push('_');
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Decode one `(ia{sv}av)` node; invisible rows are dropped.
pub fn parse(v: &Value) -> Option<MenuNode> {
    let f = fields(v)?;
    let id = f.first().and_then(as_i64)? as i32;
    let props = f.get(1).map(dict).unwrap_or_default();
    if props.get("visible").and_then(|v| as_bool(v)) == Some(false) {
        return None;
    }
    let kind = props.get("type").and_then(|v| as_str(v)).unwrap_or_default();
    let state = props.get("toggle-state").and_then(|v| as_i64(v)).unwrap_or(0) == 1;
    let toggle = match props.get("toggle-type").and_then(|v| as_str(v)).as_deref() {
        Some("checkmark") => MenuToggle::Check(state),
        Some("radio") => MenuToggle::Radio(state),
        _ => MenuToggle::None,
    };
    let children = f.get(2).and_then(elements).map(|c| c.iter().filter_map(parse).collect()).unwrap_or_default();
    Some(MenuNode {
        id,
        label: strip_mnemonic(&props.get("label").and_then(|v| as_str(v)).unwrap_or_default()),
        enabled: props.get("enabled").and_then(|v| as_bool(v)).unwrap_or(true),
        separator: kind == "separator",
        toggle,
        icon_name: props.get("icon-name").and_then(|v| as_str(v)).unwrap_or_default(),
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use zbus::zvariant::{Array, Dict, Signature, StructureBuilder};

    fn node(id: i32, props: &[(&str, Value<'static>)], children: Vec<Value<'static>>) -> Value<'static> {
        let mut d = Dict::new(&Signature::Str, &Signature::Variant);
        for (k, v) in props {
            d.append(Value::from(k.to_string()), Value::Value(Box::new(v.clone()))).unwrap();
        }
        let kids: Vec<Value> = children.into_iter().map(|c| Value::Value(Box::new(c))).collect();
        let arr = if kids.is_empty() { Array::new(&Signature::Variant) } else { Array::from(kids) };
        Value::from(
            StructureBuilder::new()
                .add_field(id)
                .append_field(Value::from(d))
                .append_field(Value::from(arr))
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn parses_layout_tree() {
        let tree = node(
            0,
            &[],
            vec![
                node(1, &[("label", Value::from("_Open"))], vec![]),
                node(2, &[("type", Value::from("separator"))], vec![]),
                node(3, &[("label", Value::from("Hidden")), ("visible", Value::from(false))], vec![]),
                node(
                    4,
                    &[
                        ("label", Value::from("Mute")),
                        ("toggle-type", Value::from("checkmark")),
                        ("toggle-state", Value::from(1i32)),
                        ("enabled", Value::from(false)),
                    ],
                    vec![],
                ),
                node(
                    5,
                    &[("label", Value::from("Mode")), ("toggle-type", Value::from("radio"))],
                    vec![node(6, &[("label", Value::from("A"))], vec![])],
                ),
            ],
        );
        let root = parse(&tree).expect("root");
        let ids: Vec<i32> = root.children.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2, 4, 5], "invisible items are dropped");
        assert_eq!(root.children[0].label, "Open");
        assert!(root.children[0].enabled);
        assert!(root.children[1].separator);
        assert_eq!(root.children[2].toggle, MenuToggle::Check(true));
        assert!(!root.children[2].enabled);
        assert_eq!(root.children[3].toggle, MenuToggle::Radio(false));
        assert_eq!(root.children[3].children[0].label, "A");
    }

    #[test]
    fn rejects_malformed_nodes() {
        assert!(parse(&Value::from(1u32)).is_none());
        assert!(parse(&Value::from(StructureBuilder::new().add_field("x").build().unwrap())).is_none());
    }

    #[test]
    fn mnemonics() {
        assert_eq!(strip_mnemonic("_Quit"), "Quit");
        assert_eq!(strip_mnemonic("Save__As"), "Save_As");
    }
}
