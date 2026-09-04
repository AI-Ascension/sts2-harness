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

fn lineage_record(change: usize) -> DecisionRecord {
    let id = |field| if change == field { 2 } else { 1 };
    let mut correlation = Correlation::for_episode(
        RunId::new(id(7)).expect("run"),
        EpisodeId::new(id(8)).expect("episode"),
        TrajectoryId::new(id(16)).expect("trajectory"),
        InstanceId::new(id(9)).expect("instance"),
        TraceId::new(id(10)).expect("trace"),
    );
    let model = (change == 6).then(|| sts2_harness::ModelExecutionId::new(1).expect("model"));
    if let Some(model) = model {
        correlation = correlation.with_model_execution(model);
    }
    if change == 11 {
        correlation = correlation.with_request(sts2_harness::RequestId::new(1).expect("request"));
    }
    if change == 12 {
        correlation = correlation.with_action(ActionId::new(1).expect("action"));
    }
    DecisionRecord::new(
        RecordId::new(id(13)).expect("record"),
        0,
        correlation,
        if change == 15 {
            DecisionRecordKind::Acceptance
        } else {
            DecisionRecordKind::Settlement
        },
        if change == 1 {
            EvidenceStatus::Unverified
        } else {
            EvidenceStatus::Confirmed
        },
        (change != 19).then_some(id(2)),
        (change != 17).then(|| if change == 3 { "state-2" } else { "state-1" }.into()),
        (change != 18).then(|| {
            if change == 4 {
                "operation-2"
            } else {
                "operation-1"
            }
            .into()
        }),
        Some(ActionId::new(id(5)).expect("action")),
        model,
        DecisionPayload::from_json(
            json!({"status": if change == 14 { "unknown" } else { "settled" }}),
        )
        .expect("safe fixture payload"),
    )
    .expect("valid record")
}

fn fingerprint(record: DecisionRecord) -> u64 {
    let trajectory = record.correlation().trajectory_id();
    let report = DecisionReplay::evaluate(&DecisionReplayRequest::new(trajectory, vec![record]));
    assert!(report.divergence().is_none());
    report.fingerprint()
}

#[test]
fn replay_fingerprint_binds_evidence_and_every_record_identity() {
    let baseline = fingerprint(lineage_record(0));
    for change in 1..=19 {
        assert_ne!(
            baseline,
            fingerprint(lineage_record(change)),
            "field case {change}"
        );
    }
    assert_eq!(baseline, fingerprint(lineage_record(0)));
}

#[test]
fn replay_fingerprint_delimits_adjacent_variable_width_fields() {
    let make = |state: &str, operation: &str| {
        DecisionRecord::new(
            RecordId::new(1).expect("record"),
            0,
            correlation(TrajectoryId::new(1).expect("trajectory")),
            DecisionRecordKind::Settlement,
            EvidenceStatus::Confirmed,
            Some(1),
            Some(state.into()),
            Some(operation.into()),
            None,
            None,
            DecisionPayload::from_json(json!({"status":"settled"})).expect("safe payload"),
        )
        .expect("valid record")
    };
    assert_ne!(fingerprint(make("a", "bc")), fingerprint(make("ab", "c")));
}
