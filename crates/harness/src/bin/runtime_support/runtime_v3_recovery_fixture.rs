// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::{
    Fixture, RECOVERY_BOOT_ID, RECOVERY_DEPLOYMENT_ID, RECOVERY_FENCE_ID, RECOVERY_INSTANCE_ID,
    RECOVERY_INSTANCE_INCAR, RECOVERY_LEASE_ID, RECOVERY_LOOKUP_CORRELATION_ID,
    RECOVERY_LOOKUP_MESSAGE_ID, RECOVERY_PRINCIPAL_ID, RECOVERY_RECONCILE_CORRELATION_ID,
    RECOVERY_RECONCILE_MESSAGE_ID, RECOVERY_TICKET_ID, RECOVERY_WITNESS_ID, reply, wire,
};

#[path = "runtime_v3_recovery_fixture_script.rs"]
mod script;
pub(in super::super) use script::{response_script, response_script_with_identity};

pub(crate) fn recovery_settled_script(
    fixture: &Fixture,
    operation_id: &str,
    state_id: &str,
    generation: u64,
    payload_digest: &str,
    catalog_digest: &str,
    canonical_json_b64: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (lookup, reconcile) = settled_frames(
        operation_id,
        state_id,
        generation,
        payload_digest,
        catalog_digest,
        canonical_json_b64,
    );
    response_script(fixture, &lookup, &reconcile)
}

pub(in super::super) fn settled_frames(
    operation_id: &str,
    state_id: &str,
    generation: u64,
    payload_digest: &str,
    catalog_digest: &str,
    canonical_json_b64: &str,
) -> (Value, Value) {
    let witness = json!({
        "witness_id": RECOVERY_WITNESS_ID,
        "operation_id": operation_id,
        "payload_digest": payload_digest,
        "boot_id": RECOVERY_BOOT_ID,
        "instance_incarnation": RECOVERY_INSTANCE_INCAR,
        "host_fence_id": RECOVERY_FENCE_ID,
        "source": "authoritative_reobserve",
        "state_id": state_id,
        "generation": generation + 1,
        "effect_digest": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "observed_at": "2026-09-07T00:00:01Z"
    });
    let operation = |state: &str| {
        let ticket_state = if state == "RECONCILED" {
            "SETTLED"
        } else {
            state
        };
        json!({
            "operation_id": operation_id,
            "state": state,
            "payload_digest": payload_digest,
            "original_context": {
                "deployment_id": RECOVERY_DEPLOYMENT_ID,
                "instance_id": RECOVERY_INSTANCE_ID,
                "instance_incarnation": RECOVERY_INSTANCE_INCAR,
                "boot_id": RECOVERY_BOOT_ID,
                "authority_generation": 1,
                "lease_id": RECOVERY_LEASE_ID,
                "lease_epoch": 1
            },
            "expected_boundary": {
                "state_id": state_id,
                "generation": generation,
                "catalog_digest": catalog_digest
            },
            "action": {
                "schema_digest": wire::RUNTIME_V3_SCHEMA_DIGEST,
                "canonical_json_b64": canonical_json_b64,
                "payload_digest": payload_digest
            },
            "ticket": {
                "ticket_id": RECOVERY_TICKET_ID,
                "operation_id": operation_id,
                "payload_digest": payload_digest,
                "boot_id": RECOVERY_BOOT_ID,
                "instance_incarnation": RECOVERY_INSTANCE_INCAR,
                "lease_epoch": 1,
                "host_fence_id": RECOVERY_FENCE_ID,
                "state": ticket_state,
                "issued_at": "2026-09-07T00:00:00Z",
                "expires_at": "2026-09-07T00:00:30Z"
            },
            "witness": witness,
            "uncertainty_reason": null,
            "created_at": "2026-09-07T00:00:00Z",
            "updated_at": "2026-09-07T00:00:01Z"
        })
    };
    let lookup = json!({
        "contract": "watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "message_id": RECOVERY_LOOKUP_MESSAGE_ID,
        "correlation_id": RECOVERY_LOOKUP_CORRELATION_ID,
        "sent_at": "2026-09-07T00:00:01Z",
        "actor": {"principal_id": RECOVERY_PRINCIPAL_ID, "role": "gateway"},
        "auth": {"principal_id": RECOVERY_PRINCIPAL_ID, "capability": "recovery_read", "proof": "synthetic-proof"},
        "kind": "operation_lookup_response",
        "payload": {
            "operation": operation("SETTLED"),
            "mutation_authorized": false,
            "result": {"status": "SETTLED", "retryable": false, "retry_after_seconds": null}
        }
    });
    let reconcile = json!({
        "contract": "watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "message_id": RECOVERY_RECONCILE_MESSAGE_ID,
        "correlation_id": RECOVERY_RECONCILE_CORRELATION_ID,
        "sent_at": "2026-09-07T00:00:01Z",
        "actor": {"principal_id": RECOVERY_PRINCIPAL_ID, "role": "gateway"},
        "auth": {"principal_id": RECOVERY_PRINCIPAL_ID, "capability": "recovery_reconcile", "proof": "synthetic-proof"},
        "kind": "operation_reconcile_response",
        "payload": {
            "operation": operation("RECONCILED"),
            "witness": witness,
            "result": {"status": "RECONCILED", "retryable": false, "retry_after_seconds": null}
        }
    });
    (lookup, reconcile)
}
