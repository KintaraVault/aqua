use std::cmp::Ordering;

fn split_evr(v: &str) -> (u64, &str, Option<&str>) {
    let (epoch, rest) = match v.split_once(':') {
        Some((e, r)) if !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()) => (e.parse().unwrap_or(0), r),
        _ => (0, v),
    };
    match rest.rsplit_once('-') {
        Some((ver, rel)) => (epoch, ver, Some(rel)),
        None => (epoch, rest, None),
    }
}

pub fn segment_cmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        while i < a.len() && !a[i].is_ascii_alphanumeric() && a[i] != b'~' {
            i += 1;
        }
        while j < b.len() && !b[j].is_ascii_alphanumeric() && b[j] != b'~' {
            j += 1;
        }
        let at = a.get(i) == Some(&b'~');
        let bt = b.get(j) == Some(&b'~');
        if at || bt {
            if !at {
                return Ordering::Greater;
            }
            if !bt {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if i >= a.len() || j >= b.len() {
            break;
        }
        let num = a[i].is_ascii_digit();
        let take = |s: &[u8], mut k: usize| {
            let st = k;
            while k < s.len() && (if num { s[k].is_ascii_digit() } else { s[k].is_ascii_alphabetic() }) {
                k += 1;
            }
            (st, k)
        };
        let (sa, ea) = take(a, i);
        let (sb, eb) = take(b, j);
        if sb == eb {
            return if num { Ordering::Greater } else { Ordering::Less };
        }
        let (x, y) = (&a[sa..ea], &b[sb..eb]);
        let ord = if num {
            let x = trim_zeros(x);
            let y = trim_zeros(y);
            x.len().cmp(&y.len()).then_with(|| x.cmp(y))
        } else {
            x.cmp(y)
        };
        if ord != Ordering::Equal {
            return ord;
        }
        i = ea;
        j = eb;
    }
    let ra = i < a.len();
    let rb = j < b.len();
    match (ra, rb) {
        (false, false) => Ordering::Equal,
        (true, false) => {
            if a[i].is_ascii_alphabetic() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if b[j].is_ascii_alphabetic() {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        _ => Ordering::Equal,
    }
}

fn trim_zeros(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|c| **c == b'0').count();
    &s[n..]
}

pub fn vercmp(a: &str, b: &str) -> Ordering {
    let (ea, va, ra) = split_evr(a.trim());
    let (eb, vb, rb) = split_evr(b.trim());
    ea.cmp(&eb).then_with(|| segment_cmp(va, vb)).then_with(|| match (ra, rb) {
        (Some(x), Some(y)) => segment_cmp(x, y),
        _ => Ordering::Equal,
    })
}

pub fn newer(candidate: &str, installed: &str) -> bool {
    vercmp(candidate, installed) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    #[test]
    fn numeric_and_alpha() {
        assert_eq!(vercmp("1.0", "1.0"), Equal);
        assert_eq!(vercmp("1.0.1", "1.0"), Greater);
        assert_eq!(vercmp("1.10", "1.9"), Greater);
        assert_eq!(vercmp("1.0a", "1.0"), Less);
        assert_eq!(vercmp("1.0", "1.0a"), Greater);
        assert_eq!(vercmp("1.0b", "1.0a"), Greater);
        assert_eq!(vercmp("001", "1"), Equal);
    }

    #[test]
    fn epoch_and_release() {
        assert_eq!(vercmp("1:1.0-1", "2.0-1"), Greater);
        assert_eq!(vercmp("1.0-2", "1.0-1"), Greater);
        assert_eq!(vercmp("1.0-1", "1.0"), Equal);
        assert_eq!(vercmp("3.2.6-1", "3.2.10-1"), Less);
    }

    #[test]
    fn tilde_sorts_first() {
        assert_eq!(vercmp("1.0~rc1", "1.0"), Less);
        assert_eq!(vercmp("1.0~rc2", "1.0~rc1"), Greater);
        assert!(newer("2.1.0-1", "2.0.9-3"));
    }
}
