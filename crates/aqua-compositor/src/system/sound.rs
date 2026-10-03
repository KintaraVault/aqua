//! Event sounds from the freedesktop sound theme.

/// Camera shutter (freedesktop sound theme), silent when the theme is missing.
pub fn play_screenshot_sound() {
    for n in ["screen-capture", "camera-shutter"] {
        for ext in ["oga", "ogg"] {
            let p = format!("/usr/share/sounds/freedesktop/stereo/{n}.{ext}");
            if std::path::Path::new(&p).exists() {
                aqua_sys::audio::play_sound(n);
                return;
            }
        }
    }
}

pub fn play_alert() {
    aqua_sys::audio::play_sound("bell");
}
