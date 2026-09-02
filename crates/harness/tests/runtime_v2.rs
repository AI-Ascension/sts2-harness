// SPDX-License-Identifier: MIT

use std::error::Error;

use serde_json::Value;
use sha2::{Digest, Sha256};
use sts2_harness::{
    RUNTIME_V2_SCHEMA_DIGEST, RuntimeV2EventKind, RuntimeV2Kind, RuntimeV2Status,
    run_runtime_v2_fake_trace, runtime_v2_schema_bytes, verify_runtime_v2_artifact,
};

#[test]
fn copied_runtime_v2_artifact_matches_the_protocol_handoff() -> Result<(), Box<dyn Error>> {
    verify_runtime_v2_artifact()?;
    assert_eq!(
        format!("{:x}", Sha256::digest(runtime_v2_schema_bytes())),
        RUNTIME_V2_SCHEMA_DIGEST
    );
    Ok(())
}

#[test]
fn fake_trace_proves_reconciliation_replay_and_epoch_fencing() -> Result<(), Box<dyn Error>> {
    let first = run_runtime_v2_fake_trace()?;
    let second = run_runtime_v2_fake_trace()?;
    assert_eq!(first, second);

    let evidence = first.evidence();
    assert_eq!(evidence.operation_id().as_str(), "op-1");
    assert_eq!(evidence.initial_generation(), 4);
    assert_eq!(evidence.settled_generation(), 5);
    assert_eq!(evidence.mutation_count(), 1);
    assert!(evidence.duplicate_replay_without_second_application());
    assert!(evidence.stale_epoch_rejected());
    assert!(evidence.no_blind_retry_after_disconnect());
    assert_eq!(evidence.live_host_settlement(), "unverified");
    assert_eq!(evidence.provider_model_lane(), "unverified");

    let records = first.trajectory().records();
    assert_eq!(records.len(), 13);
    assert_eq!(records[2].event_kind(), RuntimeV2EventKind::Requested);
    assert_eq!(records[3].event_kind(), RuntimeV2EventKind::Accepted);
    assert_eq!(records[4].event_kind(), RuntimeV2EventKind::Unknown);
    assert_eq!(records[5].event_kind(), RuntimeV2EventKind::Reconciled);
    assert_eq!(records[6].event_kind(), RuntimeV2EventKind::Settled);
    assert_eq!(records[12].event_kind(), RuntimeV2EventKind::Rejected);
    assert_eq!(records[2].message().kind(), RuntimeV2Kind::ActionRequest);
    assert_eq!(
        records[3].message().status(),
        Some(RuntimeV2Status::Accepted)
    );
    assert_eq!(
        records[4].message().status(),
        Some(RuntimeV2Status::Unknown)
    );
    assert_eq!(
        records[6].message().status(),
        Some(RuntimeV2Status::Settled)
    );
    assert_eq!(
        records[10].message().status(),
        Some(RuntimeV2Status::Settled)
    );
    assert_eq!(
        records[12].message().status(),
        Some(RuntimeV2Status::Rejected)
    );
    assert_eq!(
        records[4].message().operation_id().map(|id| id.as_str()),
        Some("op-1")
    );
    assert_eq!(
        records[6].message().operation_id().map(|id| id.as_str()),
        Some("op-1")
    );
    assert_eq!(
        records[6]
            .message()
            .effect_witness()
            .map(|witness| witness.kind.as_str()),
        Some("turn_end_settled")
    );
    assert_eq!(records[12].message().lease_epoch(), 0);
    assert_eq!(
        records[12].message().error_code(),
        Some("sts2.gateway/stale_lease_epoch")
    );
    assert!(records[4].no_retry().disconnect_after_write());
    assert_eq!(records[4].no_retry().retry_attempts(), 0);
    assert_eq!(records[4].no_retry().mutation_attempts(), 1);

    let document: Value = serde_json::from_str(first.trace_bytes())?;
    assert_eq!(document["artifact"]["schema_bytes_verified"], true);
    assert_eq!(
        document["artifact"]["schema_bytes_digest"],
        RUNTIME_V2_SCHEMA_DIGEST
    );
    assert_eq!(document["trajectory"]["records"][6]["generation"], 5);
    assert_eq!(
        document["trajectory"]["records"][6]["effect_witness"]["generation"],
        5
    );
    Ok(())
}
