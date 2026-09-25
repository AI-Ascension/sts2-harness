// SPDX-License-Identifier: MIT

//! Process-tree observation and argv-leak refusal for the `#148` bound/budget oracle.
//!
//! Split out of `support/bounds.rs` so each helper file stays inside the package's preferred line
//! budget without an exemption. `descendants` is sampled while a drive runs, so a case can report
//! how many real Exo processes a bridge or executor drive stood up; `assert_no_private_argv`
//! refuses a drive that leaked a declared private value into its own command line.

use std::path::PathBuf;

use super::Result;

/// Number of live processes in the tree rooted at `pid`, bounded to refuse a runaway walk.
pub fn descendants(pid: u32) -> Result<usize> {
    let mut pending = vec![pid];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current) {
            continue;
        }
        assert!(visited.len() <= 32, "process tree bound");
        let process = PathBuf::from(format!("/proc/{current}"));
        if let Ok(tasks) = std::fs::read_dir(process.join("task")) {
            for task in tasks.flatten() {
                if let Ok(children) = std::fs::read_to_string(task.path().join("children")) {
                    pending.extend(
                        children
                            .split_whitespace()
                            .filter_map(|id: &str| id.parse::<u32>().ok()),
                    );
                }
            }
        }
    }
    Ok(visited.len())
}

/// Refuses a drive that leaked a private value into the process's own argv.
pub fn assert_no_private_argv(pid: u32) -> Result {
    let command = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    for forbidden in [
        "host-request-private-sentinel",
        "host-turn-private-sentinel",
        "sts2-synthetic-model-key",
    ] {
        assert!(
            !command
                .windows(forbidden.len())
                .any(|part| part == forbidden.as_bytes()),
            "private value reached argv: {forbidden}"
        );
    }
    Ok(())
}
