use apolline_wall_protocol::protocol;

mod cli;
#[cfg(target_os = "linux")]
mod engine;
#[cfg(target_os = "linux")]
mod gl;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod picture;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod state;
#[cfg(target_os = "linux")]
mod video;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => serve(false),
        Some("daemon") if args.len() == 1 => serve(true),
        Some(_) => cli::run(&args),
    };
    if let Err(err) = result {
        eprintln!("apolline-wall: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn serve(daemon: bool) -> Result<(), String> {
    engine::run(if daemon {
        engine::Mode::Daemon
    } else {
        engine::Mode::Child
    })
}

#[cfg(not(target_os = "linux"))]
fn serve(_: bool) -> Result<(), String> {
    Err("it draws on Wayland or X11, so it needs Linux".into())
}
