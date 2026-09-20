// SPDX-License-Identifier: MIT

//! The harness-owned agent tool port that serves historical lookup.
//!
//! This is the only seam an agent reads a retained history through. It borrows the store, so it can
//! neither append, fork nor prune, and it never exposes the store's path, so a lookup cannot become a
//! read of an arbitrary artifact: a run, branch or operation identity that is empty, over its byte
//! bound or path-shaped is refused before the store is consulted, and the MCP game adapter holds no
//! store of its own.
//! It is bounded on both sides: a request is refused unless it fits one byte bound and deserializes as
//! exactly this shape, so an unsupported or wrongly typed field and a kind name the closed vocabulary
//! does not carry are refused, and a page is refused, not shortened, over the bound.
//!
//! Every answer discloses what the history does not hold: where capture began, whether history
//! existed before it, every declared gap and retention span, and how many records matched against how
//! many were returned. Destroyed gameplay detail is returned as retention-disclosed, never as empty
//! values, and a re-delivered request is recognised by its operation identity rather than re-served.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::ingest::validate_identity;
use super::query::{SemanticEventListQuery, SemanticHistoryContinuation, page_history};
use super::record::{SemanticCoverageInterval, SemanticEventRecord};
use super::retention::SEMANTIC_RETENTION_LABEL;
use super::roles::SemanticCoverageStatus;
use super::scope::{SEMANTIC_MAX_IDENTITY_BYTES, SemanticCatalogBinding, SemanticHistoryFence};
use super::store::SemanticHistoryStore;
use super::vocabulary::{SemanticEventKind, SemanticEventOrigin};

/// Tool name an agent calls to read a retained history.
pub const SEMANTIC_LOOKUP_TOOL: &str = "semantic_history.lookup";
/// Maximum bytes accepted for one serialized lookup request.
pub const SEMANTIC_MAX_LOOKUP_REQUEST_BYTES: usize = 4096;
/// Maximum retained text weight one lookup result may carry.
pub const SEMANTIC_MAX_LOOKUP_RESULT_BYTES: usize = 64 * 1024;

/// Whether the harness has authorized the historical lookup port for this session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SemanticLookupAuthority {
    /// No lookup is served; the default until the harness grants the port deliberately.
    #[default]
    NotGranted,
    /// The harness has granted the port.
    Granted,
}

/// One bounded lookup request, as the agent tool boundary receives or builds it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticLookupRequest {
    /// Caller operation identity, so a re-delivered request is recognised rather than re-served.
    #[serde(default)]
    pub operation_id: String,
    /// Run, branch, episode and epoch the caller believes it is reading.
    pub fence: SemanticHistoryFence,
    /// Catalog binding the caller expects the retained history to be bound to.
    pub binding: SemanticCatalogBinding,
    /// Bounded filter and pagination over the retained records.
    #[serde(default)]
    pub query: SemanticEventListQuery,
}

impl SemanticLookupRequest {
    /// Reads one request from the bytes the tool boundary received.
    ///
    /// A request that is empty or over its byte bound is refused as oversized; one that does not parse
    /// as exactly this shape, because a field is unsupported, wrongly typed or a kind name outside the
    /// closed vocabulary, is refused as a shape this port does not serve. Nothing is defaulted in place
    /// of a field the caller did not state, and a limit left unstated reaches the page bound as a zero
    /// rather than a guessed page size.
    pub fn parse(raw: &[u8]) -> SemanticHistoryResult<Self> {
        if raw.is_empty() || raw.len() > SEMANTIC_MAX_LOOKUP_REQUEST_BYTES {
            return Err(SemanticHistoryError::new(Refusal::LookupPayloadTooLarge));
        }
        serde_json::from_slice(raw).map_err(|_| SemanticHistoryError::new(Refusal::LookupShape))
    }

    /// A stable digest of this request, used to tell a re-delivery from a conflicting reuse.
    pub fn digest(&self) -> SemanticHistoryResult<String> {
        let bytes = serde_json::to_vec(self)
            .map_err(|_| SemanticHistoryError::new(Refusal::LookupShape))?;
        Ok(crate::sha256_hex(bytes))
    }
}

/// The gameplay detail one returned record carries, or the reason it carries none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticLookupDetail {
    /// The record was observed; its authoritative detail is returned whole.
    Observed(Box<SemanticEventRecord>),
    /// A retention policy destroyed this record's gameplay detail, but its identity and sequence
    /// number survive, so the span cannot read as an event that never happened.
    RetentionDisclosed,
    /// The record discloses a gap: no kind, no origin and no gameplay values.
    Gap,
}

/// One returned record: enough to identify it, and to see what it does not state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticLookupItem {
    /// Opaque event identity, unique inside its history; retained even after a prune.
    pub event_id: String,
    /// Sequence number the record occupies in the history, retained even after a prune.
    pub sequence: u64,
    /// Coverage the record itself states.
    pub coverage: SemanticCoverageStatus,
    /// Kind, present only when the record was observed.
    pub kind: Option<SemanticEventKind>,
    /// Origin, present only when the record was observed.
    pub origin: Option<SemanticEventOrigin>,
    /// The stated causal parent, and never an inferred one.
    pub causal_parent: Option<String>,
    /// The detail this record carries, or why it carries none.
    pub detail: SemanticLookupDetail,
}

impl SemanticLookupItem {
    /// Projects one retained record, disclosing retention rather than empty values.
    #[must_use]
    pub fn of(record: &SemanticEventRecord) -> Self {
        let event = &record.event;
        let detail = if event.is_observed() {
            SemanticLookupDetail::Observed(Box::new(record.clone()))
        } else if event.coverage.label.as_deref() == Some(SEMANTIC_RETENTION_LABEL) {
            SemanticLookupDetail::RetentionDisclosed
        } else {
            SemanticLookupDetail::Gap
        };
        Self {
            event_id: event.event_id.clone(),
            sequence: event.sequence,
            coverage: event.coverage.status,
            kind: event.kind,
            origin: event.origin,
            causal_parent: event
                .causal_parent
                .as_ref()
                .and_then(|parent| parent.stated_parent())
                .map(str::to_owned),
            detail,
        }
    }

    /// The retained text weight this item carries, which the result bound is checked against.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        let detail = match &self.detail {
            SemanticLookupDetail::Observed(record) => record.event.byte_len(),
            SemanticLookupDetail::RetentionDisclosed | SemanticLookupDetail::Gap => 0,
        };
        self.event_id.len() + self.causal_parent.as_ref().map_or(0, String::len) + detail
    }
}

/// What a history discloses about the history it does not hold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticLookupCoverage {
    /// Sequence number capture began at; nothing before it was watched.
    pub capture_start_sequence: u64,
    /// Whether history existed before capture began.
    pub history_before_capture: bool,
    /// Declared gaps and retention spans, in ascending sequence order.
    pub intervals: Vec<SemanticCoverageInterval>,
    /// Branch this answer was read from.
    pub branch_id: String,
    /// Branch this one was forked from, when it is a fork.
    pub parent_branch_id: Option<String>,
}

/// One bounded answer: the page, and the disclosure that lets it be read honestly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticLookupPage {
    /// Tool name this answer belongs to.
    pub tool: &'static str,
    /// Records the filter matched, in ascending sequence order, up to the requested limit.
    pub items: Vec<SemanticLookupItem>,
    /// Continuation to resume from, present only when more records matched.
    pub continuation: Option<SemanticHistoryContinuation>,
    /// Total records the filter matched, so a caller sees how much it did not read.
    pub matched: usize,
    /// What the history discloses about itself.
    pub coverage: SemanticLookupCoverage,
    /// Retained text weight of this page, bounded by [`SEMANTIC_MAX_LOOKUP_RESULT_BYTES`].
    pub result_bytes: usize,
    /// Whether this operation identity had already been served.
    pub replayed: bool,
}

/// The harness-owned port an agent reads retained history through.
pub trait SemanticLookupPort {
    /// Serves one bounded lookup, or refuses it.
    fn lookup(
        &mut self,
        request: &SemanticLookupRequest,
        correlation: &str,
    ) -> SemanticHistoryResult<SemanticLookupPage>;
}

/// The harness-owned read-only port over one retained store.
pub struct RetainedHistoryLookup<'a> {
    store: &'a SemanticHistoryStore,
    authority: SemanticLookupAuthority,
    served: BTreeMap<String, String>,
}

impl<'a> RetainedHistoryLookup<'a> {
    /// Binds a read-only port to one store under an explicit authority.
    #[must_use]
    pub fn new(store: &'a SemanticHistoryStore, authority: SemanticLookupAuthority) -> Self {
        Self {
            store,
            authority,
            served: BTreeMap::new(),
        }
    }

    /// Returns the authority this port was bound under.
    #[must_use]
    pub fn authority(&self) -> SemanticLookupAuthority {
        self.authority
    }

    /// Returns how many operation identities this session has served.
    #[must_use]
    pub fn served_operations(&self) -> usize {
        self.served.len()
    }
}

impl SemanticLookupPort for RetainedHistoryLookup<'_> {
    fn lookup(
        &mut self,
        request: &SemanticLookupRequest,
        correlation: &str,
    ) -> SemanticHistoryResult<SemanticLookupPage> {
        if correlation.is_empty() || correlation.len() > SEMANTIC_MAX_IDENTITY_BYTES {
            return Err(SemanticHistoryError::new(Refusal::Identity));
        }
        if self.authority != SemanticLookupAuthority::Granted {
            return Err(SemanticHistoryError::about(
                Refusal::LookupNotGranted,
                correlation,
            ));
        }
        validate_identity(&request.operation_id)?;
        validate_identity(&request.fence.run_id)?;
        validate_identity(&request.fence.branch_id)?;
        let branch_id = &request.fence.branch_id;
        let Some(scope) = self.store.scope(branch_id) else {
            return Err(SemanticHistoryError::about(
                Refusal::UnknownBranch,
                branch_id,
            ));
        };
        if self.store.binding(branch_id) != Some(&request.binding) {
            return Err(SemanticHistoryError::about(
                Refusal::BindingMismatch,
                branch_id,
            ));
        }
        if !request.fence.admits(scope) {
            return Err(SemanticHistoryError::about(Refusal::StaleFence, branch_id));
        }
        let Some(window) = self.store.window(branch_id).cloned() else {
            return Err(SemanticHistoryError::about(
                Refusal::UnknownBranch,
                branch_id,
            ));
        };
        let records = self.store.records(branch_id).unwrap_or_default();
        let page = page_history(records, &request.fence, &request.query)?;
        let digest = request.digest()?;
        let replayed = match self.served.get(&request.operation_id) {
            Some(previous) if *previous != digest => {
                return Err(SemanticHistoryError::about(
                    Refusal::IdempotencyConflict,
                    &request.operation_id,
                ));
            }
            Some(_) => true,
            None => false,
        };
        let items = page
            .records
            .iter()
            .map(SemanticLookupItem::of)
            .collect::<Vec<_>>();
        let result_bytes = items
            .iter()
            .map(SemanticLookupItem::byte_len)
            .sum::<usize>();
        if result_bytes > SEMANTIC_MAX_LOOKUP_RESULT_BYTES {
            return Err(SemanticHistoryError::about(
                Refusal::LookupPayloadTooLarge,
                &request.operation_id,
            ));
        }
        self.served.insert(request.operation_id.clone(), digest);
        Ok(SemanticLookupPage {
            tool: SEMANTIC_LOOKUP_TOOL,
            items,
            continuation: page.continuation,
            matched: page.matched,
            coverage: SemanticLookupCoverage {
                capture_start_sequence: window.capture_start_sequence,
                history_before_capture: window.history_before_capture,
                intervals: window.intervals,
                branch_id: branch_id.clone(),
                parent_branch_id: self.store.parent_branch(branch_id).map(str::to_owned),
            },
            result_bytes,
            replayed,
        })
    }
}
