// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::panic)]

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use sts2_harness as harness_api;

#[path = "support/exo_lifecycle.rs"]
mod fixture;
use fixture::Fixture;

/// Bounded attempts and the interval between them, matching `Lease::acquire`.
const ATTEMPTS: usize = 32;
const RETRY: Duration = Duration::from_millis(5);

/// Acquire `path` the way `Lease::acquire` does: retry the non-blocking attempt for a bounded
/// interval, because a descriptor this process has already closed can still hold the `flock` until
/// a spawned child reaches `execve`.
fn lock_with_retry(path: &Path) -> File {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("holder descriptor");
    for _ in 0..ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return file,
            Err(TryLockError::WouldBlock) => std::thread::sleep(RETRY),
            Err(error) => panic!("holder lock error: {error:?}"),
        }
    }
    panic!("the holder never observed the lock as free");
}

/// A transient holder of `owner.lock` is not a live owner, so re-acquisition must wait it out
/// rather than report `Busy`.
///
/// `flock` belongs to the open file description: it is released when the last descriptor referring
/// to that description is closed, which is not necessarily when the process that took it stops
/// using the lease. The `exo_lifecycle` tests spawn children while other tests acquire, and a
/// descriptor this process has already closed can still be held by a spawned child until it reaches
/// `execve`; that is what made them report a spurious `Busy`. Waiting for a bounded interval cannot
/// admit a second owner, because a lock held by a live lease or by another process is not released
/// by waiting.
///
/// The holder below acquires with the same bounded retry, because it is exposed to the identical
/// window while sibling tests spawn children, and it releases only part-way through the
/// re-acquisition below, so the wait is exercised rather than assumed.
#[test]
fn reacquire_waits_out_a_transient_holder_instead_of_reporting_busy() {
    let mut fixture = Fixture::new();
    drop(fixture.owner());
    let path = fixture.config.directory.join("owner.lock");

    let (holding, holding_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (released, released_rx) = mpsc::channel();
    let holder_path = path.clone();
    let holder = std::thread::spawn(move || {
        let file = lock_with_retry(&holder_path);
        holding.send(()).expect("holder signal");
        release_rx.recv().expect("release signal");
        drop(file);
        released.send(()).expect("released");
    });
    holding_rx.recv().expect("holder reached the lock");

    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("probe descriptor");
    assert!(
        matches!(probe.try_lock(), Err(TryLockError::WouldBlock)),
        "the transient holder must hold the lock"
    );
    drop(probe);

    // The holder releases only inside the re-acquisition window below.
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        release.send(()).expect("release");
    });
    let owner = fixture
        .reopen()
        .expect("a transient holder must not be reported as a busy lease");
    drop(owner);
    releaser.join().expect("releaser thread");
    released_rx.recv().expect("holder released");
    holder.join().expect("holder thread");
}
