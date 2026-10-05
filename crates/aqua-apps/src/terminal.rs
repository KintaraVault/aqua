//! Terminal emulator discovery shared by the compositor (⌃⌥T, Dock, menus), Finder
//! ("Open in Terminal") and System Settings (commands run in a terminal).
//!
//! The configured `terminal` list (`a|b|c`) is tried first; when none of its programs is
//! installed (an old config written before the user's terminal was known, e.g. only GNOME
//! Console / kgx installed) a built-in list of common emulators, `xdg-terminal-exec`,
//! Debian's `x-terminal-emulator` and finally any desktop entry in the `TerminalEmulator`
//! category are used. Opening a terminal must never silently do nothing.
use std::path::Path;

use crate::{find_in_path, shell_quote};

/// Emulators probed after the configured list, in order of preference.
pub const KNOWN_TERMINALS: &[&str] = &[
    "foot",
    "kitty",
    "alacritty",
    "ghostty",
    "wezterm",
    "ptyxis",
    "kgx",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "mate-terminal",
    "lxterminal",
    "qterminal",
    "cosmic-term",
    "terminology",
    "deepin-terminal",
    "rio",
    "xterm",
    "urxvt",
    "st",
    "weston-terminal",
    "xdg-terminal-exec",
    "x-terminal-emulator",
];

/// The terminal to start: an entry of `spec` (with its arguments) or a fallback.
pub fn resolve_terminal(spec: &str) -> Option<String> {
    for alt in spec.split('|').map(str::trim).filter(|a| !a.is_empty()) {
        let bin = alt.split_whitespace().next().unwrap_or(alt);
        if let Some(p) = find_in_path(bin) {
            return Some(if bin.contains('/') { alt.to_string() } else { alt.replacen(bin, &p.to_string_lossy(), 1) });
        }
    }
    for t in KNOWN_TERMINALS {
        if let Some(p) = find_in_path(t) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    desktop_terminal()
}

/// The first installed desktop entry of a terminal emulator (e.g. a Flatpak).
fn desktop_terminal() -> Option<String> {
    let apps = crate::scan();
    apps.iter()
        .filter(|a| a.categories.iter().any(|c| c == "TerminalEmulator"))
        .map(|a| a.command())
        .find(|c| !c.trim().is_empty())
}

/// File name of the program of a terminal command (`/usr/bin/kgx --foo` → `kgx`).
pub fn terminal_name(cmd: &str) -> &str {
    let bin = cmd.split_whitespace().next().unwrap_or("");
    bin.rsplit('/').next().unwrap_or(bin)
}

/// Shell command that opens `term` (a [`resolve_terminal`] result) with `dir` as its
/// working directory. Single-instance emulators (GNOME Console, GNOME Terminal, Ptyxis,
/// Konsole …) ignore the working directory of the launching process, so their own flag is
/// passed as well.
pub fn terminal_in_dir(term: &str, dir: &Path) -> String {
    let q = shell_quote(&dir.to_string_lossy());
    let flag = match terminal_name(term) {
        "gnome-terminal" | "kgx" | "xfce4-terminal" | "tilix" | "terminator" | "mate-terminal" | "lxterminal"
        | "foot" | "alacritty" => format!(" --working-directory={q}"),
        "ptyxis" => format!(" --new-window --working-directory={q}"),
        "ghostty" => format!(" --working-directory={q}"),
        "konsole" => format!(" --workdir {q}"),
        "kitty" => format!(" --directory {q}"),
        "wezterm" => format!(" start --cwd {q}"),
        "qterminal" => format!(" -w {q}"),
        "cosmic-term" | "rio" => format!(" --working-dir {q}"),
        _ => String::new(),
    };
    if flag.is_empty() {
        format!("cd {q} && exec {term}")
    } else {
        format!("{term}{flag}")
    }
}

/// Arguments placed between the terminal and a command it should run.
pub fn exec_separator(term: &str) -> &'static [&'static str] {
    match terminal_name(term) {
        "gnome-terminal" | "kgx" | "ptyxis" | "kitty" | "foot" => &["--"],
        "xdg-terminal-exec" => &[],
        "wezterm" => &["start", "--"],
        "xfce4-terminal" | "terminator" | "mate-terminal" => &["-x"],
        _ => &["-e"],
    }
}

/// Shell command that runs `cmd` (a shell command line) inside `term`.
pub fn terminal_run(term: &str, cmd: &str) -> String {
    let mut out = term.to_string();
    for a in exec_separator(term) {
        out.push(' ');
        out.push_str(a);
    }
    format!("{out} sh -c {}", shell_quote(cmd))
}

/// Open the user's terminal in `dir`. False when no terminal emulator is installed.
pub fn open_terminal_in(spec: &str, dir: &Path) -> bool {
    match resolve_terminal(spec) {
        Some(t) => crate::launch(&terminal_in_dir(&t, dir)),
        None => {
            tracing::warn!("no terminal emulator found (config `terminal` = {spec:?})");
            false
        }
    }
}

/// Open the user's terminal in the home folder.
pub fn open_terminal(spec: &str) -> bool {
    let home = dirs::home_dir().unwrap_or_else(|| "/".into());
    open_terminal_in(spec, &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_directory_flags() {
        let d = Path::new("/tmp/a b");
        assert_eq!(terminal_in_dir("/usr/bin/kgx", d), "/usr/bin/kgx --working-directory='/tmp/a b'");
        assert_eq!(terminal_in_dir("konsole", d), "konsole --workdir '/tmp/a b'");
        assert_eq!(terminal_in_dir("xterm", d), "cd '/tmp/a b' && exec xterm");
    }

    #[test]
    fn run_commands() {
        assert_eq!(terminal_run("/usr/bin/kgx", "passwd"), "/usr/bin/kgx -- sh -c passwd");
        assert_eq!(exec_separator("xfce4-terminal"), &["-x"]);
        assert_eq!(exec_separator("xterm"), &["-e"]);
    }

    #[test]
    fn configured_entry_keeps_arguments() {
        assert_eq!(resolve_terminal("/bin/sh -l").as_deref(), Some("/bin/sh -l"));
    }
}
