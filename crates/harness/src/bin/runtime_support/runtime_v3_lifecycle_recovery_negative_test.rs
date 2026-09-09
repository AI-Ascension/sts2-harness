// SPDX-License-Identifier: MIT

use super::diagnostics;
use super::*;

#[test]
fn subprocess_missing_or_unresolved_evidence_does_not_poll_gameplay()
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
        let result = case.run();
        if let Err(error) = &result {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::new(format!(
                    "unresolved,lookup={lookup},reconcile={reconcile}"
                )),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            assert!(!error.is_empty(), "recovery failure must remain observable");
        }
        assert!(result.is_err());
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(lookup != "NOT_FOUND", false)?;
    }
    Ok(())
}

#[test]
fn subprocess_missing_or_cross_operation_witness_never_closes_durable_uncertainty()
-> Result<(), Box<dyn std::error::Error>> {
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
        let result = case.run();
        if let Err(error) = &result {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::new(format!("witness_defect,{defect}")),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            assert!(!error.is_empty(), "recovery failure must remain observable");
        }
        assert!(result.is_err(), "accepted {defect}");
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(true, false)?;
    }
    Ok(())
}
