// SPDX-License-Identifier: MIT

use crate::error::PortError;
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
