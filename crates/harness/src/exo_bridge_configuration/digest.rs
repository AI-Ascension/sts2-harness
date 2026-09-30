// SPDX-License-Identifier: MIT

//! SHA-256 of the running bridge executable, computed at most once per successful attempt.
//!
//! Every advertised profile embeds this digest, and each profile description is derived from the
//! one-shot one, so a caller inspecting several profiles re-read and re-hashed the same unchanged
//! file each time. Under `cargo test` the executable *is* the test binary, and a debug test binary
//! of this crate is ~182 MiB, so each derivation cost a full read plus SHA-256 of that file. #782
//! reported the result as a hang and set no bound; the case passes, and the defect is the repeated
//! large read, which is what this cache removes.
//!
//! Caching succeeds per process and failures not at all — see `OnceDigest` for why that
//! distinction needs more than a `OnceLock` to get right.

use crate::sha256_hex;
use std::path::Path;
use std::sync::{Condvar, Mutex, PoisonError};

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

/// A cache that pins its first success and lets every failure be retried.
///
/// #787: caching the whole `Result` reads as a harmless simplification, and it is not. A digest is
/// a property of the process — a running executable cannot be replaced and still be executed — but
/// a read error is not a property of anything: the file can be briefly unreadable while it is
/// being replaced, or the path momentarily unresolvable. `OnceLock` keeps whatever it is first
/// given, so caching `Err` makes one transient observation permanent: every later call in that
/// process replays it, and the process is denied for its lifetime with no recovery and nothing in
/// the returned value to distinguish "still broken" from "broke once and was never retried". That
/// turns a momentary environmental fault into a deterministic one.
///
/// The obvious repair — cache only the `Ok` — trades that defect for a worse one, and it is worth
/// being explicit about why, because the naive version looks correct. #784's own cache is a
/// `get`-then-`set` over a `OnceLock`, and it has the mirror-image flaw: `OnceLock::set` reports
/// failure to a *second* caller, so two threads racing the first call both read and hashed the
/// file. Remove the `Result` from the cell and the race comes back in a new place: with nothing to
/// block on, every thread on the failing path runs the initialiser at once. Measured on the
/// straightforward success-only implementation, eight concurrent callers all hashed the file — a
/// burst of parallel advertisements reproducing exactly the redundant work #784 was merged to
/// remove, now on the path that is supposed to be retryable. A shared mutex alone would serialise
/// those callers without deduplicating them.
///
/// So the two guarantees are separated rather than traded. One guarded flag decides *whether* an
/// initialiser runs and a shared slot carries the outcome. If an attempt fails, the flag is cleared
/// without pinning anything, so the next caller takes over; callers arriving while an attempt is in
/// flight are parked on the slot rather than duplicating it. Claiming the attempt and announcing it
/// happen under the same lock, because a gap between the two would let a caller see an attempt that
/// nobody is making and park forever.
pub(super) struct OnceDigest {
    slot: Condvar,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    computing: bool,
    digest: Option<String>,
}

impl OnceDigest {
    pub(super) const fn new() -> Self {
        Self {
            slot: Condvar::new(),
            state: Mutex::new(State {
                computing: false,
                digest: None,
            }),
        }
    }

    pub(super) fn get<F>(&self, initialise: F) -> Result<String, &'static str>
    where
        F: FnOnce() -> Result<String, &'static str>,
    {
        // A poisoned lock means some other caller panicked mid-attempt. Whatever was written is
        // still there, so recovering the inner value is safe and strictly better than propagating
        // a panic into an advertisement path.
        loop {
            // Claiming the attempt and announcing it happen under the same lock. Setting the flag
            // after releasing it would leave a window in which the next caller sees an attempt
            // that nobody is making, and parks forever on a condvar that is never signalled.
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(digest) = state.digest.clone() {
                return Ok(digest);
            }
            if state.computing {
                state = self
                    .slot
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            state.computing = true;
            drop(state);

            let computed = initialise();

            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.computing = false;
            let result = match computed {
                Ok(digest) => {
                    state.digest = Some(digest.clone());
                    Ok(digest)
                }
                // A failure is an observation, not a verdict: clearing `computing` without pinning
                // anything hands the attempt to the next caller, and wakes anyone parked so they
                // stop waiting for a digest that will never arrive.
                Err(error) => Err(error),
            };
            drop(state);
            self.slot.notify_all();
            return result;
        }
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
