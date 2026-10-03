//! Mapping of common Linux applications to their counterparts, so e.g.
//! GNOME Files gets the Finder icon and Firefox the Safari icon. The table lives in
//! `aqua_config::apple_icons` (System Settings lists it); whether it applies is the
//! user's [`Policy`](aqua_config::apple_icons::Policy).

pub use aqua_config::apple_icons::{canon, AppleIcon as Equiv, Policy};

/// Branded icon for the app, ignoring the user's policy.
pub fn lookup(id: &str, icon: &str) -> Option<&'static Equiv> {
    aqua_config::apple_icons::lookup(id, icon).map(|c| &c.icon)
}
