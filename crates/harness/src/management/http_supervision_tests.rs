// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use std::sync::mpsc;

#[test]
fn abandoned_pending_supervisor_holds_finite_admission_until_joined() {
    let mut owners = Vec::new();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let dispatch = Arc::new(dispatch_executor::DispatchExecutor::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    let owner = start_owned(
        &mut owners,
        1,
        Arc::clone(&stop),
        Arc::clone(&dispatch),
        move || {
            entered_tx.send(()).expect("entered");
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("release");
            Ok(())
        },
    )
    .expect("first owner");
    entered_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("running");
    let weak = Arc::downgrade(&owner);
    stop.store(true, Ordering::Release);
    drop(owner);
    reap(&mut owners);
    assert_eq!(owners.len(), 1, "pending owner cannot be evicted");
    assert!(weak.upgrade().is_some());

    let attempted = Arc::new(AtomicBool::new(false));
    let in_task = Arc::clone(&attempted);
    let result = start_owned(
        &mut owners,
        1,
        Arc::clone(&stop),
        Arc::clone(&dispatch),
        move || {
            in_task.store(true, Ordering::Release);
            Ok(())
        },
    );
    let error = result.err().expect("capacity refusal");
    assert_eq!(error.code, "server_supervision_capacity_exhausted");
    assert_eq!(error.status(), 503);
    assert!(
        !attempted.load(Ordering::Acquire),
        "refused task was never spawned"
    );
    release_tx.send(()).expect("settle retained owner");
    owners[0]
        .wait(Some(Instant::now() + Duration::from_secs(3)))
        .expect("join")
        .expect("settled")
        .expect("task result");
    reap(&mut owners);
    assert!(owners.is_empty());
    assert!(weak.upgrade().is_none());

    let replacement = start_owned(&mut owners, 1, stop, dispatch, || Ok(()))
        .expect("capacity released only after completion");
    replacement
        .wait(Some(Instant::now() + Duration::from_secs(3)))
        .expect("replacement join")
        .expect("finished")
        .expect("result");
    reap(&mut owners);
    assert!(owners.is_empty());
}
