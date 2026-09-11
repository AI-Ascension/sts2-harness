// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MEMORY_ENTRY_SCHEMA: &str = "ascension.context-memory.entry.v1";
pub const MEMORY_QUERY_SCHEMA: &str = "ascension.context-memory.query.v1";
pub const MEMORY_RETRIEVAL_SCHEMA: &str = "ascension.context-memory.retrieval.v1";
pub const MEMORY_PROPOSAL_SCHEMA: &str = "ascension.context-memory.proposal.v1";
pub const MEMORY_REVIEW_SCHEMA: &str = "ascension.context-memory.review.v1";
pub const MEMORY_JOB_SCHEMA: &str = "ascension.context-memory.summary-job.v1";
pub const MEMORY_SELECTION_SCHEMA: &str = "ascension.context-memory.selection.v1";
pub const MEMORY_APPROVAL_SCHEMA: &str = "ascension.context-memory.approval.v1";
pub const MEMORY_POLICY_SCHEMA: &str = "ascension.context-memory.policy.v1";
pub const MEMORY_REVOCATION_SCHEMA: &str = "ascension.context-memory.revocation.v1";
pub const MEMORY_CAPABILITIES_SCHEMA: &str = "ascension.context-memory.capabilities.v1";

pub const MAX_ENTRIES_PER_RUN: usize = 10_000;
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_CORPUS_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SOURCES_PER_JOB: usize = 16;
pub const MAX_JOB_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_SUMMARY_OUTPUT_BYTES: usize = 8 * 1024;
pub const MAX_QUERY_BYTES: usize = 4 * 1024;
pub const MAX_CANDIDATES: usize = 64;
pub const MAX_RESULTS: usize = 16;
pub const MAX_SELECTED: usize = 32;
pub const MAX_OPTIONAL_BYTES: usize = 8 * 1024;
pub const MAX_LINEAGE_DEPTH: u8 = 2;
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[must_use]
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryScope {
    pub project_id: String,
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
}

impl MemoryScope {
    pub fn new(
        project_id: impl Into<String>,
        run_id: impl Into<String>,
        episode_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            run_id: run_id.into(),
            episode_id: episode_id.into(),
            agent_id: agent_id.into(),
        }
    }

    fn valid(&self) -> bool {
        valid_id(&self.project_id)
            && valid_id(&self.run_id)
            && valid_id(&self.episode_id)
            && valid_id(&self.agent_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRef {
    pub entry_id: String,
    pub version: u64,
    pub sha256: String,
}

impl MemoryRef {
    pub fn new(entry_id: impl Into<String>, version: u64, sha256: impl Into<String>) -> Self {
        Self {
            entry_id: entry_id.into(),
            version,
            sha256: sha256.into(),
        }
    }

    fn valid(&self) -> bool {
        valid_id(&self.entry_id) && self.version > 0 && valid_digest(&self.sha256)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    HistoricalObservation,
    SettledAction,
    UnresolvedMarker,
    OperatorNote,
    StaticReference,
    MapBundle,
    Extract,
    Summary,
}

impl MemoryKind {
    fn derived(self) -> bool {
        matches!(self, Self::Extract | Self::Summary | Self::MapBundle)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Observed,
    Reported,
    Derived,
    Inferred,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Admitted,
    Stale,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryParent {
    pub entry_id: String,
    pub version: u64,
    pub sha256: String,
}

impl From<&MemoryRef> for MemoryParent {
    fn from(reference: &MemoryRef) -> Self {
        Self {
            entry_id: reference.entry_id.clone(),
            version: reference.version,
            sha256: reference.sha256.clone(),
        }
    }
}

impl MemoryParent {
    fn reference(&self) -> MemoryRef {
        MemoryRef::new(&self.entry_id, self.version, &self.sha256)
    }
}

/// Metadata is wire compatible with `entry.v1`; content and protection are private store facts
/// and are intentionally omitted from serialization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntry {
    pub schema: String,
    pub entry_id: String,
    pub version: u64,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub source_record_id: String,
    pub kind: MemoryKind,
    pub evidence: EvidenceStatus,
    pub authority: String,
    pub observed_seq: u64,
    pub admitted_seq: u64,
    pub corpus_generation: u64,
    pub content_ref: String,
    pub sha256: String,
    pub byte_length: usize,
    pub parents: Vec<MemoryParent>,
    pub lineage_depth: u8,
    pub status: EntryStatus,
    pub created_at: String,
    pub expires_at: String,
    pub game_profile: String,
    #[serde(skip)]
    content: Vec<u8>,
    #[serde(skip)]
    protected: bool,
}

impl MemoryEntry {
    pub fn reference(&self) -> MemoryRef {
        MemoryRef::new(&self.entry_id, self.version, &self.sha256)
    }

    pub fn is_protected(&self) -> bool {
        self.protected
    }

    pub fn validate_contract(&self) -> Result<(), MemoryError> {
        self.validate()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: MemoryScope,
        entry_id: impl Into<String>,
        source_record_id: impl Into<String>,
        kind: MemoryKind,
        evidence: EvidenceStatus,
        branch_id: impl Into<String>,
        content_ref: impl Into<String>,
        content: Vec<u8>,
        observed_seq: u64,
        admitted_seq: u64,
        corpus_generation: u64,
        created_at: impl Into<String>,
        expires_at: impl Into<String>,
        game_profile: impl Into<String>,
        protected: bool,
    ) -> Self {
        let digest = sha256_hex(&content);
        Self {
            schema: MEMORY_ENTRY_SCHEMA.to_owned(),
            entry_id: entry_id.into(),
            version: 1,
            scope,
            branch_id: branch_id.into(),
            source_record_id: source_record_id.into(),
            kind,
            evidence,
            authority: "historical_data_only".to_owned(),
            observed_seq,
            admitted_seq,
            corpus_generation,
            content_ref: content_ref.into(),
            sha256: digest,
            byte_length: content.len(),
            parents: Vec::new(),
            lineage_depth: 0,
            status: EntryStatus::Admitted,
            created_at: created_at.into(),
            expires_at: expires_at.into(),
            game_profile: game_profile.into(),
            content,
            protected,
        }
    }

    fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_ENTRY_SCHEMA
            || !valid_id(&self.entry_id)
            || self.version == 0
            || !self.scope.valid()
            || !valid_id(&self.branch_id)
            || !valid_id(&self.source_record_id)
            || !valid_id(&self.content_ref)
            || self.authority != "historical_data_only"
            || self.observed_seq > 9_007_199_254_740_991
            || self.admitted_seq > 9_007_199_254_740_991
            || self.corpus_generation == 0
            || self.corpus_generation > 9_007_199_254_740_991
            || self.byte_length == 0
            || self.byte_length > MAX_SOURCE_BYTES
            || self.content.len() != self.byte_length
            || sha256_hex(&self.content) != self.sha256
            || !valid_digest(&self.sha256)
            || self.lineage_depth > MAX_LINEAGE_DEPTH
            || self.parents.len() > 16
            || self.parents.iter().map(MemoryParent::reference).collect::<BTreeSet<_>>().len()
                != self.parents.len()
            || self.status != EntryStatus::Admitted
            || !valid_timestamp(&self.created_at)
            || !valid_timestamp(&self.expires_at)
            || self.expires_at.as_str() <= self.created_at.as_str()
            || self.game_profile.is_empty()
        {
            return Err(MemoryError::InvalidEntry);
        }
        if self.kind.derived() && self.evidence == EvidenceStatus::Observed {
            return Err(MemoryError::AuthorityPromotion);
        }
        if self.parents.iter().any(|parent| {
            let reference = parent.reference();
            !reference.valid() || reference.entry_id == self.entry_id
        }) {
            return Err(MemoryError::LineageCycle);
        }
        if self.admitted_seq < self.observed_seq && self.kind.derived() {
            return Err(MemoryError::InvalidEntry);
        }
        Ok(())
    }

    fn active_at(&self, now: &str) -> bool {
        self.status == EntryStatus::Admitted && self.expires_at.as_str() > now
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationFailpoint {
    BeforeMetadata,
    AfterMetadata,
    BeforeGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    Inserted,
    Duplicate,
}
