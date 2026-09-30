// SPDX-License-Identifier: MIT

//! The running executable's SHA-256, memoised per process but only once it has actually succeeded.
//!
//! Every advertised profile embeds this digest, and each profile description is derived from the
//! one-shot one, so a caller inspecting several profiles re-read and re-hashed the same unchanged
//! file each time. Under `cargo test` the executable *is* the test binary, and a debug test binary
//! of this crate runs to a few hundred megabytes, so a second hash of it is a cost worth paying
//! exactly once per process — and no more.
//!
//! "Once it has actually succeeded" is the whole of #787. [`OnceDigest`] pins a success and hands
//! a failure back to the next caller, so a transient read error costs one attempt instead of the
//! process.

use crate::sha256_hex;
use std::sync::{Condvar, Mutex, PoisonError};

use super::{MAX_EXECUTOR_BYTES, read_bounded};

/// SHA-256 of the running bridge executable, computed at most once per successful attempt.
///
/// The running executable cannot be replaced and still be executed, so this digest is a property
/// of the process rather than of the filesystem: caching a *successful* one for the process
/// lifetime is sound, and nothing is cached across processes, so a rebuilt binary is never
/// reported under a previous build's digest. A failure is not a property of anything — the file
/// can be briefly unreadable while it is being replaced — so it is never pinned.
pub(super) fn bridge_digest() -> Result<String, &'static str> {
    static DIGEST: OnceDigest = OnceDigest::new();
    DIGEST.get(|| {
        let executable = std::env::current_exe().map_err(|_| "exo_bridge_package")?;
        Ok(sha256_hex(read_bounded(&executable, MAX_EXECUTOR_BYTES)?))
    })
}

/// A one-shot cache that pins its first success and retries every failure.
///
/// #787: caching the whole `Result` reads as a harmless simplification, and it is not. A
/// `OnceLock` keeps whatever it is first given, so caching `Err` makes one transient observation
/// permanent: every later call in that process replays it, and nothing in the returned value
/// distinguishes "still broken" from "broke once and was never retried". That turns a momentary
/// environmental fault into a deterministic one, for the lifetime of the process.
///
/// The opposite naive repair — keep the cell as `OnceLock<String>` and simply not store an
/// error — removes the pinned failure and gives the redundancy straight back, because
/// `OnceLock::set` reports failure to a *second* caller rather than waiting for it. Two threads
/// racing the first call both read and hashed the file. So the guarantee is expressed as a
/// one-shot attempt plus a shared slot instead of a single cell: the slot decides *whether* an
/// initialiser runs, and it carries the outcome.
///
/// A caller that arrives while an attempt is in flight waits for it and then reports *that*
/// attempt's outcome, rather than starting a second one. That is what keeps the failing path from
/// fanning out: waking the waiters on a failure without giving them its result would hand the
/// attempt to each of them in turn, and a burst of parallel advertisements would re-hash the file
/// once per caller — the same redundant work #784 removed, now on the one path that is meant to
/// be retryable. The retry belongs to the next call that arrives when nothing is in flight.
pub(super) struct OnceDigest {
    slot: Mutex<State>,
    pinned: Condvar,
}

struct State {
    computing: bool,
    digest: Option<String>,
    /// Attempts that have run to completion. A waiter compares this against the count it
    /// recorded before parking, so it can tell "the attempt I waited on finished" from "an
    /// attempt I never saw finished".
    attempts: u64,
    /// The outcome of the most recent completed attempt, kept only until the next attempt starts
    /// so a woken waiter can read it.
    failure: Option<&'static str>,
}

impl OnceDigest {
    pub(super) const fn new() -> Self {
        Self {
            slot: Mutex::new(State {
                computing: false,
                digest: None,
                attempts: 0,
                failure: None,
            }),
            pinned: Condvar::new(),
        }
    }

    /// Returns the pinned digest, computing it with `initialise` at most once.
    ///
    /// A failed attempt is returned to its caller and leaves nothing pinned, so the next call
    /// runs `initialise` again.
    pub(super) fn get<F>(&self, initialise: F) -> Result<String, &'static str>
    where
        F: FnOnce() -> Result<String, &'static str>,
    {
        // A poisoned lock means some other caller panicked mid-attempt. Whatever was written is
        // still there, so recovering the inner value is safe and strictly better than propagating
        // a panic out of an advertisement path.
        loop {
            // Claiming the attempt and announcing it happen under one lock. Setting the flag
            // after releasing it would leave a window in which the next caller sees an attempt in
            // flight that nobody is performing, and parks forever on a condvar nobody signals.
            let mut state = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(digest) = state.digest.clone() {
                return Ok(digest);
            }
            if state.computing {
                let waited = state.attempts;
                state = self
                    .pinned
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                // The attempt this caller parked behind has now finished. Its result is this
                // caller's result; retrying here instead would duplicate the work it just did.
                if state.attempts > waited {
                    return match (state.digest.clone(), state.failure) {
                        (_, Some(error)) => Err(error),
                        (Some(digest), None) => Ok(digest),
                        (None, None) => continue,
                    };
                }
                continue;
            }
            state.computing = true;
            state.failure = None;
            drop(state);

            let computed = initialise();

            let mut state = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            state.computing = false;
            state.attempts += 1;
            let result = match computed {
                Ok(digest) => {
                    state.digest = Some(digest.clone());
                    Ok(digest)
                }
                // A failure is an observation, not a verdict. Nothing is pinned, so the attempt is
                // available again to the next caller that arrives with nothing in flight, and the
                // wake-up hands this outcome to the callers that parked behind it.
                Err(error) => {
                    state.failure = Some(error);
                    Err(error)
                }
            };
            drop(state);
            self.pinned.notify_all();
            return result;
        }
    }
}

#[cfg(test)]
#[path = "digest_tests.rs"]
mod digest_tests;
