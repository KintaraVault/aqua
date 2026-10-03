//! Frame profiling: render time, damaged (redrawn) area and glass blur work per output.
//!
//! The compositor records every frame; `aqua msg stats` prints [`report`], and with
//! `AQUA_PROFILE=1` a one-line summary is logged every few seconds.
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A rectangle as (x, y, w, h).
pub type R = (i32, i32, i32, i32);

const RECENT: usize = 240;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct OutputStats {
    pub frames: u64,
    /// Frames that turned out to have no damage (nothing submitted).
    pub empty: u64,
    /// Frames scanned out directly from a client buffer (no composition).
    pub scanout: u64,
    pub render_total: Duration,
    pub render_max: Duration,
    /// Sum of the damaged area (px) over all frames.
    pub damaged_px: u64,
    /// Sum of the output area (px) over all frames.
    pub output_px: u64,
    /// Render times (ms) of the most recent frames.
    pub recent: VecDeque<f32>,
}

impl OutputStats {
    /// Share of the screen that was redrawn on average (0–1).
    pub fn damage_ratio(&self) -> f64 {
        if self.output_px == 0 {
            0.0
        } else {
            self.damaged_px as f64 / self.output_px as f64
        }
    }
    pub fn avg_ms(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.render_total.as_secs_f64() * 1000.0 / self.frames as f64
        }
    }
    /// Render time percentile `p` (0–100) over the recent frames.
    pub fn percentile_ms(&self, p: f64) -> f32 {
        let mut v: Vec<f32> = self.recent.iter().copied().collect();
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let i = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
        v[i.min(v.len() - 1)]
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Stats {
    pub outputs: BTreeMap<String, OutputStats>,
    /// Glass backdrops captured and blurred.
    pub blur_captures: u64,
    /// Captures skipped by the blur rate limit (stale blur reused).
    pub blur_skipped: u64,
    /// Pixels captured for blurring.
    pub blur_px: u64,
    pub since: Option<Instant>,
}

static STATS: Mutex<Stats> =
    Mutex::new(Stats { outputs: BTreeMap::new(), blur_captures: 0, blur_skipped: 0, blur_px: 0, since: None });

fn with<T>(f: impl FnOnce(&mut Stats) -> T) -> T {
    let mut s = STATS.lock().unwrap_or_else(|e| e.into_inner());
    s.since.get_or_insert_with(Instant::now);
    f(&mut s)
}

/// `AQUA_PROFILE=1`: log frame statistics periodically.
pub fn profiling() -> bool {
    static P: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *P.get_or_init(|| std::env::var("AQUA_PROFILE").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Area covered by possibly overlapping rectangles (clipped to `bounds`).
pub fn union_area(rects: &[R], bounds: (i32, i32)) -> u64 {
    let clip: Vec<R> = rects
        .iter()
        .filter_map(|&(x, y, w, h)| {
            let (x0, y0) = (x.max(0), y.max(0));
            let (x1, y1) = ((x + w).min(bounds.0), (y + h).min(bounds.1));
            (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
        })
        .collect();
    if clip.is_empty() {
        return 0;
    }
    let mut xs: Vec<i32> = clip.iter().flat_map(|r| [r.0, r.2]).collect();
    xs.sort_unstable();
    xs.dedup();
    let mut area = 0u64;
    for win in xs.windows(2) {
        let (a, b) = (win[0], win[1]);
        let mut spans: Vec<(i32, i32)> = clip.iter().filter(|r| r.0 <= a && r.2 >= b).map(|r| (r.1, r.3)).collect();
        spans.sort_unstable();
        let (mut covered, mut cur): (i64, Option<(i32, i32)>) = (0, None);
        for (s, e) in spans {
            match cur {
                Some((cs, ce)) if s <= ce => cur = Some((cs, ce.max(e))),
                Some((cs, ce)) => {
                    covered += (ce - cs) as i64;
                    cur = Some((s, e));
                }
                None => cur = Some((s, e)),
            }
        }
        if let Some((cs, ce)) = cur {
            covered += (ce - cs) as i64;
        }
        area += covered as u64 * (b - a) as u64;
    }
    area
}

/// Kind of frame that was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    /// Composited with this damage (`None` = everything).
    Rendered(Option<&'a [R]>),
    /// Nothing changed.
    Empty,
    /// A client buffer went straight to the display.
    Scanout,
}

pub fn record_frame(output: &str, size: (i32, i32), took: Duration, frame: Frame<'_>) {
    let full = size.0.max(0) as u64 * size.1.max(0) as u64;
    with(|s| {
        let o = s.outputs.entry(output.to_string()).or_default();
        o.frames += 1;
        o.render_total += took;
        o.render_max = o.render_max.max(took);
        o.output_px += full;
        match frame {
            Frame::Rendered(Some(d)) => o.damaged_px += union_area(d, size).min(full),
            Frame::Rendered(None) => o.damaged_px += full,
            Frame::Empty => o.empty += 1,
            Frame::Scanout => o.scanout += 1,
        }
        o.recent.push_back(took.as_secs_f32() * 1000.0);
        if o.recent.len() > RECENT {
            o.recent.pop_front();
        }
    })
}

pub fn record_blur(px: u64, skipped: bool) {
    with(|s| {
        if skipped {
            s.blur_skipped += 1;
        } else {
            s.blur_captures += 1;
            s.blur_px += px;
        }
    })
}

pub fn snapshot() -> Stats {
    with(|s| s.clone())
}

pub fn reset() {
    with(|s| *s = Stats { since: Some(Instant::now()), ..Default::default() })
}

/// Human readable report.
pub fn format(s: &Stats) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let secs = s.since.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0).max(0.001);
    let _ = writeln!(o, "over {secs:.1} s");
    for (name, st) in &s.outputs {
        let _ = writeln!(
            o,
            "{name}: {} frames ({:.1}/s), {} empty, {} direct scanout; render avg {:.2} ms, p95 {:.2} ms, max {:.2} ms; redrawn {:.1}% of the screen",
            st.frames,
            st.frames as f64 / secs,
            st.empty,
            st.scanout,
            st.avg_ms(),
            st.percentile_ms(95.0),
            st.render_max.as_secs_f64() * 1000.0,
            st.damage_ratio() * 100.0,
        );
    }
    let _ = writeln!(
        o,
        "glass: {} blur captures ({:.1} Mpx), {} reused by the rate limit",
        s.blur_captures,
        s.blur_px as f64 / 1e6,
        s.blur_skipped
    );
    o
}

pub fn report() -> String {
    format(&snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_of_rectangles() {
        assert_eq!(union_area(&[], (100, 100)), 0);
        assert_eq!(union_area(&[(0, 0, 10, 10)], (100, 100)), 100);
        // Overlap counted once.
        assert_eq!(union_area(&[(0, 0, 10, 10), (5, 5, 10, 10)], (100, 100)), 175);
        // Contained.
        assert_eq!(union_area(&[(0, 0, 10, 10), (2, 2, 3, 3)], (100, 100)), 100);
        // Clipped to the output.
        assert_eq!(union_area(&[(-5, -5, 10, 10), (95, 95, 10, 10)], (100, 100)), 50);
        // Disjoint in the same column.
        assert_eq!(union_area(&[(0, 0, 10, 10), (0, 20, 10, 10)], (100, 100)), 200);
    }

    #[test]
    fn output_statistics() {
        let mut o = OutputStats::default();
        assert_eq!(o.percentile_ms(95.0), 0.0);
        o.recent.extend([1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(o.percentile_ms(50.0), 3.0);
        assert_eq!(o.percentile_ms(100.0), 100.0);
        o.frames = 2;
        o.render_total = Duration::from_millis(6);
        assert!((o.avg_ms() - 3.0).abs() < 1e-9);
        o.output_px = 200;
        o.damaged_px = 50;
        assert!((o.damage_ratio() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn recording_and_report() {
        reset();
        record_frame("T-1", (100, 100), Duration::from_millis(2), Frame::Rendered(Some(&[(0, 0, 50, 100)])));
        record_frame("T-1", (100, 100), Duration::from_millis(1), Frame::Empty);
        record_frame("T-1", (100, 100), Duration::from_millis(1), Frame::Rendered(None));
        record_blur(1000, false);
        record_blur(1000, true);
        let s = snapshot();
        let o = &s.outputs["T-1"];
        assert_eq!((o.frames, o.empty, o.damaged_px, o.output_px), (3, 1, 15000, 30000));
        assert_eq!((s.blur_captures, s.blur_skipped, s.blur_px), (1, 1, 1000));
        let r = format(&s);
        assert!(r.contains("T-1: 3 frames"), "{r}");
        assert!(r.contains("redrawn 50.0%"), "{r}");
        assert!(r.contains("1 reused"), "{r}");
    }
}
