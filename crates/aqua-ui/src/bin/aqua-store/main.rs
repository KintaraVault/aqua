//! App Store: browse, install, update and remove apps from Flathub and the system's package
//! manager (pacman + AUR, dnf, apt).
//!   aqua-store [--updates | --account | --search Q | --app KEY | --category ID | appstream://ID | FILE.flatpakref]
//!   aqua-store --check-updates    background update checks and notifications
mod actions;
mod app;
mod conv;
mod detail;
mod images;
mod notify;
mod run;
#[cfg(test)]
mod tests;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check-updates") {
        notify::background();
        return Ok(());
    }
    aqua_ui::init("org.aqua.store");
    run::run(conv::parse_start(&args))
}
