#![windows_subsystem = "windows"]

mod assets;
mod audio;
mod config;
mod daemon;
mod devtools;
mod keymap;
mod player;
mod editor;
mod sys;

fn main() {
    // No console in a windows-subsystem app, so panics go to log.txt next to the config.
    std::panic::set_hook(Box::new(|info| sys::log(&format!("panic: {info}"))));
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--settings") => editor::run(),
        Some("--render") if args.len() >= 3 => devtools::render(&args[1], &args[2]),
        Some("--bench") if args.len() >= 2 => devtools::bench(&args[1]),
        _ => {
            // A second launch opens the settings, like the original app showing its editor.
            if sys::single_instance("Local\\animalese-rs-daemon") {
                daemon::run();
            } else {
                sys::spawn_settings();
            }
        }
    }
}
