// SPDX-License-Identifier: MIT

// Map/artifact, lineage, evaluation and migration records kept beside the memory policy.

pub const MEMORY_MAP_BUNDLE_SCHEMA: &str = "ascension.context-memory.map-bundle.v1";
pub const MEMORY_LINEAGE_SCHEMA: &str = "ascension.context-memory.lineage.v1";
pub const MEMORY_EVALUATION_SCHEMA: &str = "ascension.context-memory.evaluation.v1";
pub const MEMORY_MIGRATION_SCHEMA: &str = "ascension.context-memory.migration.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapAttachmentKind {
    Graph,
    Analysis,
    Image,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAttachment {
    pub kind: MapAttachmentKind,
    pub artifact_id: String,
    pub generation: u64,
    pub sha256: String,
    pub byte_length: usize,
    pub mime: Option<String>,
    #[serde(skip)]
    bytes: Vec<u8>,
}

impl MapAttachment {
    pub fn new(
        kind: MapAttachmentKind,
        artifact_id: impl Into<String>,
        generation: u64,
        bytes: Vec<u8>,
        mime: Option<String>,
    ) -> Self {
        Self {
            kind,
            artifact_id: artifact_id.into(),
            generation,
            sha256: sha256_hex(&bytes),
            byte_length: bytes.len(),
            mime,
            bytes,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapArtifactBundle {
    pub schema: String,
    pub bundle_id: String,
    pub scope: MemoryScope,
    pub generation: u64,
    pub graph: MapAttachment,
    pub analysis: MapAttachment,
    pub image: Option<MapAttachment>,
    pub image_requested: bool,
    pub graph_only_reason: Option<String>,
}

impl MapArtifactBundle {
    pub fn validate(&self, scope: &MemoryScope, generation: u64) -> Result<(), MemoryError> {
        if self.schema != MEMORY_MAP_BUNDLE_SCHEMA
            || !valid_id(&self.bundle_id)
            || self.scope != *scope
            || self.generation != generation
            || self.graph.kind != MapAttachmentKind::Graph
            || self.analysis.kind != MapAttachmentKind::Analysis
            || !valid_id(&self.graph.artifact_id)
            || !valid_id(&self.analysis.artifact_id)
            || self.graph.artifact_id == self.analysis.artifact_id
            || self.graph.generation != generation
            || self.analysis.generation != generation
            || self.graph.byte_length == 0
            || self.analysis.byte_length == 0
            || self.graph.byte_length > MAX_SOURCE_BYTES
            || self.analysis.byte_length > MAX_SOURCE_BYTES
            || self.graph.bytes.len() != self.graph.byte_length
            || self.analysis.bytes.len() != self.analysis.byte_length
            || sha256_hex(&self.graph.bytes) != self.graph.sha256
            || sha256_hex(&self.analysis.bytes) != self.analysis.sha256
            || self.graph.mime.as_deref().is_some_and(|mime| mime.len() > 128)
            || self.analysis.mime.as_deref().is_some_and(|mime| mime.len() > 128)
        {
            return Err(MemoryError::InvalidEntry);
        }
        match (&self.image, self.image_requested) {
            (Some(image), true) => {
                if image.kind != MapAttachmentKind::Image
                    || !valid_id(&image.artifact_id)
                    || image.artifact_id == self.graph.artifact_id
                    || image.artifact_id == self.analysis.artifact_id
                    || image.generation != generation
                    || image.byte_length == 0
                    || image.byte_length > MAX_SOURCE_BYTES
                    || image.bytes.len() != image.byte_length
                    || sha256_hex(&image.bytes) != image.sha256
                    || image.mime.as_deref().is_none_or(|mime| mime.len() > 128)
                {
                    return Err(MemoryError::InvalidEntry);
                }
            }
            (Some(_), false) => return Err(MemoryError::PermissionDenied),
            (None, true) => return Err(MemoryError::Unsupported),
            (None, false) if self.graph_only_reason.as_deref().is_none_or(str::is_empty) => {
                return Err(MemoryError::InvalidEntry)
            }
            (None, false) => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineageManifest {
    pub schema: String,
    pub selection_id: String,
    pub policy_id: String,
    pub phase2_revision_id: String,
    pub snapshot_id: String,
    pub provider_attempt_id: Option<String>,
    pub plan_id: Option<String>,
    pub action_id: Option<String>,
    pub source_manifest_sha256: String,
}

impl LineageManifest {
    pub fn validate(&self) -> Result<(), MemoryError> {
        let ids = [
            self.selection_id.as_str(),
            self.policy_id.as_str(),
            self.phase2_revision_id.as_str(),
            self.snapshot_id.as_str(),
        ];
        if self.schema != MEMORY_LINEAGE_SCHEMA
            || ids.iter().any(|id| !valid_id(id))
            || !valid_digest(&self.source_manifest_sha256)
            || self.provider_attempt_id.as_deref().is_some_and(|id| !valid_id(id))
            || self.plan_id.as_deref().is_some_and(|id| !valid_id(id))
            || self.action_id.as_deref().is_some_and(|id| !valid_id(id))
        {
            return Err(MemoryError::InvalidQuery);
        }
        let mut nonempty = ids.iter().copied().collect::<BTreeSet<_>>();
        for id in [
            self.provider_attempt_id.as_deref(),
            self.plan_id.as_deref(),
            self.action_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !nonempty.insert(id) {
                return Err(MemoryError::Conflict);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageMeasurement {
    pub summary_calls: u32,
    pub gameplay_calls: u32,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub cached_input_bytes: u64,
    pub maintenance_bytes: u64,
    pub unknown_calls: u32,
}

impl UsageMeasurement {
    pub fn public_snapshot(&self) -> Value {
        serde_json::json!({
            "schema": "ascension.context-memory.usage.v1",
            "summary_calls": self.summary_calls,
            "gameplay_calls": self.gameplay_calls,
            "input_bytes": self.input_bytes,
            "output_bytes": self.output_bytes,
            "cached_input_bytes": self.cached_input_bytes,
            "maintenance_bytes": self.maintenance_bytes,
            "unknown_calls": self.unknown_calls,
            "raw_query": Value::Null,
            "raw_content": Value::Null,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationLane {
    Retrieval,
    Extraction,
    SummarySupport,
    SelectionSafety,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReport {
    pub schema: String,
    pub lane: EvaluationLane,
    pub denominator: usize,
    pub misses: usize,
    pub reviewed: usize,
    pub critical_safety_pass: bool,
    pub source_cutoff: u64,
    pub held_out: bool,
    pub quality_gate: String,
}

impl EvaluationReport {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_EVALUATION_SCHEMA
            || self.denominator == 0
            || self.misses > self.denominator
            || self.reviewed > self.denominator
            || self.source_cutoff > 9_007_199_254_740_991
            || !valid_id(&self.quality_gate)
            || (self.held_out && self.reviewed != 0)
        {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(())
    }

    pub fn recall(&self) -> f64 {
        (self.denominator.saturating_sub(self.misses)) as f64 / self.denominator as f64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPhase {
    Prepared,
    Applying,
    Complete,
    RolledBack,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationJournal {
    pub schema: String,
    pub migration_id: String,
    pub from_version: u16,
    pub to_version: u16,
    pub phase: MigrationPhase,
    pub checkpoint: u64,
    pub tombstone_epoch: u64,
}

impl MigrationJournal {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_MIGRATION_SCHEMA
            || !valid_id(&self.migration_id)
            || self.from_version == 0
            || self.to_version <= self.from_version
        {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DowngradeFence {
    required_memory_version: u16,
    active_policy_revision: Option<String>,
}

impl DowngradeFence {
    pub fn new(required_memory_version: u16, active_policy_revision: Option<String>) -> Self {
        Self {
            required_memory_version,
            active_policy_revision,
        }
    }

    pub fn allow_reader(&self, reader_version: u16) -> Result<(), MemoryError> {
        if reader_version < self.required_memory_version {
            return Err(MemoryError::Unsupported);
        }
        Ok(())
    }

    pub fn allow_resume(&self, reader_version: u16, revision: &str) -> Result<(), MemoryError> {
        self.allow_reader(reader_version)?;
        if self.active_policy_revision.as_deref() != Some(revision) {
            return Err(MemoryError::StaleApproval);
        }
        Ok(())
    }
}
