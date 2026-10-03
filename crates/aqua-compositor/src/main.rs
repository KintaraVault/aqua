//! Aqua: a Wayland compositor and desktop shell.
//!
//! Usage: `aqua [--winit|--tty] [-c CMD]... [--size WxH] [--scale S] [--screenshot PATH --after SECS]`
//!        `aqua msg <command>`
//!        `aqua check-config [FILE]`, `aqua config-schema`
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
        Some("config-schema") => {
            println!("{}", aqua_config::schema::json_schema());
            return Ok(());
        }
        _ => {}
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,smithay=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    backend::run(cli::parse())
}
