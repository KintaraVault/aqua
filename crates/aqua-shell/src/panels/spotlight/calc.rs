//! Calculator for Spotlight queries: + - * / ^ %, parentheses, unary minus.

pub(super) fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.8}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Tiny arithmetic evaluator (+ - * / ^ %, parentheses). Requires at least one operator.
pub fn calc(s: &str) -> Option<f64> {
    let s: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '×' | 'x' => '*',
            '÷' => '/',
            ',' => '.',
            c => c,
        })
        .collect();
    if !s.chars().any(|c| "+-*/^%".contains(c)) || !s.chars().all(|c| c.is_ascii_digit() || "+-*/^%().".contains(c)) {
        return None;
    }
    let b = s.as_bytes();
    let mut i = 0;
    let v = expr(b, &mut i)?;
    (i == b.len() && v.is_finite()).then_some(v)
}

pub(super) fn expr(b: &[u8], i: &mut usize) -> Option<f64> {
    let mut v = term(b, i)?;
    while *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
        let op = b[*i];
        *i += 1;
        let r = term(b, i)?;
        v = if op == b'+' { v + r } else { v - r };
    }
    Some(v)
}

pub(super) fn term(b: &[u8], i: &mut usize) -> Option<f64> {
    let mut v = power(b, i)?;
    while *i < b.len() && (b[*i] == b'*' || b[*i] == b'/' || b[*i] == b'%') {
        let op = b[*i];
        *i += 1;
        let r = power(b, i)?;
        v = match op {
            b'*' => v * r,
            b'/' => v / r,
            _ => v % r,
        };
    }
    Some(v)
}

pub(super) fn power(b: &[u8], i: &mut usize) -> Option<f64> {
    let base = atom(b, i)?;
    if *i < b.len() && b[*i] == b'^' {
        *i += 1;
        let e = power(b, i)?;
        return Some(base.powf(e));
    }
    Some(base)
}

pub(super) fn atom(b: &[u8], i: &mut usize) -> Option<f64> {
    if *i >= b.len() {
        return None;
    }
    if b[*i] == b'-' {
        *i += 1;
        return power(b, i).map(|v| -v);
    }
    if b[*i] == b'(' {
        *i += 1;
        let v = expr(b, i)?;
        if *i < b.len() && b[*i] == b')' {
            *i += 1;
        }
        return Some(v);
    }
    let st = *i;
    while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.') {
        *i += 1;
    }
    std::str::from_utf8(&b[st..*i]).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(calc("2+2*2"), Some(6.0));
        assert_eq!(calc("(2+2)*2"), Some(8.0));
        assert_eq!(calc("10-4-3"), Some(3.0));
        assert_eq!(calc("2^3^2"), Some(512.0));
        assert_eq!(calc("-2^2"), Some(-4.0));
        assert_eq!(calc("2^-1"), Some(0.5));
        assert_eq!(calc("7%4"), Some(3.0));
        assert_eq!(calc("2--3"), Some(5.0));
    }

    #[test]
    fn input_normalisation() {
        assert_eq!(calc(" 3 × 4 "), Some(12.0));
        assert_eq!(calc("3x4"), Some(12.0));
        assert_eq!(calc("9÷3"), Some(3.0));
        assert_eq!(calc("1,5*2"), Some(3.0));
    }

    #[test]
    fn rejects_non_expressions() {
        assert_eq!(calc("42"), None, "a bare number is not a calculation");
        assert_eq!(calc("abc+1"), None);
        assert_eq!(calc("1/0"), None, "infinite results are dropped");
        assert_eq!(calc("2+"), None);
        assert_eq!(calc("1.2.3+1"), None);
        assert_eq!(calc("(1+2))"), None);
        assert_eq!(calc(""), None);
    }

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_num(6.0), "6");
        assert_eq!(fmt_num(-3.0), "-3");
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(1.0 / 3.0), "0.33333333");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(1e20), "100000000000000000000");
    }
}
