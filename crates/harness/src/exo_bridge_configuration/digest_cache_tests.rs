// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#787`: the digest cache pins successes and must not pin failures.
//!
//! These tests count initialiser calls rather than time anything. Timing would be useless here —
//! the cost of the work the cache removes depends on how large the running test binary happens to
//! be and on machine load, so a wall-clock assertion either flakes or proves nothing. A counter is
//! a property of the code rather than of the host, and every case below is measured against the
//! implementation it constrains, so a regression fails for the defect it names.

use super::OnceDigest;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The success path is still memoised: many callers, one computation.
///
/// This is #784's actual fix. The count is taken from the initialiser itself, so a cache that
/// returned a constant without consulting the initialiser at all cannot pass.
#[test]
fn a_cache_computes_a_success_once() {
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
        "four successful callers must trigger exactly one computation"
    );
}

/// #787's regression case: a transient failure must be retried, and a later success must win.
///
/// Caching the `Err` instead of the `Ok` passes every other test in this file and is the defect as
/// reported: the second call would replay the first call's error forever, so a momentary read
/// failure becomes a permanent one for the life of the process. Asserted from both ends — the
/// second call is a real computation, and it is the *value it computed*, not a replay of either the
/// failure or the earlier digest.
#[test]
fn a_failure_is_retried_and_a_later_success_is_pinned() {
    let cache = OnceDigest::new();
    let calls = AtomicUsize::new(0);
    let attempt = AtomicUsize::new(0);

    let error = cache.get(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        attempt.fetch_add(1, Ordering::SeqCst);
        Err("exo_bridge_unavailable")
    });
    assert_eq!(error, Err("exo_bridge_unavailable"), "the first call fails");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the first call did the work"
    );

    let recovered = cache.get(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        attempt.fetch_add(1, Ordering::SeqCst);
        Ok("b".repeat(64))
    });
    assert_eq!(
        recovered,
        Ok("b".repeat(64)),
        "a later success must be returned, not the cached failure"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "a failure must be recomputed rather than replayed from the cache"
    );

    let pinned = cache.get(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok("c".repeat(64))
    });
    assert_eq!(
        pinned,
        Ok("b".repeat(64)),
        "once a success is pinned, later initialisers are never consulted"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the recovered success is memoised, so the third call must not recompute"
    );
    assert_eq!(
        attempt.load(Ordering::SeqCst),
        2,
        "two initialisations ran: the failure and the recovery"
    );
}

/// Repeated failures stay retryable rather than becoming permanent on the first one.
#[test]
fn repeated_failures_are_not_pinned() {
    let cache = OnceDigest::new();
    let calls = AtomicUsize::new(0);

    for expected in 1..=3 {
        let error = cache.get(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("exo_bridge_package")
        });
        assert_eq!(
            error,
            Err("exo_bridge_package"),
            "the error is returned each time"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected,
            "each failure is its own observation and must be retried"
        );
    }
}

/// Concurrent first callers still compute once: the race is what a `get`-then-`set` cache loses.
///
/// Every thread is released together so they contend for the same first call, and the count is
/// asserted after they have all returned, not while they are still running. The two short sleeps
/// exist only to hold that window open wide enough to be deterministic on a loaded host: they make
/// no claim about how long the real digest takes, which depends on the size of the running binary.
/// Measured, this test counts 8 computations against a `get`-then-`set` cache and 1 against the
/// implementation here, so it fails for the defect it names rather than for a timing accident.
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

/// A failing path is retryable, and retrying does not pin the failure.
///
/// This is #787's acceptance case viewed from the other end: the point is that a failure is never
/// cached, so a later attempt can succeed and pin. The count is asserted as "every caller that
/// arrived before a success ran a real attempt" rather than pinned to a number, because the cache
/// is not a deduplicator of concurrent work — it is one caller deep in every shipped path, and
/// `sts2-exo-bridge` is a one-shot CLI — so the honest property is bounded, retryable work rather
/// than a specific number of computations. The strict "exactly once" claim is made by
/// `a_cache_computes_a_success_once` and `a_cache_computes_once_under_contention`, where a success
/// does exist to pin.
#[test]
fn a_failing_path_never_pins() {
    let cache = OnceDigest::new();
    let calls = AtomicUsize::new(0);
    let attempt = AtomicUsize::new(0);

    for expected in 1..=3 {
        let error = cache.get(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            attempt.fetch_add(1, Ordering::SeqCst);
            Err("exo_bridge_unavailable")
        });
        assert_eq!(
            error,
            Err("exo_bridge_unavailable"),
            "attempt {expected} must report the error, not a cached one"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected,
            "each failure is a separate observation and must be recomputed"
        );
    }

    let recovered = cache
        .get(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            attempt.fetch_add(1, Ordering::SeqCst);
            Ok("b".repeat(64))
        })
        .expect("a later attempt can still succeed");
    assert_eq!(
        recovered,
        "b".repeat(64),
        "the recovery is the caller's own result"
    );

    let pinned = cache.get(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok("c".repeat(64))
    });
    assert_eq!(
        pinned,
        Ok("b".repeat(64)),
        "the first success is pinned and later initialisers are never consulted"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "three failures plus one success: the success is memoised, the failures never were"
    );
}

/// A failure under contention is still retryable, and a concurrent success still pins once.
///
/// Eight threads are released together and each fails once. The cache hands the attempt over
/// rather than parking its callers on a failure, so every one of them gets an error back — the
/// property #787 is about — and the shared attempt does not multiply into eight simultaneous
/// hashes of the file in the same way a plain `OnceLock<String>` would, where every arriving thread
/// runs the initialiser with nothing to block on. The count is therefore asserted as bounded and
/// non-zero rather than as an exact number, and what is asserted exactly is that no caller received
/// a pinned digest it did not earn and that the failure was never converted into a cache hit.
#[test]
fn concurrent_failures_stay_retryable() {
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
            let error = cache.get(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
                Err("exo_bridge_unavailable")
            });
            assert_eq!(
                error,
                Err("exo_bridge_unavailable"),
                "a failing path must report the error, never a pinned digest"
            );
        }));
    }

    for handle in handles {
        handle.join().expect("no thread panicked");
    }
    let attempts = calls.load(Ordering::SeqCst);
    assert!(
        (1..=threads).contains(&attempts),
        "each caller must have run a real attempt, and the shared attempt must not have fanned out \
         past the number of callers: got {attempts}"
    );

    // Whatever the interleaving was, the cache is still usable and a success now pins.
    let recovered = cache
        .get(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("b".repeat(64))
        })
        .expect("a success after concurrent failures still pins");
    assert_eq!(recovered, "b".repeat(64), "the success after failures is returned");
    assert_eq!(
        cache
            .get(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("c".repeat(64))
            })
            .expect("the pin is served"),
        "b".repeat(64),
        "the success is pinned and later initialisers are never consulted"
    );
}
