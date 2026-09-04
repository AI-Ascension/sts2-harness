// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::error::Error;

use serde_json::json;
use sts2_harness::{
    ActionId, Correlation, DecisionPayload, DecisionRecord, DecisionRecordKind, DecisionReplay,
    DecisionReplayRequest, EpisodeId, EvidenceStatus, InstanceId, RecordId, RunId, TraceId,
    TrajectoryId,
};

fn correlation(trajectory_id: TrajectoryId) -> Correlation {
    Correlation::for_episode(
        RunId::new(1).expect("nonzero run"),
        EpisodeId::new(2).expect("nonzero episode"),
        trajectory_id,
        InstanceId::new(3).expect("nonzero instance"),
        TraceId::new(4).expect("nonzero trace"),
    )
}

fn record(
    trajectory_id: TrajectoryId,
    sequence: u64,
    kind: DecisionRecordKind,
    payload: serde_json::Value,
) -> DecisionRecord {
    DecisionRecord::new(
        RecordId::new(sequence + 1).expect("nonzero record"),
        sequence,
        correlation(trajectory_id),
        kind,
        EvidenceStatus::Confirmed,
        Some(sequence),
        Some("combat-1".to_owned()),
        Some("operation-1".to_owned()),
        Some(ActionId::new(7).expect("nonzero action")),
        None,
        DecisionPayload::from_json(payload).expect("safe payload"),
    )
    .expect("record is valid")
}

#[test]
fn typed_replay_counts_each_evidence_class_without_merging_unknowns() {
    let trajectory = TrajectoryId::new(9).expect("nonzero trajectory");
    let records = vec![
        record(
            trajectory,
            0,
            DecisionRecordKind::Observation,
            json!({"state":"combat"}),
        ),
        record(
            trajectory,
            1,
            DecisionRecordKind::Request,
            json!({"kind":"dispatch"}),
        ),
        record(
            trajectory,
            2,
            DecisionRecordKind::Acceptance,
            json!({"status":"accepted"}),
        ),
        record(
            trajectory,
            3,
            DecisionRecordKind::Settlement,
            json!({"status":"settled"}),
        ),
        record(
            trajectory,
            4,
            DecisionRecordKind::Recovery,
            json!({"kind":"reobserve"}),
        ),
        record(
            trajectory,
            5,
            DecisionRecordKind::Estimate,
            json!({"source":"belief"}),
        ),
        record(
            trajectory,
            6,
            DecisionRecordKind::Unavailable,
            json!({"code":"timeout"}),
        ),
    ];
    let report = DecisionReplay::evaluate(&DecisionReplayRequest::new(trajectory, records));
    assert_eq!(report.records_replayed(), 7);
    assert_eq!(report.observations(), 1);
    assert_eq!(report.requests(), 1);
    assert_eq!(report.acceptances(), 1);
    assert_eq!(report.settlements(), 1);
    assert_eq!(report.recoveries(), 1);
    assert_eq!(report.estimates(), 1);
    assert_eq!(report.unavailable(), 1);
    assert!(report.divergence().is_none());
}

#[test]
fn typed_replay_stops_at_sequence_divergence_and_payload_firewall_rejects_privileged_data()
-> Result<(), Box<dyn Error>> {
    let trajectory = TrajectoryId::new(10).expect("nonzero trajectory");
    let mut records = vec![record(
        trajectory,
        0,
        DecisionRecordKind::Observation,
        json!({"state":"map"}),
    )];
    records.push(record(
        trajectory,
        2,
        DecisionRecordKind::Unavailable,
        json!({"code":"gap"}),
    ));
    let report = DecisionReplay::evaluate(&DecisionReplayRequest::new(trajectory, records));
    assert_eq!(report.records_replayed(), 1);
    assert_eq!(
        report.divergence().map(|value| value.actual_sequence()),
        Some(2)
    );
    assert!(DecisionPayload::from_json(json!({"raw_memory":"blocked"})).is_err());
    Ok(())
}
