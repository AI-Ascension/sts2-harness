// SPDX-License-Identifier: MIT
//! Harness-owned read-only lookup admission, delivery and pinned replay.
//! The caller supplies the existing MCP dispatch port; no host or transport is owned here.

use crate::context_memory::{MemoryCorpus, MemoryPolicy, MemoryRef, MemoryScope};
use crate::game_information_validation as validation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[path = "game_information_admission.rs"]
mod admission;
#[path = "game_information_mcp.rs"]
mod mcp;
#[path = "game_information_records.rs"]
mod records;
pub use mcp::{LookupMcpContext, LookupMcpPort, call_capabilities_mcp, call_lookup_mcp};
#[path = "game_information_agent.rs"]
mod agent;
pub use agent::{
    LookupAgentInput, LookupAgentPort, LookupFeedback, LookupTurn, run_lookup_replay_tool_loop,
    run_lookup_tool_loop,
};
#[path = "game_information_archive.rs"]
mod archive;
pub use archive::LookupArchive;
#[cfg(test)]
#[path = "game_information_tests.rs"]
mod tests;

pub const PROFILE: &str = "game-information-query-v1";
pub const SCHEMA_DIGEST: &str = validation::SCHEMA_DIGEST;
pub const MAX_PAGES: usize = 16;

/// Stable sanitized errors; producer text is never used as error authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LookupError {
    Invalid,
    Scope,
    MissingCapability,
    Reobserve,
    Bounds,
    Transport,
    Retention,
    MissingRetention,
    Divergence,
    Producer(ProducerCode),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProducerCode {
    UnknownKind,
    UnknownId,
    AmbiguousId,
    UnsupportedFilter,
    UnsupportedProjection,
    UnsupportedVersion,
    DeniedScope,
    StaleSnapshot,
    StaleCursor,
    ResultLimitExceeded,
    MissingCapability,
    UnsupportedField,
    InvalidIdentity,
    InvalidBounds,
    MixedGeneration,
    Malformed,
    ReadOnlyViolation,
}
impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "game information lookup: {self:?}")
    }
}
impl std::error::Error for LookupError {}
impl From<validation::ValidationError> for LookupError {
    fn from(_: validation::ValidationError) -> Self {
        Self::Invalid
    }
}

/// Owner-supplied scope. This is never decoded from game or model text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupBinding {
    pub scope: MemoryScope,
    pub game_profile: String,
    pub content_manifest_id: String,
    pub locale: String,
    pub authority_epoch: u64,
    pub snapshot: Option<Value>,
}
impl LookupBinding {
    pub(super) fn same_owner(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.game_profile == other.game_profile
            && self.content_manifest_id == other.content_manifest_id
            && self.locale == other.locale
            && self.authority_epoch == other.authority_epoch
    }
}

/// Separate raw source and derived model-view identities. Neither is a game-state digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupRecord {
    pub schema: String,
    pub binding: LookupBinding,
    pub operation_id: String,
    pub page_sequence: usize,
    pub request: Value,
    pub source_sha256: Option<String>,
    pub source: Option<MemoryRef>,
    pub source_bytes: usize,
    pub view_sha256: Option<String>,
    pub error: Option<LookupError>,
}

/// Structured data only; a reference means full retained bytes require explicit bounded reads.
#[derive(Clone, Debug, PartialEq)]
pub struct LookupDelivery {
    pub record: LookupRecord,
    pub data: Value,
}

/// Bounded one-owner session. Invalidating authority clears capabilities and live admission.
pub struct LookupSession {
    binding: LookupBinding,
    policy: MemoryPolicy,
    capabilities: Option<Value>,
    records: Vec<LookupRecord>,
    pages: BTreeMap<String, (Value, Value, usize)>,
    replay_cursor: usize,
    now: String,
    expires_at: String,
}

impl LookupSession {
    pub fn new(
        binding: LookupBinding,
        policy: MemoryPolicy,
        corpus: &MemoryCorpus,
        now: &str,
        expires_at: &str,
    ) -> Result<Self, LookupError> {
        policy.validate(corpus).map_err(|_| LookupError::Scope)?;
        if binding.scope != policy.scope
            || binding.scope != *corpus.scope()
            || binding.game_profile.is_empty()
            || binding.game_profile.len() > 128
            || binding.content_manifest_id.is_empty()
            || binding.locale.is_empty()
        {
            return Err(LookupError::Scope);
        }
        Ok(Self {
            binding,
            policy,
            capabilities: None,
            records: Vec::new(),
            pages: BTreeMap::new(),
            replay_cursor: 0,
            now: now.to_owned(),
            expires_at: expires_at.to_owned(),
        })
    }

    pub fn binding(&self) -> &LookupBinding {
        &self.binding
    }
    pub fn records(&self) -> &[LookupRecord] {
        &self.records
    }

    /// Supply a freshly correlated capabilities envelope from the selected MCP session.
    pub fn negotiate(&mut self, raw: &[u8], correlation: &str) -> Result<(), LookupError> {
        self.capabilities = None;
        let value = validation::decode_strict(raw)?;
        validation::validate_capabilities(&value)?;
        if value["correlation_id"] != correlation {
            return Err(LookupError::Scope);
        }
        self.capabilities = Some(value["capabilities"].clone());
        Ok(())
    }
    pub fn negotiate_port<P: LookupMcpPort>(&mut self, port: &mut P) -> Result<(), LookupError> {
        self.capabilities = None;
        let (correlation, bytes) = port.information_capabilities()?;
        self.negotiate(&bytes, &correlation)
    }

    /// Revocation/restart/content/profile changes require a new session and fresh negotiation.
    pub fn invalidate(&mut self) {
        self.capabilities = None;
        self.pages.clear();
        self.binding.snapshot = None;
    }

    /// Install an owner-validated current observation snapshot. Outstanding page chains cannot
    /// survive an observation change. This method is not exposed as an agent tool.
    pub fn observe_snapshot(&mut self, snapshot: Value) {
        self.binding.snapshot = Some(snapshot);
        self.pages.clear();
    }

    /// Executes exactly one admitted MCP read, retains complete validated bytes before delivery.
    /// No retry, mutation, implicit pagination, or fallback to a different content build occurs.
    pub fn query<F>(
        &mut self,
        binding: &LookupBinding,
        operation_id: &str,
        request_bytes: &[u8],
        corpus: &mut MemoryCorpus,
        dispatch: F,
    ) -> Result<LookupDelivery, LookupError>
    where
        F: FnOnce(&str, &Value) -> Result<Vec<u8>, LookupError>,
    {
        let request = validation::decode_strict(request_bytes)?;
        self.admit(binding, operation_id, &request, corpus)?;
        let sequence = self.page_sequence(operation_id, &request)?;
        let tool = admission::tool_name(&request)?;
        let result = dispatch(tool, &request);
        match result {
            Ok(raw) => self.accept(operation_id, sequence, request, raw, corpus),
            Err(error) => {
                self.record_error(operation_id, sequence, request, error.clone());
                Err(error)
            }
        }
    }

    /// Supported owner call path: allocate correlation from the existing MCP port, then perform
    /// the same admission, complete-source validation, retention and delivery as `query`.
    pub fn query_port<P: LookupMcpPort>(
        &mut self,
        binding: &LookupBinding,
        operation_id: &str,
        request_bytes: &[u8],
        corpus: &mut MemoryCorpus,
        port: &mut P,
    ) -> Result<LookupDelivery, LookupError> {
        let mut request = validation::decode_strict(request_bytes)?;
        validation::validate_request(&request)?;
        request["correlation_id"] = Value::String(port.information_correlation()?);
        let bytes = serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?;
        self.query(binding, operation_id, &bytes, corpus, |tool, request| {
            port.call_information(tool, request)
        })
    }

    /// Replays only retained source bytes with exact owner, query, profile and content pins.
    /// This API deliberately has no MCP callback.
    pub fn replay(
        &self,
        record: &LookupRecord,
        request: &Value,
        corpus: &MemoryCorpus,
    ) -> Result<LookupDelivery, LookupError> {
        if !record.binding.same_owner(&self.binding)
            || record.request != *request
            || corpus.scope() != &self.binding.scope
            || record.schema != "ascension.game-information-record.v1"
            || request["query"]["binding"]["content_manifest_id"]
                != self.binding.content_manifest_id
            || request["query"]["binding"]["locale"] != self.binding.locale
        {
            return Err(LookupError::Divergence);
        }
        validation::validate_request(request)?;
        if request["query"]["binding"]["mode"] == "live"
            && request["query"]["binding"]["instance_ref"]["run_id"] != record.binding.scope.run_id
        {
            return Err(LookupError::Divergence);
        }
        if let Some(error) = &record.error {
            return Err(error.clone());
        }
        let bytes = records::retained(record, corpus, &self.now)?;
        let value = validation::decode_strict(bytes)?;
        validation::validate_response(request, &value)?;
        if request["query"]["binding"]["mode"] == "live"
            && record.binding.snapshot.as_ref()
                != Some(&request["query"]["binding"]["snapshot_ref"])
        {
            return Err(LookupError::Divergence);
        }
        let delivery = self.deliver(record.clone(), &value)?;
        if delivery.record.view_sha256 != record.view_sha256 {
            return Err(LookupError::Divergence);
        }
        Ok(delivery)
    }

    /// Bounded exact raw-byte chunk of a retained, previously admitted source. Chunks are data.
    pub fn read_retained(
        &self,
        record: &LookupRecord,
        corpus: &MemoryCorpus,
        offset: usize,
    ) -> Result<Vec<u8>, LookupError> {
        if !record.binding.same_owner(&self.binding) || corpus.scope() != &self.binding.scope {
            return Err(LookupError::Scope);
        }
        self.replay(record, &record.request, corpus)?;
        let bytes = records::retained(record, corpus, &self.now)?;
        if offset > bytes.len() {
            return Err(LookupError::Bounds);
        }
        let end = offset
            .saturating_add(self.policy.optional_byte_budget)
            .min(bytes.len());
        Ok(bytes[offset..end].to_vec())
    }
}
