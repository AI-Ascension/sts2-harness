// SPDX-License-Identifier: MIT

//! A temporary directory that owns itself.
//!
//! #713 lives here because two modules need it: the `fake_gh` stub in `runner` and the
//! `hanging_gh` stub in `deadline` both leaked one directory per call before this type existed.

use std::error::Error;
use std::path::PathBuf;

/// A per-call temporary directory that removes itself when the value holding it drops.
///
/// #713: the stub helpers created one of these per call and nothing ever removed it, so every
/// run of this suite left one behind in `$TMPDIR` forever -- 2,285 accumulated
/// `sts2-review-gate-test-*` directories were measured in `/tmp` during the #707 investigation.
/// The per-call nonce is what makes that a leak rather than a collision: every call gets a
/// *distinct* name, so the directories accumulate instead of overwriting one another.
///
/// `Drop` is the shape that covers the success and the error path without asking every caller
/// to remember a cleanup step, and it is the simpler one here because the crate denies
/// `unwrap`/`expect`/`panic!` -- a hand-written cleanup tail would have to propagate a second
/// error from every test and would still be skipped by any early return.
///
/// `remove_dir_all` is best-effort because `Drop` cannot report and a failed removal must not
/// turn a passing test into a failing one. The cost of ignoring it is one stale directory,
/// which is exactly what this type exists to prevent, so the swallow is bounded and the
/// property is asserted directly by `fake_gh_leaves_no_directory_behind` rather than assumed.
pub(super) struct TempDir {
    pub(super) path: PathBuf,
}

impl TempDir {
    /// Create a uniquely-named directory under the system temp directory.
    ///
    /// `tag` distinguishes helpers that stub the same thing for different reasons, so a
    /// directory that does survive is attributable without reading the tests. The counter is
    /// process-local and `fetch_add` is atomic, so every call in one process gets a distinct
    /// name and the process id separates concurrent test binaries.
    pub(super) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sts2-review-gate-{tag}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
