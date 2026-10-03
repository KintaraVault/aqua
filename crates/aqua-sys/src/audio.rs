//! Output volume through PipeWire (WirePlumber `wpctl`), PulseAudio `pactl` fallback.
use crate::{have, run, spawn_cmd};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audio {
    pub available: bool,
    pub volume: f32,
    pub muted: bool,
    pub device: String,
    pub input_volume: f32,
    pub backend: &'static str,
}

fn parse_wpctl(s: &str) -> Option<(f32, bool)> {
    let v = s.split_whitespace().nth(1)?.parse().ok()?;
    Some((v, s.contains("MUTED")))
}

/// Percentage of the first channel in `pactl get-sink-volume` output.
fn parse_pactl_volume(s: &str) -> f32 {
    s.split('/').nth(1).and_then(|s| s.trim().trim_end_matches('%').parse::<f32>().ok()).unwrap_or(0.0)
}

pub fn probe() -> Audio {
    if have("wpctl") {
        if let Some((v, m)) = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).as_deref().and_then(parse_wpctl) {
            let device = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"])
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.contains("node.description"))
                        .and_then(|l| l.split('"').nth(1))
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Speakers".into());
            let input = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
                .as_deref()
                .and_then(parse_wpctl)
                .map(|x| x.0)
                .unwrap_or(0.0);
            return Audio { available: true, volume: v, muted: m, device, input_volume: input, backend: "pipewire" };
        }
    }
    if have("pactl") {
        if let Some(v) = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]) {
            let pct = parse_pactl_volume(&v);
            let muted = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).map(|s| s.contains("yes")).unwrap_or(false);
            return Audio {
                available: true,
                volume: pct / 100.0,
                muted,
                device: "Speakers".into(),
                input_volume: 0.0,
                backend: "pulseaudio",
            };
        }
    }
    Audio::default()
}

/// Set the output volume, 0..1 (clamped).
pub fn set_volume(v: f32) {
    let v = v.clamp(0.0, 1.0);
    crate::patch(|s| {
        s.audio.volume = v;
        s.audio.muted = false;
    });
    spawn_cmd(move || {
        let pct = format!("{:.0}%", v * 100.0);
        if run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{v:.3}")]).is_some() {
            let _ = run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
        } else {
            let _ = run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &pct]);
            let _ = run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"]);
        }
    });
}

pub fn set_muted(m: bool) {
    crate::patch(|s| s.audio.muted = m);
    spawn_cmd(move || {
        let a = if m { "1" } else { "0" };
        if run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", a]).is_none() {
            let _ = run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", a]);
        }
    });
}

pub fn set_input_volume(v: f32) {
    let v = v.clamp(0.0, 1.0);
    spawn_cmd(move || {
        if run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SOURCE@", &format!("{v:.3}")]).is_none() {
            let _ = run("pactl", &["set-source-volume", "@DEFAULT_SOURCE@", &format!("{:.0}%", v * 100.0)]);
        }
    });
}

/// Play a short UI sound (alert / volume feedback) through PipeWire/Pulse.
pub fn play_sound(name: &str) {
    let name = name.to_string();
    std::thread::spawn(move || {
        let candidates = [
            format!("/usr/share/sounds/freedesktop/stereo/{name}.oga"),
            format!("/usr/share/sounds/freedesktop/stereo/{name}.ogg"),
            "/usr/share/sounds/freedesktop/stereo/bell.oga".to_string(),
        ];
        if let Some(f) = candidates.iter().find(|p| std::path::Path::new(p).exists()) {
            if run("pw-play", &[f]).is_none() {
                let _ = run("paplay", &[f]);
            }
        } else if run("canberra-gtk-play", &["-i", &name]).is_none() {
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_output() {
        assert_eq!(parse_wpctl("Volume: 0.40"), Some((0.40, false)));
        assert_eq!(parse_wpctl("Volume: 1.00 [MUTED]"), Some((1.0, true)));
        assert_eq!(parse_wpctl("garbage"), None);
        assert_eq!(parse_wpctl(""), None);
    }

    #[test]
    fn pactl_output() {
        let out = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB";
        assert_eq!(parse_pactl_volume(out), 50.0);
        assert_eq!(parse_pactl_volume("nothing"), 0.0);
    }
}
