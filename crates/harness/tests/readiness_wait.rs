// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Focused fault/recovery matrix for the identity-bound readiness wait
//! (issue #96, T1/T2). Every case is synthetic: it exercises the source-only
//! contract and does not assert a native game effect.

use sts2_harness::management::{
    READINESS_CONTRACT_VERSION, ReadinessMilestone, ReadinessObservation, ReadinessProgress,
    ReadinessTarget, ReadinessTerminal, ReadinessWait, ReadinessWaitError,
};

const INSTANCE: &str = "instance-1";
const EPOCH: u64 = 7;
const GENERATION: u64 = 3;

fn digest(seed: u8) -> String {
    std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
}

fn observation() -> ReadinessObservation {
    ReadinessObservation::new(INSTANCE, EPOCH, "obs-1", digest(0xab)).expect("observation")
}

fn target(milestone: ReadinessMilestone) -> ReadinessTarget {
    ReadinessTarget::new(milestone, 1_000, 8, READINESS_CONTRACT_VERSION).expect("target")
}

fn wait(milestone: ReadinessMilestone) -> ReadinessWait {
    ReadinessWait::begin(INSTANCE, EPOCH, GENERATION, target(milestone)).expect("wait")
}

#[test]
fn unsupported_contract_version_is_refused_before_starting() {
    assert_eq!(
        ReadinessTarget::new(
            ReadinessMilestone::Booted,
            1_000,
            8,
            READINESS_CONTRACT_VERSION + 1
        ),
        Err(ReadinessWaitError::Incompatible)
    );
}

#[test]
fn unbounded_targets_are_refused() {
    assert_eq!(
        ReadinessTarget::new(ReadinessMilestone::Booted, 0, 8, READINESS_CONTRACT_VERSION),
        Err(ReadinessWaitError::InvalidTarget)
    );
    assert_eq!(
        ReadinessTarget::new(
            ReadinessMilestone::Booted,
            1_000,
            0,
            READINESS_CONTRACT_VERSION
        ),
        Err(ReadinessWaitError::InvalidTarget)
    );
}

#[test]
fn target_deserialization_rejects_unknown_members() {
    let unknown = r#"{"milestone":"booted","deadline_ms":1000,"max_attempts":8,
        "contract_version":1,"sleep_ms":50}"#;
    assert!(serde_json::from_str::<ReadinessTarget>(unknown).is_err());
}

#[test]
fn target_deserialization_of_unknown_version_is_refused_on_validate() {
    let future = r#"{"milestone":"actionable","deadline_ms":1000,"max_attempts":8,
        "contract_version":99}"#;
    let parsed: ReadinessTarget = serde_json::from_str(future).expect("well-formed json");
    assert_eq!(parsed.validate(), Err(ReadinessWaitError::Incompatible));
}

#[test]
fn standard_target_is_backward_compatible_at_the_current_version() {
    let standard = ReadinessTarget::standard(ReadinessMilestone::Actionable);
    assert_eq!(standard.contract_version, READINESS_CONTRACT_VERSION);
    assert_eq!(standard.validate(), Ok(standard));
}

#[test]
fn milestone_labels_are_stable_and_ordered() {
    assert_eq!(ReadinessMilestone::Booted.label(), "booted");
    assert_eq!(
        ReadinessMilestone::AdapterCompatible.label(),
        "adapter_compatible"
    );
    assert_eq!(
        ReadinessMilestone::LeaseInstalled.label(),
        "lease_installed"
    );
    assert_eq!(
        ReadinessMilestone::SetupAvailable.label(),
        "setup_available"
    );
    assert_eq!(ReadinessMilestone::Actionable.label(), "actionable");
    assert!(ReadinessMilestone::Actionable.reaches(ReadinessMilestone::LeaseInstalled));
    assert!(!ReadinessMilestone::Booted.reaches(ReadinessMilestone::LeaseInstalled));
}

#[test]
fn wait_settles_only_when_the_target_milestone_is_reached() {
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::LeaseInstalled);
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION, 10),
        Ok(ReadinessProgress::AwaitingMore)
    );
    assert!(!wait.is_settled());
    assert_eq!(
        wait.observe(
            &evidence,
            ReadinessMilestone::LeaseInstalled,
            GENERATION,
            20
        ),
        Ok(ReadinessProgress::Satisfied)
    );
    assert_eq!(wait.terminal(), Some(ReadinessTerminal::Satisfied));
}

#[test]
fn a_listening_port_cannot_satisfy_gameplay_readiness() {
    // A bootable process that answers on a port reports `Booted`, not
    // `Actionable`; it must never settle a gameplay-readiness wait.
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::Actionable);
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION, 5),
        Ok(ReadinessProgress::AwaitingMore)
    );
    assert!(!wait.is_settled());
}

#[test]
fn foreign_instance_or_epoch_evidence_is_refused() {
    let mut wait = wait(ReadinessMilestone::Booted);
    let foreign_instance =
        ReadinessObservation::new("instance-2", EPOCH, "obs-1", digest(0x01)).expect("observation");
    assert_eq!(
        wait.observe(&foreign_instance, ReadinessMilestone::Booted, GENERATION, 1),
        Err(ReadinessWaitError::ForeignReadiness)
    );
    let superseded_epoch =
        ReadinessObservation::new(INSTANCE, EPOCH - 1, "obs-1", digest(0x02)).expect("observation");
    assert_eq!(
        wait.observe(&superseded_epoch, ReadinessMilestone::Booted, GENERATION, 2),
        Err(ReadinessWaitError::ForeignReadiness)
    );
    assert!(!wait.is_settled());
}

#[test]
fn stale_generation_is_refused_without_spending_the_budget() {
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::Booted);
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION - 1, 1),
        Err(ReadinessWaitError::StaleReadiness)
    );
    assert_eq!(wait.attempts(), 0);
    assert!(!wait.is_settled());
}

#[test]
fn deadline_exhaustion_times_out_and_is_terminal() {
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::Booted);
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION, 1_001),
        Err(ReadinessWaitError::Timeout)
    );
    assert_eq!(wait.terminal(), Some(ReadinessTerminal::TimedOut));
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION, 1_002),
        Err(ReadinessWaitError::Settled)
    );
}

#[test]
fn attempt_budget_exhaustion_times_out() {
    let evidence = observation();
    let narrow = ReadinessTarget::new(
        ReadinessMilestone::Actionable,
        10_000,
        2,
        READINESS_CONTRACT_VERSION,
    )
    .expect("target");
    let mut wait = ReadinessWait::begin(INSTANCE, EPOCH, GENERATION, narrow).expect("wait");
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Booted, GENERATION, 1),
        Ok(ReadinessProgress::AwaitingMore)
    );
    assert_eq!(
        wait.observe(
            &evidence,
            ReadinessMilestone::AdapterCompatible,
            GENERATION,
            2
        ),
        Ok(ReadinessProgress::AwaitingMore)
    );
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::LeaseInstalled, GENERATION, 3),
        Err(ReadinessWaitError::Timeout)
    );
    assert_eq!(wait.terminal(), Some(ReadinessTerminal::TimedOut));
}

#[test]
fn denial_and_cancellation_are_distinguishable() {
    let mut denied = wait(ReadinessMilestone::Booted);
    denied.deny().expect("deny");
    assert_eq!(denied.terminal(), Some(ReadinessTerminal::Denied));

    let mut cancelled = wait(ReadinessMilestone::Booted);
    cancelled.cancel().expect("cancel");
    assert_eq!(cancelled.terminal(), Some(ReadinessTerminal::Cancelled));
}

#[test]
fn a_restart_invalidates_prior_readiness_and_requires_a_fresh_wait() {
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::Actionable);
    wait.invalidate_for_restart().expect("invalidate");
    assert_eq!(wait.terminal(), Some(ReadinessTerminal::Invalidated));
    assert_eq!(
        wait.observe(&evidence, ReadinessMilestone::Actionable, GENERATION, 1),
        Err(ReadinessWaitError::Settled)
    );
    let fresh = ReadinessWait::begin(
        INSTANCE,
        EPOCH,
        GENERATION + 1,
        target(ReadinessMilestone::Actionable),
    )
    .expect("fresh wait");
    assert_eq!(fresh.generation(), GENERATION + 1);
    assert!(!fresh.is_settled());
}

#[test]
fn begin_refuses_incomplete_bindings() {
    assert_eq!(
        ReadinessWait::begin("", EPOCH, GENERATION, target(ReadinessMilestone::Booted)),
        Err(ReadinessWaitError::InvalidTarget)
    );
    assert_eq!(
        ReadinessWait::begin(INSTANCE, 0, GENERATION, target(ReadinessMilestone::Booted)),
        Err(ReadinessWaitError::InvalidTarget)
    );
    assert_eq!(
        ReadinessWait::begin(INSTANCE, EPOCH, 0, target(ReadinessMilestone::Booted)),
        Err(ReadinessWaitError::InvalidTarget)
    );
}

#[test]
fn settling_twice_is_refused() {
    let evidence = observation();
    let mut wait = wait(ReadinessMilestone::Booted);
    wait.observe(&evidence, ReadinessMilestone::Actionable, GENERATION, 1)
        .expect("satisfy");
    assert_eq!(wait.deny(), Err(ReadinessWaitError::Settled));
    assert_eq!(wait.cancel(), Err(ReadinessWaitError::Settled));
}
