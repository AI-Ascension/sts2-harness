// SPDX-License-Identifier: MIT

//! Harness-owned durable semantic run history: append, retain, index, query and explain.
//!
//! This is the harness half of the run-history feature whose game-mod companion states the bounded
//! event vocabulary. The host states what it authoritatively observed; this module owns everything
//! after that — durable append, retention, ordering, indexing, bounded query, causal traversal and
//! the single port an agent may read history through.
//!
//! The contract is refusal rather than reconstruction, for the same reason the companion vocabulary
//! refuses: a history that guesses is worse than a history that admits a gap.
//!
//! - History is durable and separate from provider conversation history. Nothing here reads or
//!   writes a provider transcript, and no provider message can become an event.
//! - Event identity and order bind to run, branch, episode, epoch and host sequence. A sequence that
//!   moves backwards, repeats with different content, or skips inside one epoch is refused rather
//!   than renumbered, and a declared gap stays declared.
//! - Capture start, dropped and unsupported intervals, and each event's origin are tracked, so an
//!   interval this boundary could not observe is disclosed instead of being closed by an invented
//!   event or a zeroed value.
//! - A causal parent is either explicitly stated or explicitly absent. It is never inferred from a
//!   difference between two snapshots, and a stated parent must exist in the same branch and epoch
//!   and precede its child.
//! - Appends are idempotent across restart, replay and rejoin: the same event identity with the same
//!   content replays the recorded outcome and writes nothing, and the same identity with different
//!   content is a conflict rather than a second event. Restart is served by encoding the whole
//!   owner scope to one document and restoring it through the same validation an append goes
//!   through, so a document that could not have been written is refused rather than loaded.
//! - Retention and privacy are applied by redaction, never by inventing a value: a redacted payload
//!   keeps the event, its coverage and its causal link, and reports the value as unavailable.
//! - The store is reachable only through the harness-owned port. A request that names storage
//!   directly is refused, so the MCP game adapter cannot bypass to arbitrary artifact storage.
//!
//! No native capture, live game read, transport route or exact-host compatibility is claimed here.

pub const SEMANTIC_HISTORY_SCHEMA: &str = "ascension.semantic-history.v1";
/// Maximum events retained for one run and branch.
pub const MAX_HISTORY_EVENTS: usize = 4096;
/// Maximum serialized bytes of one stored event.
pub const MAX_HISTORY_EVENT_BYTES: usize = 16 * 1024;
/// Maximum length of any identity this boundary carries.
pub const MAX_HISTORY_IDENTITY_BYTES: usize = 256;
/// Maximum number of dot-separated segments in one identity.
pub const MAX_HISTORY_IDENTITY_SEGMENTS: usize = 8;
/// Maximum length of an owner-defined label.
pub const MAX_HISTORY_LABEL_BYTES: usize = 128;
/// Maximum events returned by one page.
pub const MAX_HISTORY_PAGE: usize = 256;
/// Maximum causal depth one traversal may walk.
pub const MAX_HISTORY_TRAVERSAL_DEPTH: usize = 16;
/// Maximum events one traversal may visit.
pub const MAX_HISTORY_TRAVERSAL_VISITS: usize = 256;
/// Maximum branch-lineage depth this boundary will follow.
pub const MAX_HISTORY_BRANCH_DEPTH: usize = 16;

#[path = "semantic_history_binding.rs"]
mod binding;
#[path = "semantic_history_causal.rs"]
mod causal;
#[path = "semantic_history_coverage.rs"]
mod coverage;
#[path = "semantic_history_error.rs"]
mod error;
#[path = "semantic_history_identity.rs"]
mod identity;
#[path = "semantic_history_index.rs"]
mod index;
#[path = "semantic_history_kind.rs"]
mod kind;
#[path = "semantic_history_model.rs"]
mod model;
#[path = "semantic_history_port.rs"]
mod port;
#[path = "semantic_history_query.rs"]
mod query;
#[path = "semantic_history_reference.rs"]
mod reference;
#[path = "semantic_history_store.rs"]
mod store;
#[path = "semantic_history_subject.rs"]
mod subject;
#[path = "semantic_history_validation.rs"]
mod validation;

pub use binding::{SemanticHistoryBinding, SemanticHistoryLineage};
pub use causal::{
    SemanticHistoryExplanation, SemanticHistoryLink, SemanticHistoryTraversal,
    SemanticHistoryTraversalLimits,
};
pub use coverage::{
    SemanticHistoryCaptureWindow, SemanticHistoryCoverage, SemanticHistoryCoverageInterval,
    SemanticHistoryCoverageStatus,
};
pub use error::{SemanticHistoryAuthority, SemanticHistoryError};
pub use identity::{
    SemanticHistoryNamespace, is_opaque_history_identity, validate_history_identity,
};
pub use index::{SemanticHistoryCursor, SemanticHistoryIndex};
pub use kind::SemanticHistoryKind;
pub use model::{
    SemanticHistoryCausalParent, SemanticHistoryEvent, SemanticHistoryEventInput,
    SemanticHistoryOrigin, SemanticHistoryValue,
};
pub use port::{
    SemanticHistoryAgentPort, SemanticHistorySourcePort, SemanticHistorySourceRequest,
    SemanticHistorySourceResponse,
};
pub use query::{
    SemanticHistoryContinuation, SemanticHistoryPage, SemanticHistoryQuery, SemanticHistoryReader,
    SemanticHistorySummary,
};
pub use reference::SemanticHistoryReference;
pub use store::{SemanticHistoryAppend, SemanticHistoryRetention, SemanticHistoryStore};
pub use subject::{SemanticHistorySubject, SemanticHistorySubjectRole};

use error::SemanticHistoryError as Error;

/// Validates one owner-defined label.
pub(crate) fn validate_label(value: &str, field: &'static str) -> Result<(), Error> {
    if !value.is_empty()
        && value.len() <= MAX_HISTORY_LABEL_BYTES
        && !value.contains(|character: char| character.is_control())
    {
        Ok(())
    } else {
        Err(Error::InvalidLabel(field))
    }
}

/// Returns the digest used to detect that two records with one identity disagree.
pub(crate) fn history_digest(bytes: &[u8]) -> String {
    crate::sha256_hex(bytes)
}
