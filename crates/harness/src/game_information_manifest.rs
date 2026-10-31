// SPDX-License-Identifier: MIT
//! Complete, separately pinned content-catalog reads. No producer or host I/O lives here.
use super::*;
use serde_json::json;

#[path = "game_information_manifest_agent.rs"]
mod agent;
#[path = "game_information_manifest_retention.rs"]
mod retention;
#[path = "game_information_manifest_validation.rs"]
pub(super) mod validation;
pub(super) use agent::{handle_agent, replay_agent};
pub(super) use retention::publish_failure;

pub use sts2_protocol::{
    GAME_INFORMATION_CONTENT_MANIFEST_V1_PROTOCOL_VERSION as PROFILE,
    GAME_INFORMATION_CONTENT_MANIFEST_V1_SCHEMA_DIGEST as SCHEMA_DIGEST,
};

/// The current gateway/MCP route admits 128 KiB, below the protocol's 16 MiB ceiling.
pub const MAX_RESPONSE_BYTES: usize = 128 * 1024;
pub const MAX_RECORDS: usize = 32;
pub const RECORD_SCHEMA: &str = "ascension.game-information-content-manifest-record.v1";

/// Complete response identity and ordered encrypted-memory chunks, never game-state authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentManifestRecord {
    pub schema: String,
    pub binding: LookupBinding,
    pub operation_id: String,
    pub correlation_id: Option<String>,
    pub source_sha256: Option<String>,
    pub sources: Vec<MemoryRef>,
    pub source_bytes: usize,
    pub view_sha256: Option<String>,
    pub error: Option<LookupError>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentManifestDelivery {
    pub record: ContentManifestRecord,
    pub data: Value,
}

/// A recorded outcome is distinct from a fresh failure to authenticate or reconstruct replay.
pub(super) enum ManifestReplayOutcome {
    Delivery(Box<ContentManifestDelivery>),
    RecordedFailure {
        error: LookupError,
        retention: MemoryRef,
    },
}

impl LookupSession {
    pub fn manifest_records(&self) -> &[ContentManifestRecord] {
        &self.manifest_records
    }

    /// Admit one complete catalog read under the selected lookup owner and memory policy.
    /// A typed producer refusal is retained and delivered intact as data, never a partial catalog.
    pub fn content_manifest_port<P: LookupMcpPort>(
        &mut self,
        binding: &LookupBinding,
        operation: &str,
        corpus: &mut MemoryCorpus,
        port: &mut P,
    ) -> Result<ContentManifestDelivery, LookupError> {
        self.admit_manifest(binding, operation, corpus)?;
        let mut owner = self.binding.clone();
        // Static catalog evidence cannot carry or restore a live observation attestation.
        owner.snapshot = None;
        let mut record = ContentManifestRecord {
            schema: RECORD_SCHEMA.to_owned(),
            binding: owner,
            operation_id: operation.to_owned(),
            correlation_id: None,
            source_sha256: None,
            sources: Vec::new(),
            source_bytes: 0,
            view_sha256: None,
            error: None,
        };
        let outcome = port
            .information_content_manifest()
            .and_then(|(correlation, bytes)| {
                let value = validation::response(&bytes, &correlation, &self.binding)?;
                record.correlation_id = Some(correlation);
                record.source_sha256 = Some(crate::sha256_hex(&bytes));
                record.source_bytes = bytes.len();
                // Stage all chunks and the model view before publishing any source into this corpus.
                let (staged, sources) = retention::stage(self, &record, &bytes, corpus)?;
                record.sources = sources;
                // Duplicate entries keep their original retention window and metadata. A new
                // session must authenticate their current readability before delivering success.
                retention::retained(&record, &staged, &self.now)?;
                let delivery = self.deliver_manifest(record.clone(), &value)?;
                *corpus = staged;
                Ok(delivery)
            });
        match outcome {
            Ok(delivery) => {
                self.manifest_records.push(delivery.record.clone());
                Ok(delivery)
            }
            Err(error) => {
                record.correlation_id = None;
                record.source_sha256 = None;
                record.sources.clear();
                record.source_bytes = 0;
                record.error = Some(error.clone());
                self.manifest_records.push(record);
                Err(error)
            }
        }
    }

    fn admit_manifest(
        &self,
        binding: &LookupBinding,
        operation: &str,
        corpus: &MemoryCorpus,
    ) -> Result<(), LookupError> {
        self.policy
            .validate(corpus)
            .map_err(|_| LookupError::Scope)?;
        if binding != &self.binding || !validation::operation(operation) {
            return Err(LookupError::Scope);
        }
        if self.capabilities.is_none() {
            return Err(LookupError::MissingCapability);
        }
        if self.manifest_records.len() >= MAX_RECORDS {
            return Err(LookupError::Bounds);
        }
        if self
            .manifest_records
            .iter()
            .any(|record| record.operation_id == operation)
        {
            return Err(LookupError::Divergence);
        }
        Ok(())
    }

    /// Replay only complete retained bytes, with no transport or new capability negotiation.
    pub fn replay_content_manifest(
        &self,
        record: &ContentManifestRecord,
        corpus: &MemoryCorpus,
    ) -> Result<ContentManifestDelivery, LookupError> {
        match self.replay_manifest_outcome(record, corpus)? {
            ManifestReplayOutcome::Delivery(delivery) => Ok(*delivery),
            ManifestReplayOutcome::RecordedFailure { error, .. } => Err(error),
        }
    }

    pub(super) fn replay_manifest_outcome(
        &self,
        record: &ContentManifestRecord,
        corpus: &MemoryCorpus,
    ) -> Result<ManifestReplayOutcome, LookupError> {
        validation::record(record, &self.binding)?;
        self.policy
            .validate(corpus)
            .map_err(|_| LookupError::Scope)?;
        if corpus.scope() != &self.binding.scope {
            return Err(LookupError::Scope);
        }
        if let Some(error) = &record.error {
            let retention = retention::retained_failure(record, corpus, &self.now)?;
            return Ok(ManifestReplayOutcome::RecordedFailure {
                error: error.clone(),
                retention,
            });
        }
        let bytes = retention::retained(record, corpus, &self.now)?;
        let correlation = record
            .correlation_id
            .as_deref()
            .ok_or(LookupError::Divergence)?;
        let value = validation::response(&bytes, correlation, &self.binding)?;
        let delivery = self.deliver_manifest(record.clone(), &value)?;
        if delivery.record.view_sha256 != record.view_sha256 {
            return Err(LookupError::Divergence);
        }
        Ok(ManifestReplayOutcome::Delivery(Box::new(delivery)))
    }

    /// Exact bounded bytes; callers use the returned length as the next offset, including UTF-8.
    /// A recorded failure has no source bytes and returns `MissingRetention` after validation.
    pub fn read_retained_manifest(
        &self,
        record: &ContentManifestRecord,
        corpus: &MemoryCorpus,
        offset: usize,
    ) -> Result<Vec<u8>, LookupError> {
        if matches!(
            self.replay_manifest_outcome(record, corpus)?,
            ManifestReplayOutcome::RecordedFailure { .. }
        ) {
            return Err(LookupError::MissingRetention);
        }
        let bytes = retention::retained(record, corpus, &self.now)?;
        if offset > bytes.len() {
            return Err(LookupError::Bounds);
        }
        let end = offset
            .saturating_add(self.policy.optional_byte_budget)
            .min(bytes.len());
        Ok(bytes[offset..end].to_vec())
    }

    fn deliver_manifest(
        &self,
        mut record: ContentManifestRecord,
        response: &Value,
    ) -> Result<ContentManifestDelivery, LookupError> {
        let full = json!({"authority":"untrusted_game_information_data","protocol_version":PROFILE,
            "schema_digest":SCHEMA_DIGEST,"source_sha256":record.source_sha256,
            "manifest":response["manifest"],"error":response["error"]});
        let data = if serde_json::to_vec(&full)
            .map_err(|_| LookupError::Invalid)?
            .len()
            <= self.policy.optional_byte_budget
        {
            full
        } else {
            json!({"authority":"untrusted_game_information_data","protocol_version":PROFILE,
                "schema_digest":SCHEMA_DIGEST,"source_sha256":record.source_sha256,
                "delivery":"retained","byte_length":record.source_bytes})
        };
        let bytes = serde_json::to_vec(&data).map_err(|_| LookupError::Invalid)?;
        if bytes.len() > self.policy.optional_byte_budget {
            return Err(LookupError::Bounds);
        }
        record.view_sha256 = Some(crate::sha256_hex(bytes));
        Ok(ContentManifestDelivery { record, data })
    }
}
