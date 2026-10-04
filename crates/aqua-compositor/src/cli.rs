//! Command line.

pub struct Args {
    pub tty: bool,
    pub commands: Vec<String>,
    pub size: (i32, i32),
    pub scale: f64,
    pub screenshot: Option<(String, f64)>,
    pub quit_after: Option<f64>,
}

pub const USAGE: &str =
    "aqua [--winit|--tty] [-c CMD]... [--size WxH] [--scale S] [--screenshot PATH [--after SECS]] [--quit-after SECS]";

/// `aqua check-config [FILE]`: print every problem of a config file; exit code 1 if any.
pub fn check_config(path: Option<&str>) -> i32 {
    let p = path.map(std::path::PathBuf::from).unwrap_or_else(aqua_config::Config::file);
    let src = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", p.display());
            return 2;
        }
    };
    match aqua_config::Config::from_toml_lenient(&src) {
        Ok((_, issues)) if issues.is_empty() => {
            println!("{}: OK", p.display());
            0
        }
        Ok((_, issues)) => {
            for i in issues {
                println!("{}: {i}", p.display());
            }
            1
        }
        Err(e) => {
            println!("{}: {e}", p.display());
            1
        }
    }
}

/// Parse the process arguments; without `--tty`/`--winit` the backend is chosen by whether
/// a parent display server is running.
/// `aqua logs [PROGRAM]`: where the logs are, and the tail of one of them
/// (`compositor` by default; e.g. `org.aqua.finder`, `firefox`).
pub fn logs(which: Option<&str>) -> i32 {
    let dir = aqua_log::log_dir();
    let name = which.unwrap_or("compositor");
    let path = if name.ends_with(".log") { dir.join(name) } else { aqua_log::log_path(name) };
    println!("logs: {}", dir.display());
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut v: Vec<_> = rd.flatten().filter(|e| e.path().is_file()).collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
        for e in v.iter().take(12) {
            println!("  {}", e.file_name().to_string_lossy());
        }
    }
    match std::fs::read_to_string(&path) {
        Ok(t) => {
            let lines: Vec<&str> = t.lines().collect();
            println!("--- {} (last {} lines)", path.display(), lines.len().min(60));
            for l in &lines[lines.len().saturating_sub(60)..] {
                println!("{l}");
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            1
        }
    }
}

/// `aqua crash-report`: the newest crash report and the crash index.
pub fn crash_report() -> i32 {
    let idx = aqua_log::crash_dir().join("index.log");
    if let Ok(t) = std::fs::read_to_string(&idx) {
        let lines: Vec<&str> = t.lines().collect();
        println!("--- recent crashes ({})", idx.display());
        for l in &lines[lines.len().saturating_sub(10)..] {
            println!("{l}");
        }
    }
    match aqua_log::latest_crash() {
        Some(p) => {
            println!("--- {}", p.display());
            print!("{}", std::fs::read_to_string(&p).unwrap_or_default());
            0
        }
        None => {
            println!("no crash reports in {}", aqua_log::crash_dir().display());
            0
        }
    }
}

pub fn parse() -> Args {
    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    match parse_from(std::env::args().skip(1), !nested) {
        Ok(a) => a,
        Err(Help) => {
            println!("{USAGE}");
            std::process::exit(0);
        }
    }
}

/// `--help` was requested.
#[derive(Debug, PartialEq)]
pub struct Help;

pub fn parse_from(args: impl IntoIterator<Item = String>, tty: bool) -> Result<Args, Help> {
    let mut a = Args { tty, commands: vec![], size: (1440, 900), scale: 1.0, screenshot: None, quit_after: None };
    let mut it = args.into_iter();
    let mut shot_path = None;
    let mut after = 3.0;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tty" | "--udev" => a.tty = true,
            "--winit" => a.tty = false,
            "-c" | "--command" => a.commands.extend(it.next()),
            "--size" => {
                if let Some(size) = it.next().as_deref().and_then(parse_size) {
                    a.size = size;
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--screenshot" => shot_path = it.next(),
            "--after" => after = it.next().and_then(|s| s.parse().ok()).unwrap_or(3.0),
            "--quit-after" => a.quit_after = it.next().and_then(|s| s.parse().ok()),
            "-h" | "--help" => return Err(Help),
            _ => eprintln!("unknown argument {arg}"),
        }
    }
    if let Some(p) = shot_path {
        a.screenshot = Some((p, after));
    }
    Ok(a)
}

/// "1280x800" → (1280, 800); both sides must be positive.
fn parse_size(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args, Help> {
        parse_from(s.split_whitespace().map(String::from), false)
    }

    #[test]
    fn defaults() {
        let a = args("").unwrap();
        assert!(!a.tty);
        assert!(a.commands.is_empty());
        assert_eq!(a.size, (1440, 900));
        assert_eq!(a.scale, 1.0);
        assert!(a.screenshot.is_none() && a.quit_after.is_none());
        assert!(parse_from(Vec::<String>::new(), true).unwrap().tty);
    }

    #[test]
    fn all_options() {
        let a = args("--tty -c foot --command firefox --size 1280x800 --scale 2 --screenshot /tmp/s.png --after 1.5 --quit-after 9")
            .unwrap();
        assert!(a.tty);
        assert_eq!(a.commands, vec!["foot", "firefox"]);
        assert_eq!(a.size, (1280, 800));
        assert_eq!(a.scale, 2.0);
        assert_eq!(a.screenshot, Some(("/tmp/s.png".to_string(), 1.5)));
        assert_eq!(a.quit_after, Some(9.0));
        assert!(!args("--tty --winit").unwrap().tty, "the last backend flag wins");
    }

    #[test]
    fn bad_values_keep_defaults() {
        let a = args("--size 0x10 --scale nope --bogus").unwrap();
        assert_eq!(a.size, (1440, 900));
        assert_eq!(a.scale, 1.0);
        assert_eq!(args("--size 800X600").unwrap().size, (800, 600));
        assert_eq!(args("--screenshot x.png").unwrap().screenshot, Some(("x.png".to_string(), 3.0)));
    }

    #[test]
    fn help() {
        assert_eq!(args("--help").err(), Some(Help));
        assert_eq!(args("-c foot -h").err(), Some(Help));
    }
}
