//! Notification banners + Notification Center (click the clock in the menu bar).
use crate::{clock, hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};
use aqua_icons::IconRequest;
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct Note {
    pub id: u32,
    /// App id / desktop entry used to pick the icon and activate the app.
    pub app_id: String,
    pub app_name: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub time: (u32, u32),
    /// Banner lifetime in seconds (0 = default).
    pub timeout: f32,
    /// Client action to invoke on click instead of activating the app.
    pub action: Option<String>,
    /// Buttons: (action key, label).
    pub actions: Vec<(String, String)>,
    /// Inline reply offered: placeholder text.
    pub reply: Option<String>,
    /// Keeps its actions after the banner hides (no "expired" signal).
    pub resident: bool,
    /// The client asked for no expiry (timeout 0): no "expired" signal either.
    pub persistent: bool,
    /// Critical urgency: the banner stays until dismissed.
    pub critical: bool,
    /// The banner timed out and the client was told; actions are gone.
    pub expired: bool,
}

impl Note {
    fn interactive(&self) -> bool {
        !self.expired && (!self.actions.is_empty() || self.reply.is_some())
    }
}

/// Inline reply being typed into a banner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub id: u32,
    pub text: String,
}

#[derive(Default)]
pub struct Notifications {
    pub list: Vec<Note>,
    pub banner: Option<(Note, Instant)>,
    pub center_open: bool,
    pub center_t: f32,
    pub banner_t: f32,
    /// Focus / Do Not Disturb: keep notifications in the centre, no banners.
    pub dnd: bool,
    /// Notifications clicked by the user since the last poll (id, action).
    pub clicked: Vec<(u32, Option<String>)>,
    /// Banners that timed out since the last poll (reported as "expired").
    pub expired: Vec<u32>,
    /// Inline replies sent since the last poll.
    pub replied: Vec<(u32, String)>,
    pub replying: Option<Reply>,
}

const W: f32 = 356.0;
const CARD_H: f32 = 74.0;
const ROW_H: f32 = 38.0;
const GAP: f32 = 10.0;
const DEFAULT_LIFE: f32 = 5.0;

fn card_h(n: &Note) -> f32 {
    if n.interactive() {
        CARD_H + ROW_H
    } else {
        CARD_H
    }
}

/// What a click on part of a card does.
#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Action(String),
    StartReply,
    Field,
    Send,
}

/// Button row of a card at `r` (any coordinate space).
fn buttons(r: Rect, n: &Note, replying: bool) -> Vec<(Rect, Hit)> {
    if !n.interactive() {
        return vec![];
    }
    let row = Rect::new(r.x + 14.0, r.y + CARD_H - 4.0, r.w - 28.0, 28.0);
    if replying {
        let send = 64.0;
        return vec![
            (Rect::new(row.x, row.y, row.w - send - 8.0, row.h), Hit::Field),
            (Rect::new(row.x + row.w - send, row.y, send, row.h), Hit::Send),
        ];
    }
    let mut hits: Vec<Hit> = n.actions.iter().map(|(k, _)| Hit::Action(k.clone())).collect();
    if n.reply.is_some() {
        hits.push(Hit::StartReply);
    }
    let k = hits.len() as f32;
    let bw = (row.w - 8.0 * (k - 1.0)) / k;
    hits.into_iter().enumerate().map(|(i, h)| (Rect::new(row.x + i as f32 * (bw + 8.0), row.y, bw, row.h), h)).collect()
}

impl Notifications {
    pub fn push(&mut self, n: Note) {
        self.list.retain(|o| o.id != n.id);
        self.list.insert(0, n.clone());
        self.list.truncate(50);
        if self.replying.as_ref().is_some_and(|r| r.id != n.id) {
            return;
        }
        if !self.center_open && !self.dnd {
            self.banner = Some((n, Instant::now()));
            self.banner_t = 0.0;
        }
    }
    pub fn close(&mut self, id: u32) {
        self.list.retain(|o| o.id != id);
        if self.banner.as_ref().map(|(b, _)| b.id == id).unwrap_or(false) {
            self.banner = None;
        }
        if self.replying.as_ref().is_some_and(|r| r.id == id) {
            self.replying = None;
        }
    }
    fn clicked_note(&mut self, n: &Note) -> Vec<Action> {
        let action = if n.expired { None } else { n.action.clone() };
        self.clicked.push((n.id, action.clone()));
        let mut v = vec![Action::Activate(n.app_id.clone()), Action::Redraw];
        if action.is_some() {
            v.remove(0);
        }
        v
    }
    /// The banner of `id` hid on its own: tell the client unless it wants to stay alive.
    fn expire(&mut self, id: u32) {
        let Some(n) = self.list.iter_mut().find(|n| n.id == id) else { return };
        if n.resident || n.persistent || n.expired {
            return;
        }
        n.expired = true;
        self.expired.push(id);
    }
    fn hit(&mut self, n: &Note, h: Hit) -> Vec<Action> {
        match h {
            Hit::Action(key) => {
                self.close(n.id);
                self.clicked.push((n.id, Some(key)));
                vec![Action::Redraw]
            }
            Hit::StartReply => {
                self.replying = Some(Reply { id: n.id, text: String::new() });
                self.center_open = false;
                self.banner = Some((n.clone(), Instant::now()));
                self.banner_t = 1.0;
                vec![Action::Redraw]
            }
            Hit::Field => vec![],
            Hit::Send => self.send_reply(),
        }
    }
    fn send_reply(&mut self) -> Vec<Action> {
        let Some(r) = self.replying.take_if(|r| !r.text.trim().is_empty()) else { return vec![] };
        self.close(r.id);
        self.replied.push((r.id, r.text));
        vec![Action::Redraw]
    }
    pub fn toggle_center(&mut self) {
        self.center_open = !self.center_open;
        if self.center_open {
            self.banner = None;
            self.replying = None;
        }
    }
    pub fn center_visible(&self) -> bool {
        self.center_open || self.center_t > 0.001
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let mut anim = false;
        let target = if self.center_open { 1.0 } else { 0.0 };
        if (self.center_t - target).abs() > 0.001 {
            self.center_t += (target - self.center_t) * (1.0 - (-10.0 * dt).exp());
            if (self.center_t - target).abs() < 0.01 {
                self.center_t = target;
            }
            anim = true;
        }
        let replying = self.replying.is_some();
        if let Some((n, t0)) = &self.banner {
            let age = t0.elapsed().as_secs_f32();
            let life = if replying || n.critical {
                f32::INFINITY
            } else if n.timeout > 0.0 {
                n.timeout
            } else {
                DEFAULT_LIFE
            };
            if age > life + 0.35 {
                let id = n.id;
                self.banner = None;
                self.expire(id);
            } else {
                self.banner_t = if age < life { (age / 0.35).min(1.0) } else { 1.0 - ((age - life) / 0.35).min(1.0) };
                anim = age < 0.35 || age >= life;
            }
        }
        anim
    }
}

/// Keyboard input for the inline reply field.
pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
    let Some(r) = sh.notes.replying.as_mut() else { return (false, vec![]) };
    match key {
        Some(Key::Escape) => {
            sh.notes.replying = None;
            if let Some((_, t0)) = sh.notes.banner.as_mut() {
                *t0 = Instant::now() - std::time::Duration::from_secs_f32(0.35);
            }
        }
        Some(Key::Backspace) => {
            r.text.pop();
        }
        Some(Key::Enter) => return (true, sh.notes.send_reply()),
        _ => {
            if let Some(t) = text {
                r.text.push_str(t);
            }
        }
    }
    (true, vec![Action::Redraw])
}

fn icon_for(n: &Note) -> IconRequest {
    IconRequest {
        id: n.app_id.clone(),
        name: n.app_name.clone(),
        icon: if n.icon.is_empty() { n.app_id.clone() } else { n.icon.clone() },
    }
}

fn ago(t: (u32, u32)) -> String {
    let now = clock::now();
    let mins = (now.hour * 60 + now.minute) as i64 - (t.0 * 60 + t.1) as i64;
    if mins <= 0 {
        "now".into()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else {
        format!("{:02}:{:02}", t.0, t.1)
    }
}

/// Draw a notification card at `r` (layer-local).
fn draw_card(c: &mut aqua_gfx::Canvas, sh: &mut Shell, r: Rect, n: &Note, dark: bool) {
    let f = sh.fonts.clone();
    let fg = style::text_primary(dark);
    let fg2 = style::text_secondary(dark);
    let ipx = (38.0 * sh.scale).round() as u32;
    let icon = sh.icons.get(&icon_for(n), ipx);
    c.draw_pixmap(&icon, Rect::new(r.x + 14.0, r.y + (r.h - 38.0) / 2.0, 38.0, 38.0), 1.0);
    let tx = r.x + 64.0;
    let tw = r.w - 64.0 - 14.0;
    let time = ago(n.time);
    let tm = f.measure(&time, 12.0, Weight::Regular);
    c.text_in(&f, Rect::new(r.x, r.y + 12.0, r.w - 14.0, 16.0), 1.0, 12.0, Weight::Regular, fg2, &time);
    let title = if n.summary.is_empty() { n.app_name.clone() } else { n.summary.clone() };
    c.text_in(&f, Rect::new(tx, r.y + 12.0, tw - tm - 8.0, 18.0), 0.0, 14.0, Weight::Semibold, fg, &title);
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    let body = if sh.cfg.notification_previews { n.body.as_str() } else { "" };
    for word in body.split_whitespace() {
        let t = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if f.measure(&t, 13.0, Weight::Regular) > tw && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
            if lines.len() == 2 {
                break;
            }
        } else {
            cur = t;
        }
    }
    if lines.len() < 2 && !cur.is_empty() {
        lines.push(cur);
    }
    for (i, l) in lines.iter().enumerate() {
        c.text_in(&f, Rect::new(tx, r.y + 32.0 + i as f32 * 17.0, tw, 17.0), 0.0, 13.0, Weight::Regular, fg, l);
    }
    let reply = sh.notes.replying.clone().filter(|rp| rp.id == n.id);
    let btn_bg = if dark { rgba(255, 255, 255, 0.16) } else { rgba(0, 0, 0, 0.08) };
    for (b, h) in buttons(Rect::new(r.x, r.y, r.w, card_h(n)), n, reply.is_some()) {
        c.fill_rrect(b, 8.0, btn_bg);
        let label = match &h {
            Hit::Action(k) => n.actions.iter().find(|(a, _)| a == k).map(|(_, l)| l.clone()).unwrap_or_default(),
            Hit::StartReply => crate::tr("Reply").to_string(),
            Hit::Send => crate::tr("Send").to_string(),
            Hit::Field => {
                let rp = reply.clone().unwrap_or_default();
                let inner = Rect::new(b.x + 10.0, b.y, b.w - 20.0, b.h);
                if rp.text.is_empty() {
                    let ph =
                        n.reply.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| crate::tr("Reply…").to_string());
                    c.text_in(&f, inner, 0.0, 13.0, Weight::Regular, fg2, &ph);
                } else {
                    let shown = tail_fit(&f, &rp.text, inner.w - 4.0);
                    let x1 = c.text_in(&f, inner, 0.0, 13.0, Weight::Regular, fg, &shown);
                    c.fill_rect(Rect::new(x1 + 1.0, b.y + 7.0, 1.5, b.h - 14.0), fg);
                }
                continue;
            }
        };
        let col = if h == Hit::Send { style::accent(1.0) } else { fg };
        c.text_in(&f, b, 0.5, 13.0, Weight::Medium, col, &label);
    }
}

/// The end of `s` that fits into `w` (the caret stays visible while typing).
fn tail_fit(f: &aqua_gfx::Fonts, s: &str, w: f32) -> String {
    let mut start = 0;
    while start < s.len() && f.measure(&s[start..], 13.0, Weight::Regular) > w {
        start += s[start..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    s[start..].to_string()
}

fn banner_rect(sh: &Shell) -> Rect {
    let h = sh.notes.banner.as_ref().map(|(n, _)| card_h(n)).unwrap_or(CARD_H);
    Rect::new(sh.w - W - 12.0, sh.cfg.menubar_height + 8.0, W, h)
}

pub fn banner_layer(sh: &mut Shell) -> Option<Layer> {
    let (n, _) = sh.notes.banner.clone()?;
    let t = sh.notes.banner_t;
    let ease = 1.0 - (1.0 - t).powi(3);
    let mut r = banner_rect(sh);
    let dark = sh.style.is_dark_glass(r);
    let key = hash_of(&(
        n.id,
        n.summary.clone(),
        n.body.clone(),
        dark,
        ago(n.time),
        sh.cfg.notification_previews,
        n.expired,
        sh.notes.replying.clone().map(|r| r.text),
    ));
    let (pm, serial) =
        sh.cached(LayerId::Banner, key, r.w, r.h, |c, sh| draw_card(c, sh, Rect::new(0.0, 0.0, W, r.h), &n, dark));
    r.x += (1.0 - ease) * (W * 0.35);
    let mut g = style::glass_panel(&sh.cfg.glass, 22.0);
    g.max_luma = 0.85;
    Some(Layer {
        id: LayerId::Banner,
        rect: r,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: ease,
        zoom: 1.0,
    })
}

/// Layout of the Notification Center column: (cards, calendar widget, clock widgets).
fn center_layout(sh: &Shell) -> (Rect, Vec<Rect>, Rect, Rect, Rect) {
    let x = sh.w - W - 12.0;
    let mut y = sh.cfg.menubar_height + 8.0;
    let mut cards = vec![];
    if sh.notes.list.is_empty() {
        cards.push(Rect::new(x, y, W, 46.0));
        y += 46.0 + GAP;
    } else {
        for n in sh.notes.list.iter().take(5) {
            let h = card_h(n);
            cards.push(Rect::new(x, y, W, h));
            y += h + GAP;
        }
    }
    y += 8.0;
    let cal = Rect::new(x, y, W, 168.0);
    y += 168.0 + GAP;
    let half = (W - GAP) / 2.0;
    let clk = Rect::new(x, y, half, half);
    let world = Rect::new(x + half + GAP, y, half, half);
    let bounds = Rect::new(x, sh.cfg.menubar_height + 8.0, W, y + half - sh.cfg.menubar_height - 8.0);
    (bounds, cards, cal, clk, world)
}

pub fn center_layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.notes.center_visible() {
        return None;
    }
    let (b, cards, cal, clk, world) = center_layout(sh);
    let dark = sh.style.is_dark_glass(b);
    let now = clock::now();
    let notes: Vec<Note> = sh.notes.list.iter().take(5).cloned().collect();
    let key = hash_of(&(
        notes.iter().map(|n| (n.id, n.summary.clone(), ago(n.time), n.expired)).collect::<Vec<_>>(),
        now.hour,
        now.minute,
        now.day,
        dark,
        b.h as i32,
        clock::today_events(),
    ));
    let (pm, serial) = sh.cached(LayerId::NotificationCenter, key, b.w, b.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        let loc = |r: &Rect| r.translate(-b.x, -b.y);
        if notes.is_empty() {
            c.text_in(&f, loc(&cards[0]), 0.5, 14.0, Weight::Medium, fg2, "No Notifications");
        }
        for (n, r) in notes.iter().zip(&cards) {
            draw_card(c, sh, loc(r), n, dark);
        }
        let r = loc(&cal);
        let red = rgba(255, 69, 58, 1.0);
        c.text(
            &f,
            r.x + 18.0,
            r.y + 32.0,
            13.0,
            Weight::Bold,
            red,
            &crate::tr(clock::WEEKDAYS_LONG[now.weekday]).to_uppercase(),
        );
        c.text(&f, r.x + 16.0, r.y + 96.0, 64.0, Weight::Light, fg, &now.day.to_string());
        let evs = clock::today_events();
        match evs.first() {
            None => {
                c.text(&f, r.x + 18.0, r.y + 140.0, 12.5, Weight::Medium, fg2, "No events today");
            }
            Some((t, sum)) => {
                let more = if evs.len() > 1 { format!(" +{}", evs.len() - 1) } else { String::new() };
                let line = if t.is_empty() { format!("{sum}{more}") } else { format!("{t} {sum}{more}") };
                c.text_in(
                    &f,
                    Rect::new(r.x + 18.0, r.y + 126.0, W * 0.46 - 22.0, 20.0),
                    0.0,
                    12.5,
                    Weight::Semibold,
                    fg,
                    &line,
                );
            }
        }
        let gx = r.x + W * 0.46;
        let cw = (W * 0.54 - 18.0) / 7.0;
        c.text(&f, gx + 4.0, r.y + 30.0, 11.5, Weight::Bold, red, clock::MONTHS_LONG[(now.month - 1) as usize]);
        for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
            c.text_in(&f, Rect::new(gx + i as f32 * cw, r.y + 38.0, cw, 14.0), 0.5, 9.5, Weight::Semibold, fg2, d);
        }
        let first = clock::first_weekday(&now);
        for d in 1..=clock::days_in_month(now.year, now.month) {
            let idx = first + d as usize - 1;
            let cell = Rect::new(gx + (idx % 7) as f32 * cw, r.y + 56.0 + (idx / 7) as f32 * 18.0, cw, 18.0);
            if d == now.day {
                c.fill_circle(cell.cx(), cell.cy(), 8.5, red);
                c.text_in(&f, cell, 0.5, 10.5, Weight::Bold, rgba(255, 255, 255, 1.0), &d.to_string());
            } else {
                c.text_in(&f, cell, 0.5, 10.5, Weight::Medium, fg, &d.to_string());
            }
        }
        let r = loc(&clk);
        c.text(&f, r.x + 16.0, r.y + 28.0, 12.0, Weight::Semibold, fg2, "Local");
        c.text_in(
            &f,
            Rect::new(r.x, r.y + 44.0, r.w, 60.0),
            0.5,
            44.0,
            Weight::Medium,
            fg,
            &format!("{:02}:{:02}", now.hour, now.minute),
        );
        c.text_in(
            &f,
            Rect::new(r.x, r.y + 112.0, r.w, 20.0),
            0.5,
            12.5,
            Weight::Regular,
            fg2,
            &format!("{} {}", crate::tr(clock::WEEKDAYS[now.weekday]), now.day),
        );
        let r = loc(&world);
        let off = aqua_icons::builtin::local_offset_secs();
        let utc_min = ((now.hour * 60 + now.minute) as i64 - off / 60).rem_euclid(1440);
        c.text(&f, r.x + 16.0, r.y + 28.0, 12.0, Weight::Semibold, fg2, "UTC");
        c.text_in(
            &f,
            Rect::new(r.x, r.y + 44.0, r.w, 60.0),
            0.5,
            44.0,
            Weight::Medium,
            fg,
            &format!("{:02}:{:02}", utc_min / 60, utc_min % 60),
        );
        let d = off / 3600;
        c.text_in(
            &f,
            Rect::new(r.x, r.y + 112.0, r.w, 20.0),
            0.5,
            12.5,
            Weight::Regular,
            fg2,
            &if d == 0 { "Same time".to_string() } else { format!("{:+}h", -d) },
        );
    });
    let t = sh.notes.center_t;
    let ease = 1.0 - (1.0 - t).powi(3);
    let dx = (1.0 - ease) * 40.0;
    let mut g = style::glass_tile(&sh.cfg.glass, 22.0);
    g.max_luma = 0.8;
    let mut tiles: Vec<(Rect, aqua_config::GlassStyle)> = cards
        .iter()
        .map(|r| (r.translate(dx, 0.0), aqua_config::GlassStyle { radius: if r.h < 50.0 { 23.0 } else { 22.0 }, ..g }))
        .collect();
    tiles.push((cal.translate(dx, 0.0), aqua_config::GlassStyle { radius: 24.0, ..g }));
    tiles.push((clk.translate(dx, 0.0), aqua_config::GlassStyle { radius: 24.0, ..g }));
    tiles.push((world.translate(dx, 0.0), aqua_config::GlassStyle { radius: 24.0, ..g }));
    Some(Layer {
        id: LayerId::NotificationCenter,
        rect: b.translate(dx, 0.0),
        glass: None,
        tiles,
        content: pm,
        serial,
        opacity: ease,
        zoom: 1.0,
    })
}

pub fn wants_pointer(sh: &Shell, x: f32, y: f32) -> bool {
    sh.notes.center_open || (sh.notes.banner.is_some() && banner_rect(sh).contains(x, y)) || sh.notes.replying.is_some()
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    if sh.notes.center_open {
        let (b, cards, ..) = center_layout(sh);
        if !b.contains(x, y) {
            sh.notes.toggle_center();
            return Some(vec![Action::Redraw]);
        }
        let notes: Vec<Note> = sh.notes.list.iter().take(5).cloned().collect();
        for (n, r) in notes.iter().zip(&cards) {
            if r.contains(x, y) {
                if let Some((_, h)) = buttons(*r, n, false).into_iter().find(|(b, _)| b.contains(x, y)) {
                    return Some(sh.notes.hit(n, h));
                }
                sh.notes.close(n.id);
                return Some(sh.notes.clicked_note(n));
            }
        }
        return Some(vec![Action::Redraw]);
    }
    if let Some((n, _)) = sh.notes.banner.clone() {
        let r = banner_rect(sh);
        if r.contains(x, y) {
            let replying = sh.notes.replying.as_ref().is_some_and(|rp| rp.id == n.id);
            if let Some((_, h)) = buttons(r, &n, replying).into_iter().find(|(b, _)| b.contains(x, y)) {
                return Some(sh.notes.hit(&n, h));
            }
            if replying {
                return Some(vec![]);
            }
            sh.notes.banner = None;
            return Some(sh.notes.clicked_note(&n));
        }
        if sh.notes.replying.is_some() {
            sh.notes.replying = None;
            return Some(vec![Action::Redraw]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, action: Option<&str>) -> Note {
        Note {
            id,
            app_id: "app".into(),
            summary: format!("n{id}"),
            action: action.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn push_replaces_same_id_and_shows_banner() {
        let mut n = Notifications::default();
        n.push(note(1, None));
        n.push(note(2, None));
        n.push(Note { summary: "updated".into(), ..note(1, None) });
        assert_eq!(n.list.iter().map(|x| x.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(n.list[0].summary, "updated");
        assert_eq!(n.banner.as_ref().map(|b| b.0.id), Some(1));
    }

    #[test]
    fn list_is_capped() {
        let mut n = Notifications::default();
        for i in 0..60 {
            n.push(note(i, None));
        }
        assert_eq!(n.list.len(), 50);
        assert_eq!(n.list[0].id, 59);
    }

    #[test]
    fn do_not_disturb_and_open_centre_suppress_banners() {
        let mut n = Notifications { dnd: true, ..Default::default() };
        n.push(note(1, None));
        assert!(n.banner.is_none());
        assert_eq!(n.list.len(), 1);
        n.dnd = false;
        n.toggle_center();
        n.push(note(2, None));
        assert!(n.banner.is_none());
        assert!(n.center_visible());
    }

    #[test]
    fn close_removes_banner() {
        let mut n = Notifications::default();
        n.push(note(7, None));
        n.close(7);
        assert!(n.list.is_empty());
        assert!(n.banner.is_none());
        n.close(42);
    }

    fn interactive(id: u32) -> Note {
        Note {
            actions: vec![("mark".into(), "Mark read".into())],
            reply: Some("Message".into()),
            ..note(id, Some("default"))
        }
    }

    #[test]
    fn expired_banner_is_reported_once_and_drops_actions() {
        let mut n = Notifications::default();
        n.push(note(1, Some("default")));
        n.banner.as_mut().unwrap().1 = Instant::now() - std::time::Duration::from_secs(10);
        n.animate(0.016);
        assert!(n.banner.is_none());
        assert_eq!(n.expired, vec![1]);
        assert!(n.list[0].expired);
        n.expire(1);
        assert_eq!(n.expired, vec![1]);
        let acts = n.clicked_note(&n.list[0].clone());
        assert!(matches!(acts.first(), Some(Action::Activate(_))));
        assert_eq!(n.clicked, vec![(1, None)]);
    }

    #[test]
    fn resident_persistent_and_critical_never_expire() {
        let mut n = Notifications::default();
        for (id, note) in [
            (1, Note { resident: true, ..note(1, None) }),
            (2, Note { persistent: true, ..note(2, None) }),
            (3, Note { critical: true, ..note(3, None) }),
        ] {
            n.push(note);
            n.banner.as_mut().unwrap().1 = Instant::now() - std::time::Duration::from_secs(60);
            n.animate(0.016);
            if id == 3 {
                assert!(n.banner.is_some(), "critical banner stays");
            }
        }
        assert!(n.expired.is_empty());
    }

    #[test]
    fn action_buttons_layout_and_hits() {
        let n = interactive(1);
        let r = Rect::new(0.0, 0.0, W, card_h(&n));
        assert_eq!(r.h, CARD_H + ROW_H);
        let b = buttons(r, &n, false);
        assert_eq!(
            b.iter().map(|x| x.1.clone()).collect::<Vec<_>>(),
            vec![Hit::Action("mark".into()), Hit::StartReply]
        );
        assert!(b[0].0.x + b[0].0.w < b[1].0.x);
        assert!(b.iter().all(|(br, _)| br.y + br.h <= r.h));
        let reply = buttons(r, &n, true);
        assert_eq!(reply.iter().map(|x| x.1.clone()).collect::<Vec<_>>(), vec![Hit::Field, Hit::Send]);
        let plain = note(2, None);
        assert_eq!(card_h(&plain), CARD_H);
        assert!(buttons(r, &plain, false).is_empty());
        assert!(buttons(r, &Note { expired: true, ..interactive(3) }, false).is_empty());
    }

    #[test]
    fn button_action_closes_and_reports_key() {
        let mut n = Notifications::default();
        n.push(interactive(4));
        let note = n.list[0].clone();
        n.hit(&note, Hit::Action("mark".into()));
        assert!(n.list.is_empty());
        assert_eq!(n.clicked, vec![(4, Some("mark".to_string()))]);
    }

    #[test]
    fn inline_reply_flow() {
        let mut n = Notifications::default();
        n.push(interactive(5));
        n.push(note(6, None));
        let note5 = n.list.iter().find(|x| x.id == 5).cloned().unwrap();
        n.hit(&note5, Hit::StartReply);
        assert_eq!(n.replying, Some(Reply { id: 5, text: String::new() }));
        assert_eq!(n.banner.as_ref().map(|b| b.0.id), Some(5));
        assert!(n.send_reply().is_empty(), "empty replies are not sent");
        n.banner.as_mut().unwrap().1 = Instant::now() - std::time::Duration::from_secs(60);
        n.animate(0.016);
        assert!(n.banner.is_some(), "banner stays while typing");
        n.push(note(7, None));
        assert_eq!(n.banner.as_ref().map(|b| b.0.id), Some(5), "new banners don't steal the reply");
        n.replying.as_mut().unwrap().text = "on my way".into();
        n.send_reply();
        assert_eq!(n.replied, vec![(5, "on my way".to_string())]);
        assert!(n.replying.is_none());
        assert!(!n.list.iter().any(|x| x.id == 5));
    }

    #[test]
    fn clicks_invoke_action_or_activate_app() {
        let mut n = Notifications::default();
        let acts = n.clicked_note(&note(1, None));
        assert!(matches!(acts.first(), Some(Action::Activate(a)) if a == "app"));
        let acts = n.clicked_note(&note(2, Some("default")));
        assert!(!acts.iter().any(|a| matches!(a, Action::Activate(_))));
        assert_eq!(n.clicked, vec![(1, None), (2, Some("default".to_string()))]);
    }
}
