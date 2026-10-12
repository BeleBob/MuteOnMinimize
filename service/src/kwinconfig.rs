//! Writes the KWin script's settings, which KWin scripts cannot do themselves.

use std::process::Command;

const GROUP: &str = "Script-muteonminimize";

/// Stores the comma-separated list of applications to mute in kwinrc, where
/// the script and its settings page read it.
pub fn set_applications(value: &str) -> Result<(), String> {
    let status = Command::new("kwriteconfig6")
        .args([
            "--file",
            "kwinrc",
            "--group",
            GROUP,
            "--key",
            "Applications",
            "--",
        ])
        .arg(value)
        .status()
        .map_err(|err| format!("cannot run kwriteconfig6: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kwriteconfig6 failed: {status}"))
    }
}
