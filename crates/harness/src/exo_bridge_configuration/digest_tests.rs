// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#787`: the digest cache pins successes and must not pin failures.
//!
//! These tests count initialiser calls rather than time anything. Timing would be useless here:
//! the work the cache removes depends on how large the running test binary happens to be and on
//! machine load, so a wall-clock assertion either flakes or proves nothing. A call count is a
//! property of the code rather than of the host, and every case below is measured against the
//! implementation it constrains, so a regression fails for the defect it names.

use super::OnceDigest;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A transient failure is retried: the next call computes for itself and succeeds.
///
/// This is the regression test for #787. `OnceLock::get_or_init` runs its initialiser once and
/// keeps whatever it produced, so the version that cached the whole `Result` returned the first
/// `Err` from every later call for the life of the process. Counting the second call's work is
/// what separates a retry from a replay: a replay consults the cache and never reaches the
/// initialiser at all, which the count below reports.
#[test]
fn a_failed_attempt_is_not_pinned() {
    let cache = OnceDigest::new();
    let attempts = AtomicUsize::new(0);

    let failed = cache.get(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err("exo_bridge_unavailable")
    });
    assert_eq!(
        failed,
        Err("exo_bridge_unavailable"),
        "the first attempt's error is reported as it happened"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "the failing attempt did the work once"
    );

    let recovered = cache.get(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Ok("b".repeat(64))
    });
    assert_eq!(
        recovered,
        Ok("b".repeat(64)),
        "a later success must be returned, not the cached failure"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "the second call must perform its own computation rather than replay the failure"
    );

    let pinned = cache.get(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Ok("c".repeat(64))
    });
    assert_eq!(
        pinned,
        Ok("b".repeat(64)),
        "once a success is pinned, later initialisers are never consulted"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "the recovered success is memoised, so the third call must not recompute"
    );
}

/// Repeated failures stay retryable rather than becoming permanent on the first one.
///
/// The same defect seen twice in a row is still one observation each time, so each call must run
/// its own attempt.
#[test]
fn repeated_failures_are_each_retried() {
    let cache = OnceDigest::new();
    let attempts = AtomicUsize::new(0);

    for expected in 1..=3 {
        let error = cache.get(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("exo_bridge_package")
        });
        assert_eq!(
            error,
            Err("exo_bridge_package"),
            "each failure is reported on its own terms"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            expected,
            "every failure is a fresh observation and must be retried"
        );
    }
}

/// The success path is still memoised: many callers, one computation.
///
/// This is #784's actual fix and the reason the cache exists at all. The count comes from the
/// initialiser itself, so an implementation that returned a constant without consulting it — or
/// one that simply dropped memoisation — fails here rather than passing quietly.
#[test]
fn many_successful_callers_compute_once() {
    let cache = OnceDigest::new();
    let attempts = AtomicUsize::new(0);

    for expected in 1..=4 {
        let digest = cache.get(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok("a".repeat(64))
        });
        assert_eq!(
            digest,
            Ok("a".repeat(64)),
            "every caller sees the pinned digest"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "call {expected} must be served from the pinned success"
        );
    }
}

/// Concurrent first callers still compute once.
///
/// This is what the `get`-then-`set` cache loses: `OnceLock::set` reports failure to a second
/// caller rather than waiting for it, so two threads racing the first call both hash the file.
/// The count is asserted after every thread has returned, not while they are still running.
#[test]
fn concurrent_first_callers_compute_once() {
    let cache = std::sync::Arc::new(OnceDigest::new());
    let attempts = std::sync::Arc::new(AtomicUsize::new(0));
    let callers = 8;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(callers));
    let mut handles = Vec::with_capacity(callers);

    for _ in 0..callers {
        let (cache, attempts, barrier) = (
            std::sync::Arc::clone(&cache),
            std::sync::Arc::clone(&attempts),
            std::sync::Arc::clone(&barrier),
        );
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            cache
                .get(|| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    Ok("a".repeat(64))
                })
                .expect("the initialiser succeeds")
        }));
    }

    for handle in handles {
        assert_eq!(
            handle.join().expect("no caller panicked"),
            "a".repeat(64),
            "every racing caller must see the same digest"
        );
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "eight concurrent first callers must still trigger exactly one computation"
    );
}

/// Concurrent callers on a failing path do not all run the initialiser at once.
///
/// This is the case a plain `OnceLock<String>` gets wrong once `Err` is no longer cached: with
/// nothing to block on, every thread arriving during a failing attempt runs the initialiser, so a
/// burst of parallel advertisements re-hashes the file on the one path that is meant to be
/// retryable. Callers that arrive while an attempt is in flight share its outcome instead, so the
/// eight callers released together cost one attempt between them, not eight; the next call is the
/// one that retries. Every caller must still observe the error — that is the point of the retry —
/// so the assertions are on the count *and* on the errors, and a "fix" that quietly returns `Ok`
/// to keep the count down cannot pass.
#[test]
fn concurrent_failures_do_not_fan_out() {
    let cache = std::sync::Arc::new(OnceDigest::new());
    let attempts = std::sync::Arc::new(AtomicUsize::new(0));
    let callers = 8;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(callers));
    let mut handles = Vec::with_capacity(callers);

    for _ in 0..callers {
        let (cache, attempts, barrier) = (
            std::sync::Arc::clone(&cache),
            std::sync::Arc::clone(&attempts),
            std::sync::Arc::clone(&barrier),
        );
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let error = cache.get(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
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
        handle.join().expect("no caller panicked");
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "callers that arrive during one attempt share it: the failing path must not fan out"
    );
}

/// A caller that parks behind an attempt reports that attempt's outcome, not a fresh one.
///
/// Two callers are released together, so the second finds the first already in flight. Both must
/// see one failure, and the initialiser must run once: if the second woke up on the failure and
/// then immediately computed for itself, it would have duplicated an attempt it had already been
/// told the answer to.
#[test]
fn a_waiter_reports_the_attempt_it_parked_behind() {
    let cache = std::sync::Arc::new(OnceDigest::new());
    let attempts = std::sync::Arc::new(AtomicUsize::new(0));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::with_capacity(2);

    for _ in 0..2 {
        let (cache, attempts, barrier) = (
            std::sync::Arc::clone(&cache),
            std::sync::Arc::clone(&attempts),
            std::sync::Arc::clone(&barrier),
        );
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            cache.get(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                Err("exo_bridge_unavailable")
            })
        }));
    }

    for handle in handles {
        assert_eq!(
            handle.join().expect("no caller panicked"),
            Err("exo_bridge_unavailable"),
            "both callers observe the one attempt's failure"
        );
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "the second caller must not recompute an attempt it waited on"
    );
}
