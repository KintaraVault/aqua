//! Character Viewer ("Show Emoji & Symbols", ⌃⌘Space) and Keyboard Viewer.
//! Both are floating glass palettes that do not take keyboard focus: clicking a
//! cell types into the focused application.
use crate::{hash_of, style, Action, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    #[default]
    Chars,
    Keyboard,
}

#[derive(Default)]
pub struct CharViewer {
    pub open: bool,
    pub mode: Mode,
    pub t: f32,
    pub tab: usize,
    pub hover: Option<usize>,
    /// Shift state of the keyboard viewer.
    pub shift: bool,
    /// First visible row of the character grid, and the wheel remainder.
    pub scroll: usize,
    pub scroll_acc: f32,
}

/// (category name, tab glyph, characters).
pub const TABS: [(&str, &str, &str); 14] = [
    ("Smileys & Emotion", "😀", "😀😃😄😁😆😅🤣😂🙂🙃😉😊😇🥰😍🤩😘😗😚😋😛😜🤪😝🤑🤗🤭🤫🤔🤐🤨😐😑😶😏😒🙄😬🤥😌😔😪🤤😴😷🤒🤕🤢🤮🤧🥵🥶🥴😵🤯🤠🥳😎🤓🧐😕😟🙁😮😯😲😳🥺😦😧😨😰😥😢😭😱😖😣😞😓😩😫🥱😤😡😠🤬😈👿💀💩🤡👻👽👾🤖"),
    ("People & Body", "👋", "👋🤚🖐✋🖖👌🤌🤏✌🤞🤟🤘🤙👈👉👆👇☝👍👎✊👊🤛🤜👏🙌👐🤲🤝🙏✍💅🤳💪🦾🦿🦵🦶👂🦻👃🧠🦷🦴👀👁👅👄👶🧒👦👧🧑👱👨🧔👩🧓👴👵🙍🙎🙅🙆💁🙋🧏🙇🤦🤷👮🕵💂🥷👷🤴👸👳👲🧕🤵👰🤰🤱👼🎅🤶🦸🦹🧙🧚🧛🧜🧝🧞🧟💆💇🚶🧍🧎🏃💃🕺"),
    ("Animals & Nature", "🐻", "🐶🐱🐭🐹🐰🦊🐻🐼🐨🐯🦁🐮🐷🐸🐵🙈🙉🙊🐔🐧🐦🐤🦆🦅🦉🦇🐺🐗🐴🦄🐝🐛🦋🐌🐞🐜🕷🦂🐢🐍🦎🦖🦕🐙🦑🦐🦞🦀🐡🐠🐟🐬🐳🐋🦈🐊🐅🐆🦓🦍🐘🦛🦏🐪🦒🦘🐄🐎🐖🐑🐐🦌🐕🐈🐓🦃🦚🦜🦢🌵🎄🌲🌳🌴🌱🌿☘🍀🍃🍂🍁🍄🌾💐🌷🌹🥀🌺🌸🌼🌻🌞🌝🌛🌚🌕🌙🌎🪐💫⭐🌟✨⚡🔥🌈☀⛅☁🌧⛈❄☃⛄💧☔🌊"),
    ("Food & Drink", "🍔", "🍏🍎🍐🍊🍋🍌🍉🍇🍓🫐🍈🍒🍑🥭🍍🥥🥝🍅🍆🥑🥦🥬🥒🌶🌽🥕🧄🧅🥔🍠🥐🥯🍞🥖🥨🧀🥚🍳🧈🥞🧇🥓🥩🍗🍖🌭🍔🍟🍕🥪🥙🧆🌮🌯🥗🥘🥫🍝🍜🍲🍛🍣🍱🥟🍤🍙🍚🍘🍥🥠🍢🍡🍧🍨🍦🥧🧁🍰🎂🍮🍭🍬🍫🍿🍩🍪🌰🥜🍯🥛🍼☕🍵🧃🥤🍶🍺🍻🥂🍷🥃🍸🍹🧉🍾"),
    ("Activity", "⚽", "⚽🏀🏈⚾🥎🎾🏐🏉🥏🎱🏓🏸🏒🏑🥍🏏🥅⛳🏹🎣🤿🥊🥋🎽🛹⛸🥌🎿⛷🏂🏆🥇🥈🥉🏅🎖🎗🎫🎟🎪🎭🎨🎬🎤🎧🎼🎹🥁🎷🎺🎸🪕🎻🎲♟🎯🎳🎮🎰🧩🧸🪀🪁🔮🎴🃏🀄🎊🎉🎈🎁🎀🎏🎐🧧🎃🎆🎇🧨"),
    ("Travel & Places", "🚗", "🚗🚕🚙🚌🚎🏎🚓🚑🚒🚐🚚🚛🚜🛴🚲🛵🏍🚨🚔🚍🚘🚖🚡🚠🚃🚋🚝🚄🚅🚂🚆🚇🚉✈🛫🛬🛩💺🛰🚀🛸🚁🛶⛵🚤🛥🛳⛴🚢⚓⛽🚧🚦🗺🗿🗽🗼🏰🏯🏟🎡🎢🎠⛲⛱🏖🏝🏜🌋⛰🏔🗻🏕⛺🏠🏡🏘🏗🏭🏢🏬🏥🏦🏨🏪🏫🏩💒🏛⛪🕌🕍🕋⛩🌁🌃🏙🌄🌅🌆🌇🌉🎑"),
    ("Objects", "💡", "⌚📱💻⌨🖥🖨🖱🕹💽💾💿📀📷📸📹🎥📽📞☎📺📻🎙🧭⏱⏰⌛⏳📡🔋🔌💡🔦🕯🧯💸💵💶💷💰💳💎⚖🧰🔧🔨🛠⛏🔩⚙🧱🧲💣🔪🗡⚔🛡⚰🏺📿🧿💈🔭🔬🩹🩺💊💉🧬🦠🧪🌡🧹🧺🧻🚽🚿🛁🧼🧽🛎🔑🗝🚪🪑🛋🛏🖼🛍🛒✉📩📧💌📦🏷📮📜📄🧾📊📈📉🗒🗓📅🗑📋📁📂🗂📰📓📕📗📘📙📚📖🔖🔗📎📐📏📌📍✂🖊✒🖌🖍📝✏🔍🔒🔓"),
    ("Symbols", "❤", "❤🧡💛💚💙💜🖤🤍🤎💔❣💕💞💓💗💖💘💝💟☮✝☪🕉☸✡🔯☯☦🛐♈♉♊♋♌♍♎♏♐♑♒♓⚛☢☣📴📳✴🆚💮🅰🅱🆎🆑🅾🆘❌⭕🛑⛔📛🚫💯💢♨🚷🚯🔞📵🚭❗❓‼⁉🔅🔆〽⚠🚸🔱⚜🔰♻✅💹❇✳❎🌐💠Ⓜ🌀💤🏧🚾♿🅿🛂🛃🚹🚺🚼🚻🚮🎦📶🔣ℹ🔤🔡🔠🆖🆗🆙🆒🆕🆓🔟🔢▶⏸⏯⏹⏺⏭⏮⏩⏪🔀🔁🔂◀🔼🔽"),
    ("Stars & Bullets", "★", "★☆✦✧✩✪✫✬✭✮✯✰⁂※✱✲✳✴✵✶✷✸✹✺✻✼✽✾✿❀❁❂❃❄❅❆❇❈❉❊❋•◦‣⁃∙●○◎◉◌◍◐◑◒◓■□▪▫▲△▼▽◆◇◊♠♣♥♦♤♧♡♢♩♪♫♬✓✔✗✘☐☑☒©®™℗℠№℮§¶†‡°′″‰⌘⌥⇧⌃⎋⏏⌫⌦↩⇥⇪⌤⌅⏎"),
    ("Arrows", "→", "←↑→↓↔↕↖↗↘↙↚↛↜↝↞↟↠↡↢↣↤↥↦↧↨↩↪↫↬↭↮↯↰↱↲↳↴↵↶↷↸↹↺↻⇄⇅⇆⇇⇈⇉⇊⇋⇌⇍⇎⇏⇐⇑⇒⇓⇔⇕⇖⇗⇘⇙⇚⇛⇜⇝⇞⇟⇠⇡⇢⇣⇤⇥⇦⇧⇨⇩➔➘➙➚➛➜➝➞➟➠➡➢➣➤➥➦➧➨⟵⟶⟷⟸⟹⟺"),
    ("Math", "∑", "±×÷=≠≈≡≢≤≥≪≫∞√∛∜∑∏∐∫∬∮∂∆∇∈∉∋∌∩∪⊂⊃⊆⊇⊄⊅∧∨¬⊕⊗⊥∥∠∡∢∴∵∀∃∄∅∝∼≅≃⌈⌉⌊⌋⟨⟩ℕℤℚℝℂℙℵ℘ℓ¹²³⁴⁵⁶⁷⁸⁹⁰₀₁₂₃₄½⅓⅔¼¾⅕⅛∕∗∘∙"),
    ("Currency", "$", "$€£¥₽₴₹₩₺₿¢₪₫₦₱₲₵₸₼₾₡₭₮₳₣₤₥₧₨₯₠₢¤"),
    ("Greek & Latin", "α", "αβγδεζηθικλμνξοπρστυφχψωΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩϑϕϖϵàáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿßœšžłđħ"),
    ("Punctuation", "«", "«»„“”‘’‚‹›–—‒―…·¡¿‽⸮¦‖′″‴‵‶‷‸⁅⁆⁎⁑⁓⁕⁖⁘⁙⁚⁛⁜⁝⁞〈〉《》「」『』【】〔〕〖〗〘〙〚〛"),
];

/// Number of emoji categories at the start of [`TABS`].
const EMOJI_TABS: usize = 8;
const COLS: usize = 12;
const CELL: f32 = 34.0;
const TAB_W: f32 = 28.0;
/// Top of the character grid inside the panel.
const GRID_Y: f32 = 62.0;
/// Rows visible at once; longer categories scroll.
const MAX_ROWS: usize = 8;

impl CharViewer {
    pub fn toggle(&mut self, mode: Mode) {
        if self.open && self.mode == mode {
            self.open = false;
        } else {
            self.open = true;
            self.mode = mode;
        }
    }
    pub fn close(&mut self) {
        self.open = false;
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.t - target).abs() < 0.001 {
            self.t = target;
            return false;
        }
        self.t += (target - self.t) * (1.0 - (-12.0 * dt).exp());
        if (self.t - target).abs() < 0.01 {
            self.t = target;
        }
        true
    }
    pub fn wants_pointer(&self, _x: f32, _y: f32) -> bool {
        false
    }
}

/// Characters of a category.
fn chars_of(tab: usize) -> Vec<String> {
    let tab = tab.min(TABS.len() - 1);
    TABS[tab]
        .2
        .chars()
        .filter(|c| *c != '\u{FE0F}')
        .map(|c| if tab < EMOJI_TABS && (c as u32) < 0x1F000 { format!("{c}\u{FE0F}") } else { c.to_string() })
        .collect()
}

fn rows_of(tab: usize) -> usize {
    chars_of(tab).len().div_ceil(COLS)
}

fn char_panel(sh: &Shell) -> Rect {
    let rows = rows_of(sh.chars.tab).min(MAX_ROWS);
    let w = COLS as f32 * CELL + 24.0;
    let h = GRID_Y + rows as f32 * CELL + 12.0;
    Rect::new(sh.w - w - 24.0, (sh.h - h - 110.0).max(40.0), w, h)
}

/// Scroll offset (rows) of the current category.
fn first_row(sh: &Shell) -> usize {
    let max = rows_of(sh.chars.tab).saturating_sub(MAX_ROWS);
    sh.chars.scroll.min(max)
}

/// Scroll the character grid (wheel over the panel); true when handled.
pub fn scroll(sh: &mut Shell, x: f32, y: f32, dy: f32) -> bool {
    if !sh.chars.open || sh.chars.mode != Mode::Chars || !char_panel(sh).contains(x, y) {
        return false;
    }
    sh.chars.scroll_acc += dy;
    let max = rows_of(sh.chars.tab).saturating_sub(MAX_ROWS);
    while sh.chars.scroll_acc >= CELL * 0.5 {
        sh.chars.scroll_acc -= CELL * 0.5;
        sh.chars.scroll = (sh.chars.scroll + 1).min(max);
    }
    while sh.chars.scroll_acc <= -CELL * 0.5 {
        sh.chars.scroll_acc += CELL * 0.5;
        sh.chars.scroll = sh.chars.scroll.saturating_sub(1);
    }
    true
}

/// (evdev code, width units) per row; labels come from the active keymap.
const ROWS: [&[(u32, f32)]; 5] = [
    &[
        (41, 1.0),
        (2, 1.0),
        (3, 1.0),
        (4, 1.0),
        (5, 1.0),
        (6, 1.0),
        (7, 1.0),
        (8, 1.0),
        (9, 1.0),
        (10, 1.0),
        (11, 1.0),
        (12, 1.0),
        (13, 1.0),
        (14, 1.5),
    ],
    &[
        (15, 1.5),
        (16, 1.0),
        (17, 1.0),
        (18, 1.0),
        (19, 1.0),
        (20, 1.0),
        (21, 1.0),
        (22, 1.0),
        (23, 1.0),
        (24, 1.0),
        (25, 1.0),
        (26, 1.0),
        (27, 1.0),
        (43, 1.0),
    ],
    &[
        (58, 1.75),
        (30, 1.0),
        (31, 1.0),
        (32, 1.0),
        (33, 1.0),
        (34, 1.0),
        (35, 1.0),
        (36, 1.0),
        (37, 1.0),
        (38, 1.0),
        (39, 1.0),
        (40, 1.0),
        (28, 1.75),
    ],
    &[
        (42, 2.25),
        (44, 1.0),
        (45, 1.0),
        (46, 1.0),
        (47, 1.0),
        (48, 1.0),
        (49, 1.0),
        (50, 1.0),
        (51, 1.0),
        (52, 1.0),
        (53, 1.0),
        (54, 2.25),
    ],
    &[(29, 1.25), (56, 1.25), (125, 1.5), (57, 6.0), (100, 1.5), (105, 1.0), (108, 1.0), (106, 1.0)],
];
const KU: f32 = 30.0;

fn named(code: u32) -> Option<&'static str> {
    Some(match code {
        14 => "⌫",
        15 => "⇥",
        58 => "⇪",
        28 => "↩",
        42 | 54 => "⇧",
        29 => "⌃",
        56 | 100 => "⌥",
        125 => "⌘",
        57 => "",
        105 => "←",
        106 => "→",
        108 => "↓",
        _ => return None,
    })
}

fn key_rects(sh: &Shell) -> (Rect, Vec<(u32, Rect)>) {
    let w = 15.0 * KU + 24.0;
    let h = 5.0 * KU + 50.0;
    let p = Rect::new((sh.w - w) / 2.0, sh.h - h - 100.0, w, h);
    let mut v = vec![];
    for (ri, row) in ROWS.iter().enumerate() {
        let mut x = p.x + 12.0;
        let y = p.y + 38.0 + ri as f32 * KU;
        for (code, u) in row.iter() {
            v.push((*code, Rect::new(x + 1.5, y + 1.5, u * KU - 3.0, KU - 3.0)));
            x += u * KU;
        }
    }
    (p, v)
}

pub fn layers(sh: &mut Shell) -> Vec<Layer> {
    if sh.chars.t <= 0.001 {
        return vec![];
    }
    let dark = sh.style.dark;
    let t = sh.chars.t;
    let ease = 1.0 - (1.0 - t).powi(3);
    let mut g = style::glass_menu(&sh.cfg.glass, dark);
    g.radius = 18.0;
    match sh.chars.mode {
        Mode::Chars => {
            let p = char_panel(sh);
            let tab = sh.chars.tab;
            let first = first_row(sh);
            let key = hash_of(&(tab, sh.chars.hover, dark, p.h as i32, first));
            let (pm, serial) = sh.cached(LayerId::CharViewer, key, p.w, p.h, |c, sh| {
                let f = sh.fonts.clone();
                let fg = style::text_primary(dark);
                let fg2 = style::text_secondary(dark);
                let x0 = (p.w - TABS.len() as f32 * TAB_W) / 2.0;
                for (i, (_, icon, _)) in TABS.iter().enumerate() {
                    let r = Rect::new(x0 + i as f32 * TAB_W, 8.0, TAB_W - 2.0, 26.0);
                    if i == tab {
                        c.fill_rrect(r, 7.0, if dark { rgba(255, 255, 255, 0.18) } else { rgba(0, 0, 0, 0.10) });
                    } else if sh.chars.hover == Some(2000 + i) {
                        c.fill_rrect(r, 7.0, rgba(128, 128, 128, 0.15));
                    }
                    let col = if i == tab { style::accent(1.0) } else { fg2 };
                    c.text_in(&f, r, 0.5, 15.0, Weight::Regular, col, icon);
                }
                c.text(&f, 14.0, 52.0, 11.0, Weight::Semibold, fg2, &TABS[tab.min(TABS.len() - 1)].0.to_uppercase());
                let chars = chars_of(tab);
                for (i, ch) in chars.iter().enumerate().skip(first * COLS).take(MAX_ROWS * COLS) {
                    let row = i / COLS - first;
                    let r =
                        Rect::new(12.0 + (i % COLS) as f32 * CELL, GRID_Y + row as f32 * CELL, CELL - 2.0, CELL - 2.0);
                    if sh.chars.hover == Some(1000 + i) {
                        c.fill_rrect(r, 8.0, rgba(128, 128, 128, 0.25));
                    }
                    c.text_in(&f, r, 0.5, if tab < EMOJI_TABS { 20.0 } else { 18.0 }, Weight::Regular, fg, ch);
                }
                let rows = rows_of(tab);
                if rows > MAX_ROWS {
                    let track = MAX_ROWS as f32 * CELL - 6.0;
                    let th = (track * MAX_ROWS as f32 / rows as f32).max(18.0);
                    let ty = GRID_Y + 3.0 + (track - th) * first as f32 / (rows - MAX_ROWS) as f32;
                    c.fill_rrect(Rect::new(p.w - 7.0, ty, 3.5, th), 1.75, rgba(128, 128, 128, 0.55));
                }
            });
            vec![Layer {
                id: LayerId::CharViewer,
                rect: p,
                glass: Some(g),
                tiles: vec![],
                content: pm,
                serial,
                opacity: ease,
                zoom: 0.95 + 0.05 * ease,
            }]
        }
        Mode::Keyboard => {
            let (p, keys) = key_rects(sh);
            let labels = sh.key_labels.clone();
            let layout = crate::sysinfo::layout_name(sh.layouts.get(sh.layout_idx).map(|s| s.as_str()).unwrap_or("us"))
                .to_string();
            let key = hash_of(&(
                sh.chars.hover,
                dark,
                sh.chars.shift,
                layout.clone(),
                labels.len(),
                labels.get(&16).cloned(),
                labels.get(&1016).cloned(),
            ));
            let shift = sh.chars.shift;
            let (pm, serial) = sh.cached(LayerId::KeyboardViewer, key, p.w, p.h, |c, sh| {
                let f = sh.fonts.clone();
                let fg = style::text_primary(dark);
                let fg2 = style::text_secondary(dark);
                c.text(&f, 14.0, 24.0, 13.0, Weight::Semibold, fg, "Keyboard Viewer");
                c.text_in(&f, Rect::new(0.0, 10.0, p.w - 14.0, 20.0), 1.0, 11.5, Weight::Regular, fg2, &layout);
                for (i, (code, r)) in keys.iter().enumerate() {
                    let r = r.translate(-p.x, -p.y);
                    let hot = sh.chars.hover == Some(i);
                    let pressed = shift && (*code == 42 || *code == 54);
                    c.fill_rrect(
                        r,
                        5.0,
                        if hot || pressed {
                            style::accent(0.85)
                        } else if dark {
                            rgba(255, 255, 255, 0.12)
                        } else {
                            rgba(255, 255, 255, 0.7)
                        },
                    );
                    let base =
                        named(*code).map(str::to_string).or_else(|| labels.get(code).cloned()).unwrap_or_default();
                    let l = if shift {
                        named(*code)
                            .map(str::to_string)
                            .or_else(|| labels.get(&(code + 1000)).cloned())
                            .unwrap_or_else(|| base.to_uppercase())
                    } else {
                        base
                    };
                    c.text_in(
                        &f,
                        r,
                        0.5,
                        12.5,
                        Weight::Regular,
                        if hot || pressed { rgba(255, 255, 255, 1.0) } else { fg },
                        &l,
                    );
                }
            });
            vec![Layer {
                id: LayerId::KeyboardViewer,
                rect: p,
                glass: Some(g),
                tiles: vec![],
                content: pm,
                serial,
                opacity: ease,
                zoom: 0.95 + 0.05 * ease,
            }]
        }
    }
}

fn hit(sh: &Shell, x: f32, y: f32) -> Option<(Rect, Option<usize>)> {
    if !sh.chars.open {
        return None;
    }
    match sh.chars.mode {
        Mode::Chars => {
            let p = char_panel(sh);
            if !p.contains(x, y) {
                return None;
            }
            let (lx, ly) = (x - p.x, y - p.y);
            if ly < 38.0 {
                let x0 = (p.w - TABS.len() as f32 * TAB_W) / 2.0;
                let i = ((lx - x0) / TAB_W).floor();
                if i >= 0.0 && (i as usize) < TABS.len() {
                    return Some((p, Some(2000 + i as usize)));
                }
                return Some((p, None));
            }
            if ly < GRID_Y || lx < 12.0 {
                return Some((p, None));
            }
            let col = ((lx - 12.0) / CELL) as usize;
            let row = ((ly - GRID_Y) / CELL) as usize + first_row(sh);
            let i = row * COLS + col;
            Some((p, (col < COLS && i < chars_of(sh.chars.tab).len()).then_some(1000 + i)))
        }
        Mode::Keyboard => {
            let (p, keys) = key_rects(sh);
            if !p.contains(x, y) {
                return None;
            }
            Some((p, keys.iter().position(|(_, r)| r.contains(x, y))))
        }
    }
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    let h = hit(sh, x, y).and_then(|h| h.1);
    sh.chars.hover = h;
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let (_, h) = hit(sh, x, y)?;
    let Some(h) = h else { return Some(vec![Action::Redraw]) };
    match sh.chars.mode {
        Mode::Chars if h >= 2000 => {
            sh.chars.tab = h - 2000;
            sh.chars.scroll = 0;
            sh.chars.scroll_acc = 0.0;
            Some(vec![Action::Redraw])
        }
        Mode::Chars => Some(vec![Action::TypeText(chars_of(sh.chars.tab)[h - 1000].clone()), Action::Redraw]),
        Mode::Keyboard => {
            let (_, keys) = key_rects(sh);
            let code = keys[h].0;
            if code == 42 || code == 54 {
                sh.chars.shift = !sh.chars.shift;
                return Some(vec![Action::Redraw]);
            }
            let acts = if sh.chars.shift {
                vec![Action::SendKeys(format!("shift+#{code}"))]
            } else {
                vec![Action::TypeKey(code)]
            };
            sh.chars.shift = false;
            let mut v = acts;
            v.push(Action::Redraw);
            Some(v)
        }
    }
}

/// Is the point over an open palette (so clicks don't go to windows)?
pub fn over(sh: &Shell, x: f32, y: f32) -> bool {
    hit(sh, x, y).is_some()
}
