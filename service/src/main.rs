//! muteonminimized: mutes the audio of processes whose windows are all
//! minimized. The KWin script tells it which processes those are over D-Bus.

mod audio;
mod dbus;
mod i18n;
mod kwinconfig;
mod proctree;
mod sandbox;

use std::process::ExitCode;

fn main() -> ExitCode {
    block_termination_signals();
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

/// PipeWire receives SIGTERM and SIGINT through a signalfd, which only sees
/// signals that no thread accepts directly. Block them before any thread is
/// spawned (zbus starts its own), so that every thread inherits the mask and
/// the loop can give back the muted streams before exiting.
fn block_termination_signals() {
    // SAFETY: plain libc calls on a local, zero-initialized signal set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}
