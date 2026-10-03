//! Finder: Aqua's file manager.
//!   aqua-finder [PATH | recents: | apps: | trash: | tag:NAME]
fn main() -> Result<(), slint::PlatformError> {
    aqua_ui::init("org.aqua.finder");
    let start = std::env::args().nth(1).filter(|a| !a.starts_with("--"));
    aqua_ui::finder::run(None, start, false)
}
