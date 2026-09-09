// SPDX-License-Identifier: MIT

use super::diagnostics;
use super::*;

#[test]
fn subprocess_recovery_resolves_unknown_and_accepts_retained_terminal_states()
-> Result<(), Box<dyn std::error::Error>> {
    for (lookup, reconciled) in [
        ("UNKNOWN", "RECONCILED"),
        ("ACCEPTED", "RECONCILED"),
        ("MAY_HAVE_BEEN_DISPATCHED", "RECONCILED"),
        ("INTENT_RECORDED", "RECONCILED"),
        ("SETTLED", "SETTLED"),
        ("RECONCILED", "RECONCILED"),
        ("REJECTED", "REJECTED"),
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, lookup);
        set_state(&mut case.reconcile, reconciled);
        if reconciled == "REJECTED" {
            for frame in [&mut case.lookup, &mut case.reconcile] {
                frame["payload"]["operation"]["ticket"]["state"] = json!("REJECTED");
                frame["payload"]["operation"]["witness"] = Value::Null;
            }
            case.reconcile["payload"]["witness"] = Value::Null;
        }
        if let Err(error) = case.run() {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::new(format!(
                    "retained_terminal,lookup={lookup},reconcile={reconciled}"
                )),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            return Err(error.into());
        }
        assert_eq!(
            case.durable.operation_state(PENDING_OPERATION_ID)?,
            OperationState::Reconciled
        );
        case.assert_requests(true, reconciled != "REJECTED")?;
    }
    Ok(())
}
