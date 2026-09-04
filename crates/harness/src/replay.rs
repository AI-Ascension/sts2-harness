// SPDX-License-Identifier: MIT

use crate::error::PortError;
use crate::decision_records::{DecisionRecord, DecisionRecordKind};
use crate::identity::TrajectoryId;
use crate::records::{Record, RecordKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRequest {
    trajectory_id: TrajectoryId,
    records: Vec<Record>,
}

impl ReplayRequest {
    #[must_use]
    pub fn new(trajectory_id: TrajectoryId, records: Vec<Record>) -> Self {
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
    pub fn records(&self) -> &[Record] {
        &self.records
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Divergence {
    index: usize,
    expected_sequence: u64,
    actual_sequence: u64,
    reason: String,
}

impl Divergence {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayReport {
    trajectory_id: TrajectoryId,
    records_replayed: usize,
    last_sequence: Option<u64>,
    fingerprint: u64,
    divergence: Option<Divergence>,
}

impl ReplayReport {
    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    #[must_use]
    pub const fn records_replayed(&self) -> usize {
        self.records_replayed
    }

    #[must_use]
    pub const fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    #[must_use]
    pub fn divergence(&self) -> Option<&Divergence> {
        self.divergence.as_ref()
    }
}

pub trait ReplayPort {
    fn replay(&mut self, request: ReplayRequest) -> Result<ReplayReport, PortError>;

    fn close(&mut self) -> Result<(), PortError>;
}

pub struct DeterministicReplay;

impl DeterministicReplay {
    #[must_use]
    pub fn evaluate(request: &ReplayRequest) -> ReplayReport {
        let mut fingerprint = 14_695_981_039_346_656_037_u64;
        let mut last_sequence = None;
        let mut divergence = None;
        let mut records_replayed = 0;

        for (index, record) in request.records().iter().enumerate() {
            let expected_sequence = index as u64;
            if record.sequence() != expected_sequence {
                divergence = Some(Divergence {
                    index,
                    expected_sequence,
                    actual_sequence: record.sequence(),
                    reason: "record sequence is not contiguous".to_owned(),
                });
                break;
            }
            if record.trajectory_id() != request.trajectory_id()
                || record.correlation().trajectory_id() != request.trajectory_id()
            {
                divergence = Some(Divergence {
                    index,
                    expected_sequence,
                    actual_sequence: record.sequence(),
                    reason: "record belongs to another trajectory".to_owned(),
                });
                break;
            }
            fingerprint = mix(fingerprint, record.record_id().get().to_le_bytes());
            fingerprint = mix(fingerprint, [kind_code(record.kind())]);
            fingerprint = mix(fingerprint, record.payload().as_bytes().iter().copied());
            last_sequence = Some(record.sequence());
            records_replayed += 1;
        }

        ReplayReport {
            trajectory_id: request.trajectory_id(),
            records_replayed,
            last_sequence,
            fingerprint,
            divergence,
        }
    }
}

fn kind_code(kind: RecordKind) -> u8 {
    match kind {
        RecordKind::Observation => 1,
        RecordKind::ActionRequested => 2,
        RecordKind::ActionAccepted => 3,
        RecordKind::ActionCompleted => 4,
        RecordKind::ModelRequested => 5,
        RecordKind::ModelCompleted => 6,
        RecordKind::Marker => 7,
    }
}

fn mix(mut value: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    for byte in bytes {
        value ^= u64::from(byte);
        value = value.wrapping_mul(1_099_511_628_211);
    }
    value
}

/// Request for replaying the typed, sanitized decision-memory lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReplayRequest {
    trajectory_id: TrajectoryId,
    records: Vec<DecisionRecord>,
}

impl DecisionReplayRequest {
    #[must_use]
    pub fn new(trajectory_id: TrajectoryId, records: Vec<DecisionRecord>) -> Self {
        Self { trajectory_id, records }
    }

    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId { self.trajectory_id }

    #[must_use]
    pub fn records(&self) -> &[DecisionRecord] { &self.records }
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
    pub const fn index(&self) -> usize { self.index }

    #[must_use]
    pub const fn expected_sequence(&self) -> u64 { self.expected_sequence }

    #[must_use]
    pub const fn actual_sequence(&self) -> u64 { self.actual_sequence }

    #[must_use]
    pub fn reason(&self) -> &str { &self.reason }
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
    #[must_use] pub const fn trajectory_id(&self) -> TrajectoryId { self.trajectory_id }
    #[must_use] pub const fn records_replayed(&self) -> usize { self.records_replayed }
    #[must_use] pub const fn observations(&self) -> usize { self.observations }
    #[must_use] pub const fn requests(&self) -> usize { self.requests }
    #[must_use] pub const fn acceptances(&self) -> usize { self.acceptances }
    #[must_use] pub const fn settlements(&self) -> usize { self.settlements }
    #[must_use] pub const fn recoveries(&self) -> usize { self.recoveries }
    #[must_use] pub const fn estimates(&self) -> usize { self.estimates }
    #[must_use] pub const fn unavailable(&self) -> usize { self.unavailable }
    #[must_use] pub const fn fingerprint(&self) -> u64 { self.fingerprint }
    #[must_use] pub fn divergence(&self) -> Option<&DecisionReplayDivergence> { self.divergence.as_ref() }
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
            fingerprint: 14_695_981_039_346_656_037_u64,
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
            report.fingerprint = mix(report.fingerprint, record.record_id().get().to_le_bytes());
            report.fingerprint = mix(report.fingerprint, record.kind().as_str().bytes());
            report.fingerprint = mix(report.fingerprint, record.payload().as_bytes().iter().copied());
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
