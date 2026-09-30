// SPDX-License-Identifier: MIT

//! `sts2-harness#782`: the running executable's digest is computed at most once per process.
//!
//! These tests count initialiser calls rather than time anything. Timing would be useless here: the
//! cost of the work the cache removes depends on how large the running test binary happens to be
//! and how loaded the machine is, so a wall-clock assertion either flakes or proves nothing. A
//! counter is a property of the code rather than of the host, and it fails against the
//! un-memoised implementation for the same reason the defect was reported -- one derivation per
//! advertised profile -- which is exactly the regression these tests exist to hold shut.

#![allow(clippy::expect_used)]

use super::OnceDigest;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh cache runs its initialiser exactly once, however many callers ask.
///
/// The count is taken from the initialiser itself, so a cache that returned a constant without
/// consulting the initialiser at all cannot pass this.
#[test]
fn a_cache_computes_its_value_once() {
    let cache = OnceDigest::new();
    let calls = AtomicUsize::new(0);

    for _ in 0..4 {
        let digest = cache
            .get(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("a".repeat(64))
            })
            .expect("the initialiser succeeds");
        assert_eq!(
            digest,
            "a".repeat(64),
            "every caller sees the computed digest"
        );
    }

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "four callers must trigger exactly one computation"
    );
}

/// A failure is memoised too, so a missing executable is not re-read on every advertisement.
#[test]
fn a_cache_computes_a_failure_once() {
    let cache = OnceDigest::new();
    let calls = AtomicUsize::new(0);

    for _ in 0..4 {
        let error = cache.get(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("exo_bridge_package")
        });
        assert_eq!(
            error,
            Err("exo_bridge_package"),
            "the first error is replayed"
        );
    }

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a failure is a stable property of the process, so it is computed once"
    );
}

/// Concurrent first callers still compute once: the race is what a `get`-then-`set` cache loses.
///
/// Every thread is released together so they contend for the same first call, and the count is
/// asserted after they have all returned, not while they are still running. The two short sleeps
/// exist only to hold that window open wide enough to be deterministic on a loaded host: they make
/// no claim about how long the real digest takes, which depends on the size of the running binary.
/// Measured, this test counts 8 computations against a `get`-then-`set` cache and 1 against
/// `get_or_init`, so it fails for the defect it names rather than for a timing accident.
#[test]
fn a_cache_computes_once_under_contention() {
    use std::sync::{Arc, Barrier};

    let cache = Arc::new(OnceDigest::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::with_capacity(threads);

    for _ in 0..threads {
        let (cache, calls, barrier) =
            (Arc::clone(&cache), Arc::clone(&calls), Arc::clone(&barrier));
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            std::thread::sleep(std::time::Duration::from_millis(20));
            cache
                .get(|| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    Ok("b".repeat(64))
                })
                .expect("the initialiser succeeds")
        }));
    }

    for handle in handles {
        let digest = handle.join().expect("no thread panicked");
        assert_eq!(
            digest,
            "b".repeat(64),
            "every racing caller sees the same digest"
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "concurrent first callers must still trigger exactly one computation"
    );
}
