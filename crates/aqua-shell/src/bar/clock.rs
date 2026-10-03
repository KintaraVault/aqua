//! Local-time helpers (no external tz database: uses `date +%z`).
use aqua_icons::builtin::{civil_from_days, local_offset_secs};

pub struct Now {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub weekday: usize,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn now() -> Now {
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
            + local_offset_secs();
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    Now {
        year,
        month,
        day,
        weekday: ((days + 3).rem_euclid(7)) as usize,
        hour: (rem / 3600) as u32,
        minute: ((rem % 3600) / 60) as u32,
        second: (rem % 60) as u32,
    }
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const MONTHS_LONG: [&str; 12] = [
    "JANUARY",
    "FEBRUARY",
    "MARCH",
    "APRIL",
    "MAY",
    "JUNE",
    "JULY",
    "AUGUST",
    "SEPTEMBER",
    "OCTOBER",
    "NOVEMBER",
    "DECEMBER",
];

/// "Wed Jun 10  13:58"
pub fn menubar_string(n: &Now) -> String {
    menubar_string_fmt(n, true, false, true)
}

/// Menu-bar clock honouring the Menu Bar settings (24-hour, seconds, date).
pub fn menubar_string_fmt(n: &Now, h24: bool, seconds: bool, date: bool) -> String {
    let mut t = if h24 {
        format!("{:02}:{:02}", n.hour, n.minute)
    } else {
        let h = match n.hour % 12 {
            0 => 12,
            x => x,
        };
        format!("{}:{:02}", h, n.minute)
    };
    if seconds {
        t.push_str(&format!(":{:02}", n.second));
    }
    if !h24 {
        t.push_str(if n.hour < 12 { " AM" } else { " PM" });
    }
    if date {
        let (wd, mon) = (crate::tr(WEEKDAYS[n.weekday]), crate::tr(MONTHS[(n.month - 1) as usize]));
        if aqua_i18n::lang() == "ru" {
            format!("{wd} {} {mon}  {t}", n.day)
        } else {
            format!("{wd} {mon} {}  {t}", n.day)
        }
    } else {
        t
    }
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Weekday (0=Mon) of the 1st of the month.
pub fn first_weekday(n: &Now) -> usize {
    (n.weekday + 35 - ((n.day as usize - 1) % 7)) % 7
}

/// Current local (hour, minute).
pub fn now_hm() -> (u32, u32) {
    let n = now();
    (n.hour, n.minute)
}

/// Today's events from local iCalendar stores (GNOME Calendar / Evolution, Thunderbird exports,
/// khal/vdirsyncer): `(HH:MM or "", summary)`, sorted.
pub fn today_events() -> Vec<(String, String)> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::Instant, String, Vec<(String, String)>)>> = Mutex::new(None);
    let n = now();
    let today = format!("{:04}{:02}{:02}", n.year, n.month, n.day);
    if let Some((t, d, v)) = CACHE.lock().unwrap().as_ref() {
        if t.elapsed().as_secs() < 60 && *d == today {
            return v.clone();
        }
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let mut files = vec![];
    let mut stack = vec![
        (home.join(".local/share/evolution/calendar"), 0),
        (home.join(".local/share/aqua/calendar"), 0),
        (home.join(".calendars"), 0),
        (home.join(".local/share/khal/calendars"), 0),
    ];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if p.extension().map(|x| x == "ics").unwrap_or(false) {
                files.push(p);
            }
        }
    }
    let mut out = vec![];
    for f in files.iter().take(500) {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        out.extend(parse_ics_day(&text, &today));
    }
    out.sort();
    out.dedup();
    *CACHE.lock().unwrap() = Some((std::time::Instant::now(), today, out.clone()));
    out
}

/// Events of `day` (YYYYMMDD) in an iCalendar text.
pub fn parse_ics_day(text: &str, day: &str) -> Vec<(String, String)> {
    let mut lines: Vec<String> = vec![];
    for l in text.lines() {
        if (l.starts_with(' ') || l.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&l[1..]);
        } else {
            lines.push(l.trim_end_matches('\r').to_string());
        }
    }
    let mut out = vec![];
    let mut in_ev = false;
    let (mut start, mut summary) = (String::new(), String::new());
    for l in lines {
        if l == "BEGIN:VEVENT" {
            in_ev = true;
            start.clear();
            summary.clear();
        } else if l == "END:VEVENT" {
            in_ev = false;
            let mut s = start.clone();
            if s.ends_with('Z') && s.len() >= 15 {
                s = utc_to_local(&s);
            }
            if s.len() >= 8 && &s[..8] == day {
                let time = if s.len() >= 13 && s.as_bytes()[8] == b'T' {
                    format!("{}:{}", &s[9..11], &s[11..13])
                } else {
                    String::new()
                };
                out.push((
                    time,
                    if summary.is_empty() { "Event".into() } else { summary.replace("\\,", ",").replace("\\;", ";") },
                ));
            }
        } else if in_ev {
            if let Some((k, v)) = l.split_once(':') {
                let key = k.split(';').next().unwrap_or("");
                match key {
                    "DTSTART" => start = v.trim().to_string(),
                    "SUMMARY" => summary = v.trim().to_string(),
                    _ => {}
                }
            }
        }
    }
    out
}

fn utc_to_local(s: &str) -> String {
    let num = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    let (y, m, d, hh, mm) = (num(0, 4), num(4, 6) as u32, num(6, 8) as u32, num(9, 11), num(11, 13));
    let days = days_from_civil(y, m, d);
    let secs = days * 86400 + hh * 3600 + mm * 60 + local_offset_secs();
    let (y2, m2, d2) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!("{:04}{:02}{:02}T{:02}{:02}00", y2, m2, d2, rem / 3600, (rem % 3600) / 60)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod ics_tests {
    #[test]
    fn parses_events() {
        let t = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261001\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Europe/Moscow:20261001T093000\r\nSUMMARY:Stand\r\n up\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20261002T093000\r\nSUMMARY:Tomorrow\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let v = super::parse_ics_day(t, "20261001");
        assert_eq!(v.len(), 2);
        assert!(v.contains(&("".into(), "Holiday".into())));
        assert!(v.contains(&("09:30".into(), "Standup".into())));
    }
    #[test]
    fn civil_roundtrip() {
        let d = super::days_from_civil(2026, 10, 1);
        assert_eq!(super::civil_from_days(d), (2026, 10, 1));
    }
}
