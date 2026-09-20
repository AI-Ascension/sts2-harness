// SPDX-License-Identifier: MIT

//! Closed refusal vocabulary for the durable semantic history boundary.

use serde::{Deserialize, Serialize};

/// Whether this boundary may serve history at all.
///
/// History authority is not implied by owning a store handle. The MCP game adapter is deliberately
/// not an authority: it may ask the harness-owned port, and it may not read storage itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistoryAuthority {
    /// This boundary serves history through its own port only.
    HarnessOwned,
    /// The caller asked to read storage directly; that is refused.
    NotGranted,
}

/// Stable refusal reasons. Producer text is never used as error authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticHistoryError {
    /// A label is empty, oversized or carries a control byte.
    InvalidLabel(&'static str),
    /// An identity is empty, oversized or carries a control byte.
    InvalidIdentity(&'static str),
    /// An identity could be read as a host path, so it is refused.
    NonOpaqueIdentity(&'static str),
    /// A required field was absent or a field was stated where it is not admitted.
    InvalidField(&'static str),
    /// A kind required an end of the event that was not named.
    MissingSubject(&'static str),
    /// A subject named the target end of a kind that does not act on a target.
    UnexpectedSubjectRole(&'static str),
    /// One end of an event was named more than once.
    DuplicateSubjectRole(&'static str),
    /// A subject was minted in a namespace that is not a live instance.
    WrongSubjectNamespace(&'static str),
    /// A live subject shares a token with the event, the branch or the run.
    IdentityNamespaceCollision(&'static str),
    /// The record belongs to a different owner scope.
    Scope,
    /// The record belongs to a different authority epoch.
    Epoch,
    /// The record belongs to a different branch.
    Branch,
    /// A branch lineage is cyclic, self-referential or deeper than the bound.
    Lineage,
    /// A sequence moved backwards, skipped inside one epoch, or repeated.
    Sequence,
    /// A declared coverage gap was not honoured, or an interval was stated where none exists.
    Coverage,
    /// A causal parent pairing this vocabulary does not admit.
    Causality,
    /// A stated causal parent does not precede its child.
    ParentNotPreceding,
    /// An imported event attempted to state a causal parent.
    ImportedStatesParent,
    /// A backfill offered an event whose origin claims it was observed here.
    ImportedOrigin,
    /// A filter, page or traversal bound was exceeded or malformed.
    Bounds,
    /// A continuation is unknown, superseded, already consumed or bound to another query.
    Continuation,
    /// A causal traversal found a cycle.
    Cycle,
    /// A causal traversal exceeded its depth or visit bound.
    Traversal,
    /// The store is at capacity for this run and branch.
    Capacity,
    /// The store is held by another writer.
    Locked,
    /// A durable record could not be decoded as written.
    Corrupt,
    /// The durable medium refused an operation.
    Persistence(String),
    /// Retention refused the append rather than silently dropping an event.
    Retention,
    /// Privacy redaction refused a payload that may not be retained.
    Privacy,
    /// The caller asked to bypass the harness-owned port.
    Authority,
    /// The supplied port refused the request.
    Port,
    /// Two records in one generation disagree about identity, order or content.
    MixedGeneration,
}

impl std::fmt::Display for SemanticHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLabel(field) => write!(f, "semantic history: invalid label: {field}"),
            Self::InvalidIdentity(field) => {
                write!(f, "semantic history: invalid identity: {field}")
            }
            Self::NonOpaqueIdentity(field) => {
                write!(f, "semantic history: non-opaque identity: {field}")
            }
            Self::InvalidField(field) => write!(f, "semantic history: invalid field: {field}"),
            Self::MissingSubject(role) => {
                write!(f, "semantic history: missing subject: {role}")
            }
            Self::UnexpectedSubjectRole(role) => {
                write!(f, "semantic history: unexpected subject role: {role}")
            }
            Self::DuplicateSubjectRole(role) => {
                write!(f, "semantic history: duplicate subject role: {role}")
            }
            Self::WrongSubjectNamespace(role) => {
                write!(f, "semantic history: wrong subject namespace: {role}")
            }
            Self::IdentityNamespaceCollision(role) => {
                write!(f, "semantic history: identity namespace collision: {role}")
            }
            other => write!(f, "semantic history: {other:?}"),
        }
    }
}

impl std::error::Error for SemanticHistoryError {}
