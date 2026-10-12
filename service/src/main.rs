//! muteonminimized: mutes the audio of processes whose windows are all
//! minimized. The KWin script tells it which processes those are over D-Bus.

mod audio;
mod dbus;
mod i18n;
mod kwinconfig;
mod proctree;

use std::process::ExitCode;

fn main() -> ExitCode {
    pipewire::init();
    let (tx, rx) = pipewire::channel::channel();
    // Claim the bus name before connecting to PipeWire: the KWin script gets
    // its translations right away, and calls made meanwhile are queued.
    let _connection = match dbus::serve(tx) {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("muteonminimized: {err}");
            return ExitCode::FAILURE;
        }
    };
    match audio::run(rx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("muteonminimized: {err}");
            ExitCode::FAILURE
        }
    }
}
