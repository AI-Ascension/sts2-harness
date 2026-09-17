// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{observation, setup};
use sts2_harness::management::{LiveContextObservationPort, RuntimeAuthorityBinding};

/// A superseded or foreign runtime lease must not be able to re-establish a
/// run's context authority by winning the first observation of a restarted
/// owner. Only the run's current (post-allocation) lease may do so.
#[test]
fn stale_or_foreign_runtime_lease_is_refused_as_the_first_observation() {
    let (owner, actor, request, binding, digest, selected) = setup();
    let allocated = RuntimeAuthorityBinding {
        lease_epoch: 4,
        ..binding.clone()
    };
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &allocated,
            &observation(1),
            &selected,
        )
        .expect("genuine allocated lease observation");
    for (case, candidate) in [
        (
            "superseded lease epoch",
            RuntimeAuthorityBinding {
                lease_epoch: 2,
                ..allocated.clone()
            },
        ),
        (
            "foreign lease id",
            RuntimeAuthorityBinding {
                lease_id: "replacement-lease".into(),
                ..allocated.clone()
            },
        ),
    ] {
        owner.current.lock().expect("lock").clear();
        let outcome = owner.record_observation(
            &actor,
            &request,
            &digest,
            &candidate,
            &observation(2),
            &selected,
        );
        if case == "superseded lease epoch" {
            println!(
                "L94FIX2 stale/foreign lease admitted as FIRST observation = {}",
                outcome.is_ok()
            );
        }
        let error = outcome.expect_err("stale or foreign lease must not re-establish authority");
        assert_eq!(error.code, "context_owner_runtime_scope", "{case}");
        assert!(
            !owner
                .current
                .lock()
                .expect("lock")
                .contains_key(&allocated.run_id),
            "{case} must not install a run authority"
        );
    }
}

/// A genuine post-restart re-allocation advances the run's lease epoch and
/// must still be admitted, so the refusal above cannot be an over-refusal.
#[test]
fn advanced_runtime_lease_is_admitted_as_the_first_observation() {
    let (owner, actor, request, binding, digest, selected) = setup();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &binding,
            &observation(1),
            &selected,
        )
        .expect("genuine first observation");
    let reallocated = RuntimeAuthorityBinding {
        lease_id: "lease-2".into(),
        lease_epoch: 5,
        ..binding.clone()
    };
    owner.current.lock().expect("lock").clear();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &reallocated,
            &observation(2),
            &selected,
        )
        .expect("re-allocated lease is admitted after restart");
}
