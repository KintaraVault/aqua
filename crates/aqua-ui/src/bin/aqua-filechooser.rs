//! Finder-style open/save panel, used by Aqua's xdg-desktop-portal FileChooser backend and
//! usable from scripts:
//!   aqua-filechooser [--title T] [--accept-label L] [--save] [--multiple] [--directory]
//!                    [--name FILE] [--folder DIR] [--filter "Images:*.png;*.jpg"]...
//! Prints the chosen absolute paths (one per line) and exits 0; exits 1 when cancelled.
//!
//! Started as `zenity` or `kdialog` (links Aqua puts first in `PATH`) it answers their file
//! dialog options (`zenity --file-selection`, `kdialog --getopenfilename` …) in their output
//! format and runs the real program for everything else.
use aqua_ui::finder::shim;

fn main() -> Result<(), slint::PlatformError> {
    let argv: Vec<String> = std::env::args().collect();
    let prog = argv
        .first()
        .and_then(|a| std::path::Path::new(a).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rest = &argv[1.min(argv.len())..];
    let args: Vec<String> = match prog.as_str() {
        "zenity" | "kdialog" => {
            let compat = if prog == "zenity" { shim::zenity(rest) } else { shim::kdialog(rest) };
            match compat {
                Some(c) => {
                    let _ = aqua_ui::finder::OUTPUT_SEP.set(c.sep);
                    c.args
                }
                None => {
                    use std::os::unix::process::CommandExt;
                    let Some(real) = shim::real_program(&prog) else {
                        eprintln!("{prog}: not installed (Aqua only provides its file dialogs)");
                        std::process::exit(127);
                    };
                    let e = std::process::Command::new(real).arg0(&prog).args(rest).exec();
                    eprintln!("{prog}: {e}");
                    std::process::exit(127);
                }
            }
        }
        _ => rest.to_vec(),
    };
    aqua_ui::init("org.aqua.filechooser");
    let (chooser, folder, hidden) = aqua_ui::finder::Chooser::from_args(args.into_iter());
    let start = folder.filter(|p| p.is_dir()).map(|p| p.to_string_lossy().into_owned());
    aqua_ui::finder::run(Some(chooser), start, hidden)
}
