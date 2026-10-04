//! View geometry shared by drawing and hit-testing: groups, rows, icon grid, list columns.
use super::fs::{self, Entry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    None,
    Name,
    Kind,
    Modified,
    Created,
    Opened,
    Size,
    Tags,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::None,
        Group::Name,
        Group::Kind,
        Group::Modified,
        Group::Created,
        Group::Opened,
        Group::Size,
        Group::Tags,
    ];
    pub fn from_i32(i: i32) -> Group {
        Self::ALL.get(i.max(0) as usize).copied().unwrap_or(Group::None)
    }
    pub fn to_i32(self) -> i32 {
        Self::ALL.iter().position(|g| *g == self).unwrap_or(0) as i32
    }
    pub fn label(self) -> &'static str {
        match self {
            Group::None => "None",
            Group::Name => "Name",
            Group::Kind => "Kind",
            Group::Modified => "Date Modified",
            Group::Created => "Date Created",
            Group::Opened => "Date Last Opened",
            Group::Size => "Size",
            Group::Tags => "Tags",
        }
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Relative date bucket: (order, label).
pub fn date_group(t: i64, now: i64) -> (i64, String) {
    if t <= 0 {
        return (10_000, crate::tr("Unknown").into());
    }
    let (ny, _, nd) = fs::local_parts(now);
    let (y, m, d) = fs::local_parts(t);
    let diff = nd - d;
    match diff {
        i64::MIN..=0 => (0, crate::tr("Today").into()),
        1 => (1, crate::tr("Yesterday").into()),
        2..=7 => (2, crate::tr("Previous 7 Days").into()),
        8..=30 => (3, crate::tr("Previous 30 Days").into()),
        _ if y == ny => (4 + (11 - m as i64), crate::tr(MONTHS[m.clamp(0, 11) as usize]).into()),
        _ => (100 + (ny - y) as i64, y.to_string()),
    }
}

fn kind_group(e: &Entry) -> (i64, &'static str) {
    if e.app.is_some() || e.kind == 5 {
        return (1, "Applications");
    }
    let ext = e.ext.to_lowercase();
    match e.kind {
        0 => (0, "Folders"),
        2 => (3, "Images"),
        3 => (5, "Music"),
        4 => (4, "Movies"),
        6 => (7, "Text"),
        7 => (9, "Archives"),
        8 => (2, "PDF Documents"),
        _ if matches!(ext.as_str(), "xls" | "xlsx" | "ods" | "numbers" | "csv") => (8, "Spreadsheets"),
        _ if matches!(ext.as_str(), "ppt" | "pptx" | "odp" | "key") => (8, "Presentations"),
        _ if matches!(ext.as_str(), "doc" | "docx" | "odt" | "rtf" | "pages") => (6, "Documents"),
        _ => (10, "Other"),
    }
}

fn size_group(e: &Entry) -> (i64, &'static str) {
    if e.is_dir {
        return (10, "Folders");
    }
    match e.size {
        0 => (6, "Zero bytes"),
        1..=99_999 => (5, "Less than 100 KB"),
        100_000..=999_999 => (4, "100 KB to 1 MB"),
        1_000_000..=99_999_999 => (3, "1 MB to 100 MB"),
        100_000_000..=999_999_999 => (2, "100 MB to 1 GB"),
        _ => (1, "1 GB or more"),
    }
}

/// Group of an item: (order, label). Order sorts groups; ties sort by label.
pub fn group_of(e: &Entry, by: Group, tags: &[String], now: i64) -> (i64, String) {
    match by {
        Group::None => (0, String::new()),
        Group::Name => {
            let c = e.name.chars().find(|c| !c.is_whitespace()).unwrap_or('#');
            if c.is_ascii_digit() {
                (0, "0–9".into())
            } else if c.is_alphabetic() {
                (1, c.to_uppercase().collect())
            } else {
                (2, "#".into())
            }
        }
        Group::Kind => {
            let (o, l) = kind_group(e);
            (o, crate::tr(l).into())
        }
        Group::Modified => date_group(e.mtime, now),
        Group::Created => date_group(e.ctime, now),
        Group::Opened => date_group(e.atime, now),
        Group::Size => {
            let (o, l) = size_group(e);
            (o, crate::tr(l).into())
        }
        Group::Tags => match fs::TAGS.iter().position(|(t, _)| tags.iter().any(|x| x == t)) {
            Some(i) => (i as i64, crate::tr(fs::TAGS[i].0).into()),
            None => (100, crate::tr("No Tags").into()),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub header: String,
    pub start: usize,
    pub count: usize,
}

/// Split items (already ordered by group) into rows of at most `per_row`, with a header row
/// whenever the group label changes. Empty labels mean "no groups".
pub fn rows(labels: &[String], per_row: usize) -> Vec<Row> {
    let per_row = per_row.max(1);
    let mut out = vec![];
    let mut i = 0;
    while i < labels.len() {
        let g = &labels[i];
        let end = (i..labels.len()).find(|&k| &labels[k] != g).unwrap_or(labels.len());
        if !g.is_empty() {
            out.push(Row { header: g.clone(), start: i, count: 0 });
        }
        let mut k = i;
        while k < end {
            let n = per_row.min(end - k);
            out.push(Row { header: String::new(), start: k, count: n });
            k += n;
        }
        i = end;
    }
    out
}

/// Top of each row plus the total height.
pub fn offsets(rows: &[Row], header_h: f32, row_h: f32) -> (Vec<f32>, f32) {
    let mut y = 0.0;
    let mut v = Vec::with_capacity(rows.len());
    for r in rows {
        v.push(y);
        y += if r.count == 0 { header_h } else { row_h };
    }
    (v, y)
}

/// Row containing content-y.
pub fn row_at(offs: &[f32], total: f32, y: f32) -> Option<usize> {
    if y < 0.0 || y >= total || offs.is_empty() {
        return None;
    }
    Some(offs.partition_point(|&o| o <= y).saturating_sub(1))
}

/// Icon view metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub icon: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: usize,
    pub pad: f32,
}

pub const HEADER_ICON: f32 = 34.0;
pub const HEADER_LIST: f32 = 26.0;
pub const ROW_LIST: f32 = 22.0;
pub const LIST_TOP: f32 = 30.0;
pub const COLUMN_W: f32 = 240.0;

impl Grid {
    pub fn new(width: f32, icon: f32, spacing: f32, text: f32, info: bool) -> Grid {
        let icon = icon.clamp(16.0, 512.0);
        let cell_w = (icon + 63.0).max(84.0) + spacing;
        let cell_h = icon + 24.0 + 2.0 * (text + 4.0) + if info { text + 2.0 } else { 0.0 } + spacing * 0.5;
        let cols = (((width - 16.0) / cell_w).floor() as usize).max(1);
        let pad = ((width - 16.0 - cols as f32 * cell_w) / 2.0).max(0.0) + 8.0;
        Grid { icon, cell_w, cell_h, cols, pad }
    }

    /// Item under content point (x, y) given the row layout.
    pub fn hit(&self, rows: &[Row], offs: &[f32], total: f32, x: f32, y: f32) -> Option<usize> {
        let r = &rows[row_at(offs, total, y)?];
        if r.count == 0 {
            return None;
        }
        let c = ((x - self.pad) / self.cell_w).floor();
        if c < 0.0 || c as usize >= r.count {
            return None;
        }
        let cx = self.pad + c * self.cell_w;
        let ix = cx + (self.cell_w - self.icon) / 2.0 - 4.0;
        let iy = offs[row_at(offs, total, y)?] + 20.0;
        if x < ix || x > ix + self.icon + 8.0 || y < iy {
            let lx = cx + 2.0;
            if x < lx || x > cx + self.cell_w - 2.0 || y < iy + self.icon {
                return None;
            }
        }
        Some(r.start + c as usize)
    }

    /// Whether content point (x, y) is on the icon or label of the cell at (cx, cy).
    pub fn cell_hit(&self, cx: f32, cy: f32, x: f32, y: f32) -> bool {
        let ix = cx + (self.cell_w - self.icon) / 2.0 - 4.0;
        let iy = cy + 20.0;
        let bottom = cy + self.cell_h - 4.0;
        if y > bottom {
            return false;
        }
        (x >= ix && x <= ix + self.icon + 8.0 && y >= iy)
            || (x >= cx + 2.0 && x <= cx + self.cell_w - 2.0 && y >= iy + self.icon)
    }

    /// Whether the icon of the cell at (cx, cy) touches the rectangle.
    pub fn cell_touches(&self, cx: f32, cy: f32, r: (f32, f32, f32, f32)) -> bool {
        let (x0, y0, x1, y1) = (r.0.min(r.2), r.1.min(r.3), r.0.max(r.2), r.1.max(r.3));
        let ix = cx + (self.cell_w - self.icon) / 2.0 - 4.0;
        ix + self.icon + 8.0 >= x0 && ix <= x1 && cy + self.cell_h - 4.0 >= y0 && cy + 20.0 <= y1
    }

    /// Items whose cell touches the rectangle (content coordinates).
    pub fn in_rect(&self, rows: &[Row], offs: &[f32], r: (f32, f32, f32, f32)) -> Vec<usize> {
        let (x0, y0, x1, y1) = (r.0.min(r.2), r.1.min(r.3), r.0.max(r.2), r.1.max(r.3));
        let mut v = vec![];
        for (ri, row) in rows.iter().enumerate() {
            if row.count == 0 {
                continue;
            }
            let top = offs[ri] + 20.0;
            let bottom = offs[ri] + self.cell_h - 4.0;
            if bottom < y0 || top > y1 {
                continue;
            }
            for c in 0..row.count {
                let cx = self.pad + c as f32 * self.cell_w + (self.cell_w - self.icon) / 2.0 - 4.0;
                if cx + self.icon + 8.0 >= x0 && cx <= x1 {
                    v.push(row.start + c);
                }
            }
        }
        v
    }
}

/// List view columns: (key, x, width). Key: 0 name, 1 kind, 2 modified, 3 size, 4 created, 5 opened, 6 tags.
pub fn list_columns(width: f32, visible: &[i32], widths: &dyn Fn(i32) -> f32) -> Vec<(i32, f32, f32)> {
    const ORDER: [i32; 6] = [2, 4, 5, 3, 1, 6];
    const NAME_MIN: f32 = 200.0;
    const COL_MIN: f32 = 56.0;
    let avail = width - 16.0;
    let mut others: Vec<(i32, f32)> =
        ORDER.iter().filter(|k| visible.contains(k)).map(|&k| (k, widths(k).clamp(50.0, 600.0))).collect();
    let mut used: f32 = others.iter().map(|o| o.1).sum();
    if used + NAME_MIN > avail && !others.is_empty() {
        let room = (avail - NAME_MIN).max(0.0);
        let min_total = COL_MIN * others.len() as f32;
        if room >= min_total {
            let scale = (room - min_total) / (used - min_total).max(1.0);
            for o in &mut others {
                o.1 = COL_MIN + (o.1 - COL_MIN).max(0.0) * scale;
            }
        } else {
            let keep = (room / COL_MIN).floor() as usize;
            others.truncate(keep);
            for o in &mut others {
                o.1 = COL_MIN;
            }
        }
        used = others.iter().map(|o| o.1).sum();
    }
    let name_w = (avail - used).max(NAME_MIN);
    let mut x = 0.0;
    let mut v = vec![(0, x, name_w)];
    x += name_w;
    for (k, w) in others {
        v.push((k, x, w));
        x += w;
    }
    v
}

pub fn default_width(key: i32) -> f32 {
    match key {
        2 | 4 | 5 => 168.0,
        3 => 84.0,
        1 => 140.0,
        6 => 84.0,
        _ => 260.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_columns_fit_narrow_windows() {
        let w = |k: i32| default_width(k);
        let wide = list_columns(1200.0, &[1, 2, 3], &w);
        assert_eq!(wide.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 2, 3, 1]);
        assert_eq!(wide[1].2, 168.0);
        for width in [600.0, 420.0, 300.0] {
            let cols = list_columns(width, &[1, 2, 3], &w);
            let end = cols.last().map(|c| c.1 + c.2).unwrap();
            assert!(end <= width - 16.0 + 0.5, "{width}: {cols:?}");
            assert!(cols[0].2 >= 200.0);
        }
        assert_eq!(list_columns(300.0, &[1, 2, 3], &w).len(), 2);
    }

    fn entry(name: &str, kind: i32, size: u64, mtime: i64) -> Entry {
        Entry {
            name: name.into(),
            path: format!("/x/{name}").into(),
            is_dir: kind == 0,
            size,
            mtime,
            ctime: mtime,
            atime: mtime,
            kind,
            ext: fs::split_ext(name).1.trim_start_matches('.').to_uppercase(),
            label: String::new(),
            app: None,
            orig: None,
            mode: 0o644,
        }
    }

    #[test]
    fn row_building() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            rows(&l(&["", "", "", "", ""]), 2),
            vec![
                Row { header: "".into(), start: 0, count: 2 },
                Row { header: "".into(), start: 2, count: 2 },
                Row { header: "".into(), start: 4, count: 1 }
            ]
        );
        let r = rows(&l(&["A", "A", "A", "B"]), 2);
        assert_eq!(r.len(), 5);
        assert_eq!(r[0], Row { header: "A".into(), start: 0, count: 0 });
        assert_eq!(r[2], Row { header: "".into(), start: 2, count: 1 });
        assert_eq!(r[3].header, "B");
        assert!(rows(&[], 4).is_empty());
        let (o, total) = offsets(&r, 30.0, 100.0);
        assert_eq!(o, vec![0.0, 30.0, 130.0, 230.0, 260.0]);
        assert_eq!(total, 360.0);
        assert_eq!(row_at(&o, total, 0.0), Some(0));
        assert_eq!(row_at(&o, total, 129.0), Some(1));
        assert_eq!(row_at(&o, total, 130.0), Some(2));
        assert_eq!(row_at(&o, total, 359.0), Some(4));
        assert_eq!(row_at(&o, total, 360.0), None);
    }

    #[test]
    fn grid_metrics_and_hits() {
        let g = Grid::new(800.0, 64.0, 0.0, 12.0, false);
        assert_eq!(g.cell_w, 127.0);
        assert_eq!(g.cols, 6);
        let labels = vec![String::new(); 10];
        let r = rows(&labels, g.cols);
        let (o, t) = offsets(&r, HEADER_ICON, g.cell_h);
        let icon_x = g.pad + (g.cell_w - 64.0) / 2.0 + 10.0;
        assert_eq!(g.hit(&r, &o, t, icon_x, 40.0), Some(0));
        assert_eq!(g.hit(&r, &o, t, icon_x + g.cell_w, 40.0), Some(1));
        assert_eq!(g.hit(&r, &o, t, icon_x, g.cell_h + 40.0), Some(6));
        assert_eq!(g.hit(&r, &o, t, icon_x + 4.0 * g.cell_w, g.cell_h + 40.0), None, "past the last item");
        assert_eq!(g.hit(&r, &o, t, icon_x, 5.0), None, "gap above the icons");
        let sel = g.in_rect(&r, &o, (0.0, 0.0, g.pad + g.cell_w * 1.5, 50.0));
        assert_eq!(sel, vec![0, 1]);
        let all = g.in_rect(&r, &o, (0.0, 0.0, 2000.0, 2000.0));
        assert_eq!(all.len(), 10);
        let big = Grid::new(800.0, 128.0, 20.0, 13.0, true);
        assert!(big.cols < g.cols && big.cell_h > g.cell_h);
        assert_eq!(Grid::new(50.0, 64.0, 0.0, 12.0, false).cols, 1);
    }

    #[test]
    fn grouping() {
        let now = fs::now_secs();
        let today = entry("b.png", 2, 5, now);
        let old = entry("a.pdf", 8, 5_000_000, now - 400 * 86400);
        assert_eq!(group_of(&today, Group::Modified, &[], now).0, 0);
        assert_eq!(group_of(&entry("y", 1, 0, now - 86400 - 60), Group::Modified, &[], now).1, crate::tr("Yesterday"));
        assert!(group_of(&old, Group::Modified, &[], now).0 >= 100);
        assert_eq!(group_of(&old, Group::Kind, &[], now).1, crate::tr("PDF Documents"));
        assert_eq!(group_of(&entry("d", 0, 0, now), Group::Kind, &[], now).0, 0);
        assert_eq!(group_of(&old, Group::Size, &[], now).1, crate::tr("1 MB to 100 MB"));
        assert_eq!(group_of(&entry("z", 1, 0, now), Group::Size, &[], now).1, crate::tr("Zero bytes"));
        assert_eq!(group_of(&today, Group::Name, &[], now).1, "B");
        assert_eq!(group_of(&entry("7up", 1, 0, now), Group::Name, &[], now).1, "0–9");
        assert_eq!(group_of(&entry("ёж", 1, 0, now), Group::Name, &[], now).1, "Ё");
        assert_eq!(group_of(&today, Group::Tags, &["Blue".into(), "Red".into()], now).1, crate::tr("Red"));
        assert_eq!(group_of(&today, Group::Tags, &[], now).0, 100);
        assert_eq!(group_of(&today, Group::None, &[], now), (0, String::new()));
        for g in Group::ALL {
            assert_eq!(Group::from_i32(g.to_i32()), g);
        }
    }

    #[test]
    fn columns_fill_the_width() {
        let c = list_columns(1000.0, &[2, 3, 1], &default_width);
        assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 2, 3, 1]);
        let end = c.last().map(|x| x.1 + x.2).unwrap();
        assert_eq!(end, 1000.0 - 16.0);
        let narrow = list_columns(600.0, &[2, 3, 1, 4, 5, 6], &default_width);
        assert!((narrow[0].2 - 200.0).abs() < 0.01);
        assert_eq!(narrow.len(), 7);
        assert_eq!(narrow[1].0, 2);
        assert_eq!(narrow[2].0, 4);
    }
}
