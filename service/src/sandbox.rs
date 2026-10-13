//! Maps PIDs from Flatpak sandboxes to host PIDs.
//!
//! A sandboxed app reports its PID inside its own PID namespace (often 2),
//! which means nothing on the host, where KWin sees its windows. PipeWire tags
//! such streams with the Flatpak instance id; Flatpak records the instance's
//! PID namespace in `$XDG_RUNTIME_DIR/.flatpak/<instance>/bwrapinfo.json`, and
//! the `NSpid` line of `/proc/<pid>/status` lists a process's PID in each
//! nested namespace, innermost last.

use std::env;
use std::fs;
use std::path::Path;

/// The host PID of the process that has `sandbox_pid` inside Flatpak
/// instance `instance_id`, if it still exists.
pub fn host_pid(instance_id: &str, sandbox_pid: u32) -> Option<u32> {
    // The id becomes part of a path.
    if instance_id.is_empty() || !instance_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")?;
    let info = Path::new(&runtime_dir)
        .join(".flatpak")
        .join(instance_id)
        .join("bwrapinfo.json");
    let namespace = parse_pid_namespace(&fs::read_to_string(info).ok()?)?;
    let link = format!("pid:[{namespace}]");

    fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let in_namespace = fs::read_link(entry.path().join("ns/pid"))
            .is_ok_and(|target| target.as_os_str() == link.as_str());
        if !in_namespace {
            return None;
        }
        let status = fs::read_to_string(entry.path().join("status")).ok()?;
        (parse_inner_pid(&status)? == sandbox_pid).then_some(pid)
    })
}

/// Reads `"pid-namespace": <inode>` from bwrapinfo.json without a JSON
/// parser; the file is a flat object of numbers.
fn parse_pid_namespace(info: &str) -> Option<u64> {
    let rest = &info[info.find("\"pid-namespace\"")? + "\"pid-namespace\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The innermost PID from the `NSpid` line of /proc/<pid>/status.
fn parse_inner_pid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))?
        .split_whitespace()
        .last()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bwrapinfo() {
        let info = "{\n    \"child-pid\": 11675,\n    \"mnt-namespace\": 4026533577,\n    \"pid-namespace\": 4026533578\n}\n";
        assert_eq!(parse_pid_namespace(info), Some(4026533578));
        assert_eq!(parse_pid_namespace("{\"pid-namespace\":7}"), Some(7));
        assert_eq!(parse_pid_namespace("{\"child-pid\": 1}"), None);
    }

    #[test]
    fn reads_nspid() {
        let status =
            "Name:\tgst-launch-1.0\nTgid:\t12162\nPid:\t12162\nNSpid:\t12162\t2\nPPid:\t12161\n";
        assert_eq!(parse_inner_pid(status), Some(2));
        assert_eq!(parse_inner_pid("Pid:\t5\nNSpid:\t5\n"), Some(5));
        assert_eq!(parse_inner_pid("Pid:\t5\n"), None);
    }

    #[test]
    fn rejects_odd_instance_ids() {
        assert_eq!(host_pid("../../etc", 2), None);
        assert_eq!(host_pid("", 2), None);
    }
}
