use serde_json::{Map, Value};

struct Line {
    indent: usize,
    text: String,
}

fn strip_comment(s: &str) -> &str {
    let mut in_s = false;
    let mut in_d = false;
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\'' if !in_d => in_s = !in_s,
            b'"' if !in_s => in_d = !in_d,
            b'#' if !in_s && !in_d && (i == 0 || b[i - 1] == b' ') => return s[..i].trim_end(),
            _ => {}
        }
    }
    s
}

pub fn scalar(s: &str) -> Value {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        return Value::String(inner.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\"));
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return Value::String(t[1..t.len() - 1].replace("''", "'"));
    }
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        if inner.trim().is_empty() {
            return Value::Array(vec![]);
        }
        return Value::Array(inner.split(',').map(scalar).collect());
    }
    if t == "{}" {
        return Value::Object(Map::new());
    }
    match t {
        "" | "~" | "null" => Value::Null,
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(n) = t.parse::<i64>() {
                return Value::from(n);
            }
            Value::String(t.to_string())
        }
    }
}

fn split_key(t: &str) -> Option<(&str, &str)> {
    if t.starts_with('"') || t.starts_with('\'') {
        let q = &t[..1];
        let end = t[1..].find(q)? + 1;
        let rest = t[end + 1..].strip_prefix(':')?;
        return Some((&t[1..end], rest));
    }
    if let Some(k) = t.strip_suffix(':') {
        if !k.contains(": ") {
            return Some((k, ""));
        }
    }
    let i = t.find(": ")?;
    Some((&t[..i], &t[i + 1..]))
}

struct Parser {
    lines: Vec<Line>,
    raw: Vec<String>,
    i: usize,
}

impl Parser {
    fn skip_blank(&mut self) {
        while self.i < self.lines.len() && self.lines[self.i].text.is_empty() {
            self.i += 1;
        }
    }

    fn node(&mut self, min_indent: usize) -> Value {
        self.skip_blank();
        if self.i >= self.lines.len() || self.lines[self.i].indent < min_indent {
            return Value::Null;
        }
        let ind = self.lines[self.i].indent;
        let t = self.lines[self.i].text.clone();
        if t == "-" || t.starts_with("- ") {
            return self.seq(ind);
        }
        if split_key(&t).is_some() {
            return self.map(ind);
        }
        self.i += 1;
        scalar(&t)
    }

    fn seq(&mut self, ind: usize) -> Value {
        let mut out = vec![];
        loop {
            self.skip_blank();
            if self.i >= self.lines.len() || self.lines[self.i].indent != ind {
                break;
            }
            let t = self.lines[self.i].text.clone();
            if t != "-" && !t.starts_with("- ") {
                break;
            }
            let rest = t[1..].trim_start().to_string();
            if rest.is_empty() {
                self.i += 1;
                out.push(self.node(ind + 1));
            } else {
                self.lines[self.i] = Line { indent: ind + 2, text: rest };
                out.push(self.node(ind + 2));
            }
        }
        Value::Array(out)
    }

    fn block(&mut self, ind: usize, style: &str) -> Value {
        let fold = style.starts_with('>');
        let keep = style.contains('+');
        let strip = style.contains('-');
        let mut collected: Vec<String> = vec![];
        let mut block_indent: Option<usize> = None;
        while self.i < self.lines.len() {
            let raw = &self.raw[self.i];
            if raw.trim().is_empty() {
                collected.push(String::new());
                self.i += 1;
                continue;
            }
            let li = raw.len() - raw.trim_start().len();
            if li <= ind {
                break;
            }
            let bi = *block_indent.get_or_insert(li);
            if li < bi {
                break;
            }
            collected.push(raw[bi..].to_string());
            self.i += 1;
        }
        while !keep && collected.last().is_some_and(|l| l.is_empty()) {
            collected.pop();
        }
        let mut s = String::new();
        if fold {
            let mut prev_blank = true;
            for l in &collected {
                if l.is_empty() {
                    s.push('\n');
                    prev_blank = true;
                } else {
                    if !prev_blank && !l.starts_with(' ') {
                        s.push(' ');
                    }
                    s.push_str(l);
                    prev_blank = false;
                }
            }
        } else {
            s = collected.join("\n");
        }
        if !strip {
            s.push('\n');
        }
        Value::String(s)
    }

    fn map(&mut self, ind: usize) -> Value {
        let mut m = Map::new();
        loop {
            self.skip_blank();
            if self.i >= self.lines.len() || self.lines[self.i].indent != ind {
                break;
            }
            let t = self.lines[self.i].text.clone();
            if t.starts_with("- ") || t == "-" {
                break;
            }
            let Some((k, v)) = split_key(&t) else { break };
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            self.i += 1;
            let val = if v.starts_with('|') || v.starts_with('>') {
                self.block(ind, &v)
            } else if v.is_empty() {
                self.skip_blank();
                if self.i < self.lines.len()
                    && self.lines[self.i].indent == ind
                    && (self.lines[self.i].text.starts_with("- ") || self.lines[self.i].text == "-")
                {
                    self.seq(ind)
                } else {
                    self.node(ind + 1)
                }
            } else {
                scalar(&v)
            };
            m.insert(k, val);
        }
        Value::Object(m)
    }
}

pub fn parse_doc(text: &str) -> Value {
    let raw: Vec<String> = text.lines().map(str::to_string).collect();
    let lines = raw
        .iter()
        .map(|l| {
            let indent = l.len() - l.trim_start().len();
            let t = l.trim();
            let text = if t.starts_with('#') { String::new() } else { strip_comment(t).to_string() };
            Line { indent, text }
        })
        .collect();
    let mut p = Parser { lines, raw, i: 0 };
    p.node(0)
}

pub fn documents(text: &str) -> Vec<Value> {
    let mut docs = vec![];
    let mut cur = String::new();
    for l in text.lines() {
        if l.starts_with("---") {
            if !cur.trim().is_empty() {
                docs.push(parse_doc(&cur));
            }
            cur.clear();
            continue;
        }
        if l.starts_with("...") {
            continue;
        }
        cur.push_str(l);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        docs.push(parse_doc(&cur));
    }
    docs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dep11_like() {
        let y = "---\nFile: DEP-11\nOrigin: debian-bookworm-main\nMediaBaseUrl: https://appstream.debian.org/media/pool\n---\nType: desktop-application\nID: org.gimp.GIMP\nPackage: gimp\nName:\n  C: GNU Image Manipulation Program\n  de: GIMP\nSummary:\n  C: 'Create images: and edit'\nDescription:\n  C: >-\n    <p>GIMP is an advanced\n    picture editor.</p>\n\n    <ul><li>x</li></ul>\nCategories:\n- Graphics\n- 2DGraphics\nIcon:\n  cached:\n  - name: gimp_gimp.png\n    width: 64\n    height: 64\n  stock: gimp\nScreenshots:\n- default: true\n  caption:\n    C: Main\n  source-image:\n    url: org/gimp/a.png\n    width: 1920\nKeywords:\n  C: [paint, draw]\n";
        let d = documents(y);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["Origin"], "debian-bookworm-main");
        let c = &d[1];
        assert_eq!(c["Name"]["C"], "GNU Image Manipulation Program");
        assert_eq!(c["Summary"]["C"], "Create images: and edit");
        assert_eq!(c["Description"]["C"], "<p>GIMP is an advanced picture editor.</p>\n<ul><li>x</li></ul>");
        assert_eq!(c["Categories"][1], "2DGraphics");
        assert_eq!(c["Icon"]["cached"][0]["name"], "gimp_gimp.png");
        assert_eq!(c["Icon"]["cached"][0]["width"], 64);
        assert_eq!(c["Icon"]["stock"], "gimp");
        assert_eq!(c["Screenshots"][0]["source-image"]["url"], "org/gimp/a.png");
        assert_eq!(c["Screenshots"][0]["default"], true);
        assert_eq!(c["Keywords"]["C"][1], "draw");
    }

    #[test]
    fn literal_block_and_comments() {
        let v = parse_doc("a: |\n  line1\n  line2\nb: x # note\nc: \"q # not\"\n");
        assert_eq!(v["a"], "line1\nline2\n");
        assert_eq!(v["b"], "x");
        assert_eq!(v["c"], "q # not");
    }
}
