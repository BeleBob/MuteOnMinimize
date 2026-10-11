//! muteonminimized: mutes the audio of processes whose windows are all
//! minimized. The KWin script tells it which processes those are over D-Bus.

mod audio;
mod dbus;
mod proctree;

use std::process::ExitCode;

fn main() -> ExitCode {
    pipewire::init();
    let (tx, rx) = pipewire::channel::channel();
    match audio::run(rx, || dbus::serve(tx)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("muteonminimized: {err}");
            ExitCode::FAILURE
        }
    }
}
