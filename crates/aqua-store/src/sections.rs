#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Feed {
    Collection(&'static str),
    Category(&'static str, Option<&'static str>),
}

impl Feed {
    pub fn id(&self) -> String {
        match self {
            Feed::Collection(c) => format!("col:{c}"),
            Feed::Category(c, None) => format!("cat:{c}"),
            Feed::Category(c, Some(s)) => format!("cat:{c}/{s}"),
        }
    }

    pub fn local_categories(&self) -> Vec<&'static str> {
        match self {
            Feed::Category(_, Some(s)) => vec![s],
            Feed::Category(c, None) => match *c {
                "audiovideo" => vec!["AudioVideo", "Audio", "Video"],
                "development" => vec!["Development"],
                "education" => vec!["Education"],
                "game" => vec!["Game"],
                "graphics" => vec!["Graphics"],
                "network" => vec!["Network"],
                "office" => vec!["Office"],
                "science" => vec!["Science"],
                "system" => vec!["System"],
                "utility" => vec!["Utility"],
                _ => vec![],
            },
            Feed::Collection(_) => vec![],
        }
    }
}

pub fn feed_from_id(id: &str) -> Option<Feed> {
    let leak = |s: &str| -> &'static str {
        for c in CATEGORIES {
            if c.feed_cat() == s {
                return c.feed_cat();
            }
            if c.feed_sub() == Some(s) {
                return c.feed_sub().unwrap();
            }
        }
        for s2 in SECTIONS.iter().flat_map(|x| x.shelves.iter()) {
            if let Feed::Category(a, b) = &s2.1 {
                if *a == s {
                    return a;
                }
                if *b == Some(s) {
                    return b.unwrap();
                }
            }
            if let Feed::Collection(a) = &s2.1 {
                if *a == s {
                    return a;
                }
            }
        }
        for c in COLLECTIONS {
            if *c == s {
                return c;
            }
        }
        ""
    };
    if let Some(c) = id.strip_prefix("col:") {
        let c = leak(c);
        return (!c.is_empty()).then_some(Feed::Collection(c));
    }
    let rest = id.strip_prefix("cat:")?;
    let (c, s) = match rest.split_once('/') {
        Some((c, s)) => (leak(c), Some(leak(s))),
        None => (leak(rest), None),
    };
    if c.is_empty() || s == Some("") {
        return None;
    }
    Some(Feed::Category(c, s))
}

pub const COLLECTIONS: &[&str] = &["popular", "trending", "recently-updated", "recently-added", "verified"];

pub struct Category {
    pub id: &'static str,
    pub title: &'static str,
    pub feed: Feed,
    pub glyph: &'static str,
    pub tint: u32,
}

impl Category {
    pub fn feed_cat(&self) -> &'static str {
        match self.feed {
            Feed::Category(c, _) => c,
            Feed::Collection(c) => c,
        }
    }

    pub fn feed_sub(&self) -> Option<&'static str> {
        match self.feed {
            Feed::Category(_, s) => s,
            _ => None,
        }
    }
}

macro_rules! cat {
    ($id:expr, $title:expr, $c:expr, $s:expr, $g:expr, $t:expr) => {
        Category { id: $id, title: $title, feed: Feed::Category($c, $s), glyph: $g, tint: $t }
    };
}

pub const CATEGORIES: &[Category] = &[
    cat!("developer-tools", "Developer Tools", "development", None, "hammer", 0x3478f6),
    cat!("education", "Education", "education", None, "graduationcap", 0x5b5b60),
    cat!("entertainment", "Entertainment", "audiovideo", Some("Player"), "popcorn", 0xff453a),
    cat!("finance", "Finance", "office", Some("Finance"), "banknote", 0x30b350),
    cat!("games", "Games", "game", None, "rocket", 0x2f8cff),
    cat!("graphics", "Graphics & Design", "graphics", None, "paintpalette", 0xff9f0a),
    cat!("medical", "Medical", "science", Some("MedicalSoftware"), "stethoscope", 0x7d5cf0),
    cat!("music", "Music", "audiovideo", Some("Audio"), "musicnotes", 0x2fc759),
    cat!("news", "News", "network", Some("News"), "newspaper", 0x636366),
    cat!("photo-video", "Photo & Video", "graphics", Some("Photography"), "camera", 0x636366),
    cat!("productivity", "Productivity", "office", None, "paperplane", 0x2f8cff),
    cat!("science", "Science", "science", None, "atom", 0x32ade6),
    cat!("social", "Social Networking", "network", Some("Chat"), "bubbles", 0xff375f),
    cat!("security", "Security", "system", Some("Security"), "lockshield", 0x5e5ce6),
    cat!("system", "System", "system", None, "gearshape", 0x8e8e93),
    cat!("utilities", "Utilities", "utility", None, "calculator", 0x636366),
    cat!("browsers", "Web Browsers", "network", Some("WebBrowser"), "safari", 0x2f8cff),
    cat!("video", "Video", "audiovideo", Some("Video"), "film", 0xbf5af2),
];

pub fn category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub glyph: &'static str,
    pub hero: Feed,
    pub shelves: &'static [(&'static str, Feed)],
}

pub const SECTIONS: &[Section] = &[
    Section {
        id: "arcade",
        title: "Arcade",
        glyph: "joystick",
        hero: Feed::Category("game", Some("ArcadeGame")),
        shelves: &[
            ("Arcade Classics", Feed::Category("game", Some("ArcadeGame"))),
            ("Action Games", Feed::Category("game", Some("ActionGame"))),
            ("Puzzle & Logic", Feed::Category("game", Some("LogicGame"))),
            ("Board & Card Games", Feed::Category("game", Some("BoardGame"))),
            ("Emulators", Feed::Category("game", Some("Emulator"))),
        ],
    },
    Section {
        id: "create",
        title: "Create",
        glyph: "paintbrush",
        hero: Feed::Category("graphics", None),
        shelves: &[
            ("Graphics & Design", Feed::Category("graphics", None)),
            ("Photography", Feed::Category("graphics", Some("Photography"))),
            ("Music & Audio", Feed::Category("audiovideo", Some("Audio"))),
            ("Video Editing", Feed::Category("audiovideo", Some("AudioVideoEditing"))),
            ("3D & Modeling", Feed::Category("graphics", Some("3DGraphics"))),
        ],
    },
    Section {
        id: "work",
        title: "Work",
        glyph: "paperplane",
        hero: Feed::Category("office", None),
        shelves: &[
            ("Productivity", Feed::Category("office", None)),
            ("Communication", Feed::Category("network", Some("Chat"))),
            ("Email", Feed::Category("network", Some("Email"))),
            ("Finance", Feed::Category("office", Some("Finance"))),
            ("Web Browsers", Feed::Category("network", Some("WebBrowser"))),
        ],
    },
    Section {
        id: "play",
        title: "Play",
        glyph: "rocket",
        hero: Feed::Category("game", None),
        shelves: &[
            ("Top Games", Feed::Category("game", None)),
            ("Strategy", Feed::Category("game", Some("StrategyGame"))),
            ("Adventure", Feed::Category("game", Some("AdventureGame"))),
            ("Media Players", Feed::Category("audiovideo", Some("Player"))),
            ("Kids", Feed::Category("game", Some("KidsGame"))),
        ],
    },
    Section {
        id: "develop",
        title: "Develop",
        glyph: "hammer",
        hero: Feed::Category("development", None),
        shelves: &[
            ("Developer Tools", Feed::Category("development", None)),
            ("Code Editors & IDEs", Feed::Category("development", Some("IDE"))),
            ("Version Control", Feed::Category("development", Some("RevisionControl"))),
            ("Debugging", Feed::Category("development", Some("Debugger"))),
            ("Science & Engineering", Feed::Category("science", None)),
        ],
    },
];

pub fn section(id: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.id == id)
}

pub const DISCOVER: &[(&str, Feed)] = &[
    ("Top Apps", Feed::Collection("popular")),
    ("Trending Now", Feed::Collection("trending")),
    ("New & Updated", Feed::Collection("recently-updated")),
    ("Fresh Arrivals", Feed::Collection("recently-added")),
    ("Verified Developers", Feed::Collection("verified")),
];

pub fn category_title(fd: &str) -> String {
    let t = match fd.to_ascii_lowercase().as_str() {
        "audiovideo" | "audio" | "video" => "Audio & Video",
        "development" => "Developer Tools",
        "education" => "Education",
        "game" => "Games",
        "graphics" => "Graphics & Design",
        "network" => "Networking",
        "office" => "Productivity",
        "science" => "Science",
        "system" => "System",
        "utility" => "Utilities",
        "photography" => "Photo & Video",
        "chat" | "instantmessaging" => "Social Networking",
        "finance" => "Finance",
        "webbrowser" => "Web Browsers",
        _ => "",
    };
    if t.is_empty() {
        fd.to_string()
    } else {
        t.to_string()
    }
}

pub fn primary_category(cats: &[String]) -> String {
    const MAIN: [&str; 10] = [
        "game",
        "development",
        "graphics",
        "audiovideo",
        "office",
        "education",
        "science",
        "network",
        "utility",
        "system",
    ];
    for m in MAIN {
        if cats.iter().any(|c| c.eq_ignore_ascii_case(m)) {
            return category_title(m);
        }
    }
    cats.iter()
        .find(|c| !c.contains("GTK") && !c.contains("Qt") && *c != "GNOME" && *c != "KDE")
        .map(|c| category_title(c))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_ids_roundtrip() {
        for s in SECTIONS {
            for (_, f) in s.shelves {
                assert_eq!(feed_from_id(&f.id()).as_ref(), Some(f), "{}", f.id());
            }
        }
        for c in CATEGORIES {
            assert_eq!(feed_from_id(&c.feed.id()), Some(c.feed.clone()));
        }
        for (_, f) in DISCOVER {
            assert_eq!(feed_from_id(&f.id()).as_ref(), Some(f));
        }
        assert_eq!(feed_from_id("cat:nope"), None);
        assert_eq!(feed_from_id("col:x"), None);
    }

    #[test]
    fn categories_are_unique() {
        let mut ids: Vec<&str> = CATEGORIES.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CATEGORIES.len());
        assert_eq!(CATEGORIES.len() % 3, 0);
    }

    #[test]
    fn primary() {
        assert_eq!(primary_category(&["GTK".into(), "Graphics".into()]), "Graphics & Design");
        assert_eq!(primary_category(&["GNOME".into(), "Chat".into()]), "Social Networking");
        assert_eq!(primary_category(&[]), "");
        assert_eq!(Feed::Category("game", None).local_categories(), vec!["Game"]);
    }
}
