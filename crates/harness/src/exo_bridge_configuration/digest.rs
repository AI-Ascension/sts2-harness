// SPDX-License-Identifier: MIT

//! SHA-256 of the running bridge executable, computed at most once per process.
//!
//! Every advertised profile embeds this digest, and each profile description is derived from the
//! one-shot one, so a caller inspecting several profiles re-read and re-hashed the same unchanged
//! file each time. Under `cargo test` the executable *is* the test binary, and a debug test binary
//! of this crate is ~190 MB: one read plus SHA-256 of it costs seconds on an idle machine, so four
//! advertisements cost four of those and made a purely in-memory advertisement test do tens of
//! seconds of CPU-bound work it had no reason to do. The report in #782 described the result as a
//! hang and set no bound, so no wall-clock figure is claimed here; the defect is the repeated large
//! read, which is what this cache removes.

use crate::sha256_hex;
use std::path::Path;
use std::sync::OnceLock;

/// The digest of the running bridge executable.
pub(super) fn bridge_digest() -> Result<String, &'static str> {
    static DIGEST: OnceDigest = OnceDigest::new();
    DIGEST.get(|| {
        let executable = std::env::current_exe().map_err(|_| "exo_bridge_package")?;
        Ok(sha256_hex(read_bounded(
            &executable,
            crate::exo_bridge_configuration::MAX_EXECUTOR_BYTES,
        )?))
    })
}

/// A result cache that runs its initialiser at most once, whatever the concurrency.
///
/// `OnceLock::set` reports failure to a *second* caller, so a hand-rolled `get`-then-`set` cache
/// computes the digest more than once when two threads race — the exact duplicate work this cache
/// exists to remove, reintroduced under the one caller pattern that can actually produce it.
/// `OnceLock::get_or_init` runs the initialiser inside the library's own one-time barrier, so the
/// first call computes and every other call waits for and reuses that one result. Because the
/// closure returns a `Result`, a failure is memoised as well: re-reading the same file to fail
/// identically on every advertisement is the same defect, and the error is stable for the life of
/// the process for the same reason the digest is.
pub(super) struct OnceDigest {
    cell: OnceLock<Result<String, &'static str>>,
}

impl OnceDigest {
    pub(super) const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    pub(super) fn get<F>(&self, initialise: F) -> Result<String, &'static str>
    where
        F: FnOnce() -> Result<String, &'static str>,
    {
        self.cell.get_or_init(initialise).clone()
    }
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, &'static str> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "exo_bridge_unavailable")?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_bridge_unavailable")?;
    if bytes.len() > maximum {
        return Err("exo_bridge_package_bound");
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "digest_cache_tests.rs"]
mod digest_cache_tests;
