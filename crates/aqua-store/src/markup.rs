use crate::model::Block;

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // Within 12 characters (not bytes: `&` followed by Cyrillic must not split a char).
        let Some(end) = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

fn clean(s: &str) -> String {
    collapse_ws(&decode_entities(s))
}

#[derive(PartialEq)]
enum Ctx {
    Top,
    Para,
    Ul,
    Ol,
    Li,
}

pub fn parse(markup: &str) -> Vec<Block> {
    let src = markup.trim();
    if src.is_empty() {
        return vec![];
    }
    if !src.contains('<') {
        return plain(src);
    }
    let mut out = vec![];
    let mut stack: Vec<Ctx> = vec![Ctx::Top];
    let mut buf = String::new();
    let mut counter = 0usize;
    let flush = |buf: &mut String, out: &mut Vec<Block>, ctx: &Ctx, counter: usize| {
        let t = clean(buf);
        buf.clear();
        if t.is_empty() {
            return;
        }
        match ctx {
            Ctx::Li if counter > 0 => out.push(Block::Number(counter, t)),
            Ctx::Li => out.push(Block::Bullet(t)),
            _ => out.push(Block::Para(t)),
        }
    };
    let mut rest = src;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            buf.push_str(rest);
            break;
        };
        buf.push_str(&rest[..lt]);
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else {
            buf.push_str(rest);
            break;
        };
        let tag = rest[1..gt].trim();
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let self_closing = tag.ends_with('/');
        let top = stack.last().unwrap_or(&Ctx::Top);
        match (name.as_str(), closing) {
            ("p", false) => {
                flush(&mut buf, &mut out, top, counter);
                stack.push(Ctx::Para);
            }
            ("p", true) => {
                flush(&mut buf, &mut out, top, counter);
                if stack.last() == Some(&Ctx::Para) {
                    stack.pop();
                }
            }
            ("ul" | "ol", false) => {
                flush(&mut buf, &mut out, top, counter);
                counter = 0;
                stack.push(if name == "ol" { Ctx::Ol } else { Ctx::Ul });
            }
            ("ul" | "ol", true) => {
                flush(&mut buf, &mut out, top, counter);
                while let Some(c) = stack.pop() {
                    if c == Ctx::Ul || c == Ctx::Ol || stack.len() == 1 {
                        break;
                    }
                }
                counter = 0;
            }
            ("li", false) => {
                flush(&mut buf, &mut out, top, counter);
                if stack.contains(&Ctx::Ol)
                    && stack.iter().rev().find(|c| **c == Ctx::Ol || **c == Ctx::Ul) == Some(&Ctx::Ol)
                {
                    counter += 1;
                } else {
                    counter = 0;
                }
                stack.push(Ctx::Li);
            }
            ("li", true) => {
                flush(&mut buf, &mut out, top, counter);
                if stack.last() == Some(&Ctx::Li) {
                    stack.pop();
                }
            }
            ("br", _) if self_closing || !closing => buf.push(' '),
            _ => {}
        }
        if stack.is_empty() {
            stack.push(Ctx::Top);
        }
    }
    let top = stack.last().unwrap_or(&Ctx::Top);
    flush(&mut buf, &mut out, top, counter);
    out
}

pub fn plain(text: &str) -> Vec<Block> {
    let mut out = vec![];
    for para in text.split("\n\n") {
        let mut cur = String::new();
        for line in para.lines() {
            let l = line.trim();
            let bullet = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("• "));
            if let Some(b) = bullet {
                if !cur.trim().is_empty() {
                    out.push(Block::Para(collapse_ws(&cur)));
                }
                cur.clear();
                out.push(Block::Bullet(collapse_ws(b)));
            } else if l == "." {
                if !cur.trim().is_empty() {
                    out.push(Block::Para(collapse_ws(&cur)));
                }
                cur.clear();
            } else {
                cur.push(' ');
                cur.push_str(l);
            }
        }
        if !cur.trim().is_empty() {
            out.push(Block::Para(collapse_ws(&cur)));
        }
    }
    out
}

pub fn first_line(blocks: &[Block]) -> String {
    blocks.first().map(|b| b.text().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_next_to_non_ascii_text() {
        assert_eq!(decode_entities("R&D отдел разработки"), "R&D отдел разработки");
        assert_eq!(decode_entities("&ёжик; &amp;ё"), "&ёжик; &ё");
        assert_eq!(decode_entities("中&文字文字文字文字;"), "中&文字文字文字文字;");
    }

    #[test]
    fn paragraphs_and_lists() {
        let b = parse(
            "<p>\n  GIMP is &amp; free.\n</p><ul><li>One</li><li>Two &lt;b&gt;</li></ul><p>End</p><ol><li>a</li><li>b</li></ol>",
        );
        assert_eq!(
            b,
            vec![
                Block::Para("GIMP is & free.".into()),
                Block::Bullet("One".into()),
                Block::Bullet("Two <b>".into()),
                Block::Para("End".into()),
                Block::Number(1, "a".into()),
                Block::Number(2, "b".into()),
            ]
        );
    }

    #[test]
    fn inline_markup_is_flattened() {
        let b = parse("<p>Use <em>this</em> and <code>that</code></p>");
        assert_eq!(b, vec![Block::Para("Use this and that".into())]);
    }

    #[test]
    fn plain_text_debian_style() {
        let b = plain("A tool\n that does things.\n .\n - first\n - second");
        assert_eq!(
            b,
            vec![
                Block::Para("A tool that does things.".into()),
                Block::Bullet("first".into()),
                Block::Bullet("second".into())
            ]
        );
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &#65;&#x42; &unknown; &"), "a AB &unknown; &");
    }

    #[test]
    fn text_rendering() {
        let t = crate::model::blocks_to_text(&parse("<p>Hi</p><ul><li>a</li><li>b</li></ul><p>Bye</p>"));
        assert_eq!(t, "Hi\n\n• a\n• b\n\nBye");
    }
}
