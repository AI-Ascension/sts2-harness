// SPDX-License-Identifier: MIT

use super::*;

#[path = "runtime_v3_lifecycle_recovery_runner_fixture.rs"]
mod fixture;

#[test]
fn episode_runner_dispatch_eof_is_durable_unknown_without_retry_or_ordinary_wait()
-> Result<(), Box<dyn std::error::Error>> {
    for mode in ["not-found", "unknown"] {
        fixture::run_in_process(mode)?;
    }
    Ok(())
}

#[test]
fn episode_runner_reconnects_after_historical_settlement_and_completes()
-> Result<(), Box<dyn std::error::Error>> {
    fixture::run_in_process("settled")
}

#[test]
fn episode_runner_fails_closed_before_gameplay_poll_for_invalid_recovery_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    for (lookup, reconcile) in [
        ("NOT_FOUND", "RECONCILED"),
        ("UNKNOWN", "UNKNOWN"),
        ("UNKNOWN", "NOT_FOUND"),
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, lookup);
        set_state(&mut case.reconcile, reconcile);
        if lookup == "NOT_FOUND" {
            case.lookup["payload"]["operation"] = Value::Null;
        }
        if reconcile == "NOT_FOUND" {
            case.reconcile["payload"]["operation"] = Value::Null;
            case.reconcile["payload"]["witness"] = Value::Null;
        }
        case.run_through_episode_runner(recovery_authority())?;
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(lookup != "NOT_FOUND", false)?;
    }

    for defect in [
        "missing",
        "other_operation",
        "other_context",
        "other_fence",
        "generation",
        "duplicate_witness",
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, "UNKNOWN");
        let witness = &mut case.reconcile["payload"]["operation"]["witness"];
        match defect {
            "missing" => *witness = Value::Null,
            "other_operation" => witness["operation_id"] = json!(PENDING_STATE_ID),
            "other_context" => witness["boot_id"] = json!(PENDING_STATE_ID),
            "other_fence" => witness["host_fence_id"] = json!(PENDING_STATE_ID),
            "generation" => witness["generation"] = json!(0),
            _ => {}
        }
        if defect != "duplicate_witness" {
            case.reconcile["payload"]["witness"] = witness.clone();
        } else {
            case.reconcile["payload"]["witness"]["witness_id"] = json!(PENDING_STATE_ID);
        }
        case.run_through_episode_runner(recovery_authority())?;
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(true, false)?;
    }

    let case = RecoveryCase::new()?;
    case.run_through_episode_runner(replacement_authority())?;
    assert!(
        case.durable
            .operation_state(PENDING_OPERATION_ID)?
            .is_unresolved()
    );
    case.assert_requests(true, false)?;
    Ok(())
}

#[test]
fn episode_runner_keeps_durable_uncertainty_when_retained_witness_is_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let case = RecoveryCase::new()?;
    case.run_through_episode_runner_with_wait(recovery_authority(), true)?;
    assert!(
        case.durable
            .operation_state(PENDING_OPERATION_ID)?
            .is_unresolved()
    );
    case.assert_requests(true, true)?;
    Ok(())
}
