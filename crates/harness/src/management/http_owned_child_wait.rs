// SPDX-License-Identifier: MIT

//! Refuse inherited auto-reaping before the owned transport creates any children.

use std::fs::File;
use std::io::Read;

use rustix::process::Signal;

use super::HttpError;

const MAX_STATUS_BYTES: usize = 65_536;
const CHILD_SIGNAL_BIT: u64 = 1_u64 << (Signal::CHILD.as_raw() - 1);

pub(super) fn require_waitable_children() -> Result<(), HttpError> {
    let mut status = String::new();
    File::open("/proc/self/status")
        .and_then(|file| {
            file.take(MAX_STATUS_BYTES as u64 + 1)
                .read_to_string(&mut status)
        })
        .map_err(|_| inspection_error())?;
    inspect_status(&status)
}

fn inspect_status(status: &str) -> Result<(), HttpError> {
    if status.len() > MAX_STATUS_BYTES {
        return Err(inspection_error());
    }
    let mut ignored = status
        .lines()
        .filter_map(|line| line.strip_prefix("SigIgn:"));
    let mask = ignored.next().ok_or_else(inspection_error)?.trim();
    if ignored.next().is_some()
        || mask.is_empty()
        || mask.len() > 16
        || !mask.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(inspection_error());
    }
    let mask = u64::from_str_radix(mask, 16).map_err(|_| inspection_error())?;
    if mask & CHILD_SIGNAL_BIT != 0 {
        // SIG_IGN survives exec and makes Linux discard child exit statuses. Retaining a
        // Child cannot restore those statuses; its ECHILD error is not an observed exit.
        return Err(HttpError::new(
            "owned_management_child_wait",
            "owned management cannot start while SIGCHLD is ignored; restore default child waiting before launch",
        ));
    }
    Ok(())
}

fn inspection_error() -> HttpError {
    HttpError::new(
        "owned_management_child_wait",
        "cannot inspect inherited SIGCHLD disposition for owned management",
    )
}

#[cfg(test)]
#[path = "http_owned_child_wait_tests.rs"]
mod tests;
