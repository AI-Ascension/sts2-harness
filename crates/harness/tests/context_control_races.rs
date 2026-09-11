// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sts2_harness::{ContextBoundary, ControlAuthority, GateStatus};

const REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        state_id: "combat-1".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: REVISION.to_owned(),
        model_revision: "model-v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 0,
    }
}

#[test]
fn pause_admission_race_has_one_order_and_drains_provider_before_ready() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    authority
        .admit_operation("provider-attempt-1", authority.state().plan_epoch)
        .expect("provider wins the pre-pause admission boundary");
    let paused = authority
        .request_pause("pause-race", authority.state().control_version)
        .expect("pause latches while provider is running");
    assert_eq!(authority.state().status, GateStatus::PauseRequested);
    let pause_boundary = authority.state().boundary.clone();
    let pause_version = authority.state().control_version;
    assert!(
        authority
            .resume("resume-before-drain", pause_version, &pause_boundary)
            .is_err()
    );
    authority
        .complete_provider_attempt("provider-attempt-1", paused.plan_epoch)
        .expect("provider completion drains the pause barrier");
    assert_eq!(authority.state().status, GateStatus::PausedReady);

    let mut after_pause = ControlAuthority::new(boundary(), "revision-1");
    after_pause
        .request_pause("pause-first", after_pause.state().control_version)
        .expect("pause wins the second ordering");
    assert_eq!(
        after_pause.admit_operation("provider-attempt-2", after_pause.state().plan_epoch),
        Err("obsolete_plan".to_owned())
    );
}

#[test]
fn unsettled_game_operation_and_unknown_mutation_block_readiness() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    authority
        .admit_operation("game-operation-1", authority.state().plan_epoch)
        .expect("game operation is admitted");
    authority
        .request_pause("pause-game", authority.state().control_version)
        .expect("pause latches during game operation");
    assert_eq!(authority.state().status, GateStatus::PauseRequested);
    authority
        .retain_unknown_operation("game-operation-1")
        .expect("unknown mutation remains unresolved");
    assert_eq!(
        authority.state().unresolved_operations,
        ["game-operation-1"]
    );
    let unknown_boundary = authority.state().boundary.clone();
    let unknown_version = authority.state().control_version;
    assert!(
        authority
            .resume("resume-unknown", unknown_version, &unknown_boundary)
            .is_err()
    );
    authority
        .settle_operation("game-operation-1")
        .expect("explicit reconciliation settles the same operation");
    assert_eq!(authority.state().status, GateStatus::PausedReady);
}

#[test]
fn multiple_participant_operations_cannot_report_run_wide_readiness_early() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    let epoch = authority.state().plan_epoch;
    authority
        .admit_operation("participant-a-provider", epoch)
        .expect("first participant operation is admitted");
    authority
        .admit_operation("participant-b-game", epoch)
        .expect("second participant operation is admitted");
    authority
        .request_pause("pause-multi", authority.state().control_version)
        .expect("pause latches for all participants");
    authority
        .settle_operation("participant-a-provider")
        .expect("first participant reconciles");
    assert_eq!(authority.state().status, GateStatus::PauseRequested);
    assert_eq!(
        authority.state().unresolved_operations,
        ["participant-b-game"]
    );
    authority
        .settle_operation("participant-b-game")
        .expect("second participant reconciles");
    assert_eq!(authority.state().status, GateStatus::PausedReady);
}

#[test]
fn old_provider_completion_is_discarded_after_commit_advances_the_plan_epoch() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    authority
        .request_pause("pause-old-provider", authority.state().control_version)
        .expect("pause");
    let old_plan_epoch = authority.state().plan_epoch;
    let expected_boundary = authority.state().boundary.clone();
    authority
        .commit(
            "commit-old-provider",
            authority.state().control_version,
            "revision-1",
            &expected_boundary,
            "manifest",
            "manifest",
        )
        .expect("commit advances the plan epoch");
    assert_eq!(
        authority.complete_provider_attempt("late-provider", old_plan_epoch),
        Err("obsolete_plan".to_owned())
    );
    assert_eq!(authority.state().active_revision_id, "revision-2");
}

#[test]
fn game_boundary_change_invalidates_the_final_resume_check() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    authority
        .request_pause("pause-boundary-change", authority.state().control_version)
        .expect("pause");
    let expected_boundary = authority.state().boundary.clone();
    authority.advance_boundary();
    assert_eq!(
        authority.resume(
            "resume-boundary-change",
            authority.state().control_version,
            &expected_boundary,
        ),
        Err("preview_stale".to_owned())
    );
}

#[test]
fn operation_identifiers_are_bounded_before_the_control_ledger_changes() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    assert_eq!(
        authority.admit_operation("bad/id", authority.state().plan_epoch),
        Err("invalid_operation".to_owned())
    );
    assert!(authority.state().unresolved_operations.is_empty());
}
