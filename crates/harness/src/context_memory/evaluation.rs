// SPDX-License-Identifier: MIT

// Evaluation labels live in a separate partition.  Only the bounded lane configuration is
// serializable; product retrieval receives a scope/cutoff check and never the evaluator labels.

pub const MEMORY_EVALUATION_PARTITION_SCHEMA: &str =
    "ascension.context-memory.evaluation-partition.v1";
pub const MAX_HELD_OUT_CASES: usize = 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationLaneConfig {
    pub schema: String,
    pub lane: EvaluationLane,
    pub scope: MemoryScope,
    pub source_cutoff: u64,
    pub configuration_sha256: String,
    pub held_out: bool,
    pub case_count: usize,
}

#[derive(Clone, Eq, PartialEq)]
pub struct HeldOutEvaluation {
    scope: MemoryScope,
    source_cutoff: u64,
    labels: BTreeMap<String, Vec<String>>,
}

impl HeldOutEvaluation {
    pub fn new(scope: MemoryScope, source_cutoff: u64) -> Result<Self, MemoryError> {
        if !scope.valid() || source_cutoff > 9_007_199_254_740_991 {
            return Err(MemoryError::InvalidScope);
        }
        Ok(Self {
            scope,
            source_cutoff,
            labels: BTreeMap::new(),
        })
    }

    pub fn record_private_label(
        &mut self,
        case_id: impl Into<String>,
        labels: Vec<String>,
    ) -> Result<(), MemoryError> {
        let case_id = case_id.into();
        if !valid_id(&case_id)
            || labels.is_empty()
            || labels.len() > 16
            || labels.iter().any(|label| label.is_empty() || label.len() > 128)
        {
            return Err(MemoryError::InvalidQuery);
        }
        if self.labels.contains_key(&case_id) {
            return Err(MemoryError::Conflict);
        }
        if self.labels.len() >= MAX_HELD_OUT_CASES {
            return Err(MemoryError::Capacity);
        }
        self.labels.insert(case_id, labels);
        Ok(())
    }

    pub fn authorize_lane(
        &self,
        scope: &MemoryScope,
        source_cutoff: u64,
    ) -> Result<(), MemoryError> {
        if &self.scope != scope || source_cutoff != self.source_cutoff {
            return Err(MemoryError::PermissionDenied);
        }
        Ok(())
    }

    pub fn lane_config(
        &self,
        lane: EvaluationLane,
        configuration_sha256: impl Into<String>,
    ) -> Result<EvaluationLaneConfig, MemoryError> {
        let configuration_sha256 = configuration_sha256.into();
        if !valid_digest(&configuration_sha256) {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(EvaluationLaneConfig {
            schema: MEMORY_EVALUATION_PARTITION_SCHEMA.to_owned(),
            lane,
            scope: self.scope.clone(),
            source_cutoff: self.source_cutoff,
            configuration_sha256,
            held_out: true,
            case_count: self.labels.len(),
        })
    }

    pub fn case_count(&self) -> usize {
        self.labels.len()
    }
}
