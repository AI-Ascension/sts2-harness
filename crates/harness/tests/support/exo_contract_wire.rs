// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn control_receipts_bind_run_episode_and_turn_without_model_fields() {
    let identity = control_identity("run-1");
    let mut expected = identity.clone();
    expected.run_id = String::from("run-2");
    assert_eq!(
        verify_control_identity(&identity, &expected),
        Err(ExoWireError::IdentityMismatch)
    );
    let receipt = sts2_harness::ExoBridgeTurn {
        identity,
        outcome: ExoTerminalOutcome::Decision,
    };
    receipt
        .validate()
        .expect("control receipt identity is valid");
}
