// SPDX-License-Identifier: MIT

use super::mix;
use crate::decision_records::{DecisionRecord, DecisionRecordKind};
use crate::identity::TrajectoryId;

/// Request for replaying the typed, sanitized decision-memory lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReplayRequest {
    trajectory_id: TrajectoryId,
    records: Vec<DecisionRecord>,
}

impl DecisionReplayRequest {
    #[must_use]
    pub fn new(trajectory_id: TrajectoryId, records: Vec<DecisionRecord>) -> Self {
        Self {
            trajectory_id,
            records,
        }
    }

    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    #[must_use]
    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
    }
}

/// Typed replay divergence. The first mismatch stops replay and remains visible to evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReplayDivergence {
    index: usize,
    expected_sequence: u64,
    actual_sequence: u64,
    reason: String,
}

impl DecisionReplayDivergence {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }

    #[must_use]
    pub const fn actual_sequence(&self) -> u64 {
        self.actual_sequence
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Replay counts preserve the difference between accepted, settled, recovery, and unavailable facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReplayReport {
    trajectory_id: TrajectoryId,
    records_replayed: usize,
    observations: usize,
    requests: usize,
    acceptances: usize,
    settlements: usize,
    recoveries: usize,
    estimates: usize,
    unavailable: usize,
    fingerprint: u64,
    divergence: Option<DecisionReplayDivergence>,
}

impl DecisionReplayReport {
    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }
    #[must_use]
    pub const fn records_replayed(&self) -> usize {
        self.records_replayed
    }
    #[must_use]
    pub const fn observations(&self) -> usize {
        self.observations
    }
    #[must_use]
    pub const fn requests(&self) -> usize {
        self.requests
    }
    #[must_use]
    pub const fn acceptances(&self) -> usize {
        self.acceptances
    }
    #[must_use]
    pub const fn settlements(&self) -> usize {
        self.settlements
    }
    #[must_use]
    pub const fn recoveries(&self) -> usize {
        self.recoveries
    }
    #[must_use]
    pub const fn estimates(&self) -> usize {
        self.estimates
    }
    #[must_use]
    pub const fn unavailable(&self) -> usize {
        self.unavailable
    }
    #[must_use]
    /// Non-cryptographic comparison fingerprint of the complete validated record projection.
    /// This is not an artifact integrity digest or proof of the recorded evidence.
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
    #[must_use]
    pub fn divergence(&self) -> Option<&DecisionReplayDivergence> {
        self.divergence.as_ref()
    }
}

/// Deterministic replay implementation for decision records.
pub struct DecisionReplay;

impl DecisionReplay {
    #[must_use]
    pub fn evaluate(request: &DecisionReplayRequest) -> DecisionReplayReport {
        let mut report = DecisionReplayReport {
            trajectory_id: request.trajectory_id,
            records_replayed: 0,
            observations: 0,
            requests: 0,
            acceptances: 0,
            settlements: 0,
            recoveries: 0,
            estimates: 0,
            unavailable: 0,
            fingerprint: mix(
                mix(
                    14_695_981_039_346_656_037_u64,
                    b"decision-replay-v2".iter().copied(),
                ),
                request.trajectory_id.get().to_le_bytes(),
            ),
            divergence: None,
        };
        for (index, record) in request.records.iter().enumerate() {
            let expected_sequence = index as u64;
            if record.sequence() != expected_sequence
                || record.correlation().trajectory_id() != request.trajectory_id
            {
                report.divergence = Some(DecisionReplayDivergence {
                    index,
                    expected_sequence,
                    actual_sequence: record.sequence(),
                    reason: if record.sequence() != expected_sequence {
                        "typed record sequence is not contiguous"
                    } else {
                        "typed record belongs to another trajectory"
                    }
                    .to_owned(),
                });
                break;
            }
            report.fingerprint = decision_fingerprint(report.fingerprint, record);
            match record.kind() {
                DecisionRecordKind::Observation => report.observations += 1,
                DecisionRecordKind::Request => report.requests += 1,
                DecisionRecordKind::Acceptance => report.acceptances += 1,
                DecisionRecordKind::Settlement => report.settlements += 1,
                DecisionRecordKind::Recovery => report.recoveries += 1,
                DecisionRecordKind::Estimate => report.estimates += 1,
                DecisionRecordKind::Unavailable => report.unavailable += 1,
            }
            report.records_replayed += 1;
        }
        report
    }
}

fn decision_fingerprint(mut fingerprint: u64, record: &DecisionRecord) -> u64 {
    let correlation = record.correlation();
    for number in [
        record.record_id().get(),
        record.sequence(),
        correlation.run_id().get(),
        correlation.episode_id().get(),
        correlation.trajectory_id().get(),
        correlation.instance_id().get(),
        correlation.trace_id().get(),
    ] {
        fingerprint = mix(fingerprint, number.to_le_bytes());
    }
    for number in [
        record.generation(),
        record.action_id().map(|id| id.get()),
        record.model_execution_id().map(|id| id.get()),
        correlation.request_id().map(|id| id.get()),
        correlation.action_id().map(|id| id.get()),
        correlation.model_execution_id().map(|id| id.get()),
    ] {
        fingerprint = mix(fingerprint, [u8::from(number.is_some())]);
        if let Some(number) = number {
            fingerprint = mix(fingerprint, number.to_le_bytes());
        }
    }
    for bytes in [
        Some(record.kind().as_str().as_bytes()),
        Some(record.evidence().as_str().as_bytes()),
        record.state_id().map(str::as_bytes),
        record.operation_id().map(str::as_bytes),
        Some(record.payload().as_bytes()),
    ] {
        fingerprint = mix(fingerprint, [u8::from(bytes.is_some())]);
        if let Some(bytes) = bytes {
            fingerprint = mix(fingerprint, (bytes.len() as u64).to_le_bytes());
            fingerprint = mix(fingerprint, bytes.iter().copied());
        }
    }
    fingerprint
}
