#[cfg(target_os = "linux")]
mod gl;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod picture;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod protocol;
#[cfg(target_os = "linux")]
mod wayland;

fn main() {
    #[cfg(target_os = "linux")]
    let result = wayland::run();
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), String> = Err("it draws on Wayland, so it needs Linux".into());
    if let Err(err) = result {
        eprintln!("apolline-wall: {err}");
        std::process::exit(1);
    }
}
