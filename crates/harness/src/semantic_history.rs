// SPDX-License-Identifier: MIT

//! Durable, queryable run history over the game-mod's semantic event vocabulary.
//!
//! The game-mod states authoritative gameplay events and the coverage of its own capture; this
//! module owns everything after that statement: durable append, retention, indexing, bounded
//! filter/pagination, a bounded causal traversal, and idempotent replay across a restart, a
//! replayed batch or a branch fork.
//!
//! The contract is the same one the producer keeps, for the same reason: a history that guesses is
//! worse than a history that admits a gap.
//!
//! - A disclosed gap keeps its sequence number and is stored beside the observed events, so a gap
//!   is never closed by an invented event or renumbered away.
//! - A causal parent is stored only when the producer stated one. Nothing here infers causality
//!   from a difference between two snapshots, and a traversal that would revisit an event is
//!   refused rather than truncated silently.
//! - Sequence order is monotonic and contiguous inside one run, branch, episode and epoch, and a
//!   cross-scope read is refused rather than answered with another run's history.
//! - Re-appending the same batch is idempotent: an identical payload returns the retained history
//!   unchanged, and a changed payload for the same append identity is refused.
//! - A fork copies its ancestor's history by lineage and appends only what is new, so a replayed
//!   or re-joined batch cannot duplicate an event.
//! - Retention is explicit and reference-aware: a policy an operator must disable deliberately can
//!   select observed detail for pruning, a record another surviving record still names as its stated
//!   cause is kept anyway, and every pruned span keeps its sequence number as a declared gap
//!   carrying the retention label, so nothing a policy removed can read as measured.
//! - Lookup is served through one harness-owned agent tool port that borrows the store: it is
//!   capability-gated, refused unless a request is the shape it serves and fits one byte bound, and
//!   refused rather than truncated when a page's retained text weight exceeds the result bound. The
//!   port exposes no store path, so a lookup cannot become a read of an arbitrary artifact.
//!
//! This is a harness-owned historical artifact surface. It reaches no host, no game process and no
//! gateway, and it claims no native event capture.

mod causal;
mod error;
mod ingest;
mod lookup;
mod query;
mod record;
mod replay;
mod retention;
mod roles;
mod scope;
mod store;
mod vocabulary;
mod window;

pub use causal::{SemanticCausalTraversal, SemanticCausalVisit, traverse_causes};
pub use error::{SemanticHistoryError, SemanticHistoryRefusal};
pub use ingest::admit_batch;
pub use lookup::{
    RetainedHistoryLookup, SEMANTIC_LOOKUP_TOOL, SEMANTIC_MAX_LOOKUP_REQUEST_BYTES,
    SEMANTIC_MAX_LOOKUP_RESULT_BYTES, SemanticLookupAuthority, SemanticLookupCoverage,
    SemanticLookupDetail, SemanticLookupItem, SemanticLookupPage, SemanticLookupPort,
    SemanticLookupRequest,
};
pub use query::{
    SemanticEventListQuery, SemanticEventPage, SemanticHistoryContinuation, page_history,
};
pub use record::{
    SemanticCaptureWindow, SemanticCausalParent, SemanticCoverageInterval, SemanticEventBatch,
    SemanticEventCoverage, SemanticEventInput, SemanticEventRecord, SemanticEventSubject,
    SemanticQuantity, SemanticReference,
};
pub use replay::{
    SemanticAppendOutcome, SemanticForkOutcome, SemanticHistoryAppend, SemanticHistoryFork,
};
pub use retention::{
    SEMANTIC_RETENTION_LABEL, SemanticPrunePlan, SemanticPruneRequest, SemanticRetentionPolicy,
};
pub use roles::{
    SemanticCausalProvenance, SemanticCoverageStatus, SemanticIdentityNamespace,
    SemanticSubjectRole,
};
pub use scope::{
    SEMANTIC_HISTORY_PRODUCER_VERSION, SEMANTIC_MAX_CAUSAL_DEPTH, SEMANTIC_MAX_CAUSAL_VISITS,
    SEMANTIC_MAX_EVENTS, SEMANTIC_MAX_HISTORY_BYTES, SEMANTIC_MAX_IDENTITY_BYTES,
    SEMANTIC_MAX_INTERVALS, SEMANTIC_MAX_LABEL_BYTES, SEMANTIC_MAX_PAGE_ITEMS,
    SEMANTIC_MAX_UNIT_BYTES, SemanticCatalogBinding, SemanticEventScope, SemanticFamilyCoverage,
    SemanticFamilyState, SemanticHistoryFence,
};
pub use store::SemanticHistoryStore;
pub use vocabulary::{SemanticEventKind, SemanticEventOrigin};
