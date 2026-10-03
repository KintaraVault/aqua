//! Finder-style open/save panel, used by Aqua's xdg-desktop-portal FileChooser backend and
//! usable from scripts:
//!   aqua-filechooser [--title T] [--accept-label L] [--save] [--multiple] [--directory]
//!                    [--name FILE] [--folder DIR] [--filter "Images:*.png;*.jpg"]...
//! Prints the chosen absolute paths (one per line) and exits 0; exits 1 when cancelled.
fn main() -> Result<(), slint::PlatformError> {
    aqua_ui::init("org.aqua.filechooser");
    let (chooser, folder, hidden) = aqua_ui::finder::Chooser::from_args(std::env::args().skip(1));
    let start = folder.filter(|p| p.is_dir()).map(|p| p.to_string_lossy().into_owned());
    aqua_ui::finder::run(Some(chooser), start, hidden)
}
