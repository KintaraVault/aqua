//! Aqua: a Wayland compositor and desktop shell.
//!
//! Usage: `aqua [--winit|--tty] [-c CMD]... [--size WxH] [--scale S] [--screenshot PATH --after SECS]`
//!        `aqua msg <command>`
//!        `aqua check-config [FILE]`, `aqua config-schema`
//!        `aqua logs [PROGRAM]`, `aqua crash-report`
mod backend;
mod capture;
mod cli;
mod control;
mod input;
mod ipc;
mod portal;
mod render;
mod selection;
mod session;
mod state;
mod system;
mod wayland;
mod wm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("msg") => std::process::exit(ipc::send(&argv[2..])),
        Some("check-config") => std::process::exit(cli::check_config(argv.get(2).map(String::as_str))),
        Some("logs") => std::process::exit(cli::logs(argv.get(2).map(String::as_str))),
        Some("crash-report") => std::process::exit(cli::crash_report()),
        Some("config-schema") => {
            println!("{}", aqua_config::schema::json_schema());
            return Ok(());
        }
        _ => {}
    }
    aqua_log::init("compositor");
    backend::run(cli::parse())
}
