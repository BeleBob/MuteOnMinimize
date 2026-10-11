//! Process tree lookups through /proc.

use std::collections::HashSet;
use std::fs;

/// Guards against cycles or absurdly deep trees.
const MAX_DEPTH: usize = 64;

/// Parent PID of `pid`, or `None` if the process is gone or unreadable.
pub fn parent_pid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_ppid(&stat)
}

/// Extracts the parent PID from the contents of /proc/<pid>/stat.
///
/// The command name in field 2 may contain spaces and parentheses, so fields
/// are counted from the last ')'.
fn parse_ppid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // After the name: state, ppid, ...
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Whether `pid` is one of `roots` or a descendant of one.
pub fn in_trees(pid: u32, roots: &HashSet<u32>) -> bool {
    in_trees_with(pid, roots, parent_pid)
}

fn in_trees_with(pid: u32, roots: &HashSet<u32>, parent: impl Fn(u32) -> Option<u32>) -> bool {
    if roots.is_empty() {
        return false;
    }
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if roots.contains(&current) {
            return true;
        }
        match parent(current) {
            Some(p) if p > 1 && p != current => current = p,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn parses_ppid_with_tricky_comm() {
        let stat = "99802 (Discord) S 99539 99539 99539 0 -1 4194560";
        assert_eq!(parse_ppid(stat), Some(99539));
        let stat = "42 (a) b (c)) R 7 42 42 0";
        assert_eq!(parse_ppid(stat), Some(7));
    }

    #[test]
    fn walks_up_to_root() {
        // 1 <- 990 <- 99539 (window) <- 99802 (audio service)
        let parents: HashMap<u32, u32> = [(990, 1), (99539, 990), (99802, 99539)].into();
        let parent = |p: u32| parents.get(&p).copied();
        let roots: HashSet<u32> = [99539].into();

        assert!(in_trees_with(99539, &roots, parent));
        assert!(in_trees_with(99802, &roots, parent));
        assert!(!in_trees_with(990, &roots, parent));
        assert!(!in_trees_with(12345, &roots, parent));
        assert!(!in_trees_with(99802, &HashSet::new(), parent));
    }

    #[test]
    fn current_process_is_in_own_tree() {
        let me = std::process::id();
        assert!(in_trees(me, &[me].into()));
        let parent = parent_pid(me).unwrap();
        assert!(in_trees(me, &[parent].into()));
    }
}
