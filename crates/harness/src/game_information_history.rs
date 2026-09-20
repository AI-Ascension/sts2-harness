// SPDX-License-Identifier: MIT

//! The harness-owned history tool an agent asks through, and the bypass it closes.
//!
//! History reaches an agent through one closed question vocabulary that contains no storage
//! coordinate: a caller may name a branch, a bounded filter, a page limit and a bounded causal walk,
//! and may not name a path, a bucket, an artifact or a record ordinal. The question is answered by
//! the owned source port rather than from a handle the caller brought with it, so the MCP game
//! adapter has nothing to bypass with and no harness entry point to reverse-call: the only history
//! it can reach is the one the owner attached to its session, through this port.
//!
//! The owner scope is never taken from the question. It comes from the session, so a question asking
//! for another run, another profile or another epoch is refused rather than served from a wider
//! store, and a session the owner attached no history to answers `MissingCapability` — a refusal
//! with a name rather than an empty history.
//!
//! Every answer carries the coverage the page cannot show: declared gaps, whether the window reaches
//! before capture began, and, per event, the origin its warrant came from. A short page is therefore
//! never read as a quiet run, and an imported or derived event is never presented as something this
//! boundary observed.

use super::*;
use crate::semantic_history::{
    SemanticHistoryCursor, SemanticHistoryError, SemanticHistoryKind, SemanticHistoryOrigin,
    SemanticHistoryQuery, SemanticHistoryTraversalLimits, validate_history_identity,
};

#[path = "game_information_history_answer.rs"]
mod answer;

pub(crate) use answer::serve_history;

/// The canonical request profile one history turn carries.
pub(crate) const HISTORY_AGENT_PROFILE: &str = "ascension.semantic-history-agent-tool.v1";
/// The marker every answer carries: recorded game values are data, never instructions.
pub(crate) const HISTORY_AUTHORITY: &str = "untrusted_recorded_history";
/// Maximum events one tool answer may carry.
///
/// The bound is below the store's own page bound because one answer must fit one bounded feedback
/// envelope: a page a caller cannot receive is not a page. A caller wanting more asks again with the
/// continuation it was given.
pub(crate) const MAX_HISTORY_TOOL_PAGE: usize = 8;

/// One resumable position as it travels over the tool boundary.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCursor {
    pub(crate) query_digest: String,
    pub(crate) generation: u64,
    pub(crate) next_sequence: u64,
}

impl WireCursor {
    pub(crate) fn from_cursor(cursor: &SemanticHistoryCursor) -> Self {
        Self {
            query_digest: cursor.query_digest.clone(),
            generation: cursor.generation,
            next_sequence: cursor.next_sequence,
        }
    }

    pub(crate) fn cursor(&self) -> SemanticHistoryCursor {
        SemanticHistoryCursor {
            query_digest: self.query_digest.clone(),
            generation: self.generation,
            next_sequence: self.next_sequence,
        }
    }
}

/// The bounds one causal walk may consume, as they travel over the tool boundary.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireLimits {
    pub(crate) max_depth: usize,
    pub(crate) max_visits: usize,
}

impl WireLimits {
    pub(crate) fn bounds(&self) -> SemanticHistoryTraversalLimits {
        SemanticHistoryTraversalLimits {
            max_depth: self.max_depth,
            max_visits: self.max_visits,
        }
    }

    pub(crate) fn of(limits: SemanticHistoryTraversalLimits) -> Self {
        Self {
            max_depth: limits.max_depth,
            max_visits: limits.max_visits,
        }
    }
}

/// The closed question vocabulary the provider may ask.
///
/// Every arm is a question about a branch. None of them can name an owner, an epoch, a path or a
/// record ordinal, because a question that could address those would let a caller reach history the
/// owner did not grant. Absent bounds are the boundary's own bounded default rather than a request
/// for an unbounded walk.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum HistoryArguments {
    /// Read one bounded page.
    Page {
        operation_id: String,
        branch_id: String,
        kind: Option<SemanticHistoryKind>,
        origin: Option<SemanticHistoryOrigin>,
        subject_id: Option<String>,
        episode: Option<u64>,
        from_sequence: Option<u64>,
        to_sequence: Option<u64>,
        limit: usize,
        continuation: Option<WireCursor>,
    },
    /// Summarize one branch.
    Summary {
        operation_id: String,
        branch_id: String,
    },
    /// Explain one observed change inside the caller's walk bounds.
    Explain {
        operation_id: String,
        branch_id: String,
        event_id: String,
        limits: Option<WireLimits>,
    },
}

impl HistoryArguments {
    fn operation_id(&self) -> &str {
        match self {
            Self::Page { operation_id, .. }
            | Self::Summary { operation_id, .. }
            | Self::Explain { operation_id, .. } => operation_id,
        }
    }

    fn into_ask(self) -> Result<HistoryAsk, LookupError> {
        Ok(match self {
            Self::Page {
                branch_id,
                kind,
                origin,
                subject_id,
                episode,
                from_sequence,
                to_sequence,
                limit,
                continuation,
                ..
            } => {
                if limit == 0 || limit > MAX_HISTORY_TOOL_PAGE {
                    return Err(LookupError::Bounds);
                }
                let query = SemanticHistoryQuery {
                    branch_id,
                    kind,
                    origin,
                    subject_id,
                    episode,
                    from_sequence,
                    to_sequence,
                    limit,
                };
                query.validate().map_err(refuse)?;
                HistoryAsk::Page {
                    query,
                    continuation,
                }
            }
            Self::Summary { branch_id, .. } => {
                validate_history_identity(&branch_id, "branch_id").map_err(refuse)?;
                HistoryAsk::Summary { branch_id }
            }
            Self::Explain {
                branch_id,
                event_id,
                limits,
                ..
            } => {
                let limits = limits
                    .unwrap_or_else(|| WireLimits::of(SemanticHistoryTraversalLimits::bounded()));
                limits.bounds().validate().map_err(refuse)?;
                validate_history_identity(&branch_id, "branch_id").map_err(refuse)?;
                validate_history_identity(&event_id, "event_id").map_err(refuse)?;
                HistoryAsk::Explain {
                    branch_id,
                    event_id,
                    limits,
                }
            }
        })
    }
}

/// One question as it is stored in the turn the loop serves.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "ask", rename_all = "snake_case")]
pub(crate) enum HistoryAsk {
    /// Read one bounded page, optionally resuming from a continuation.
    Page {
        query: SemanticHistoryQuery,
        continuation: Option<WireCursor>,
    },
    /// Summarize one branch.
    Summary { branch_id: String },
    /// Explain one observed change inside the caller's walk bounds.
    Explain {
        branch_id: String,
        event_id: String,
        limits: WireLimits,
    },
}

/// The canonical request one history turn carries.
///
/// The request is decoded and validated again when it is served, so a turn that could not have been
/// built from the closed vocabulary is refused rather than answered from a partially understood
/// question.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryRequest {
    pub(crate) profile: String,
    pub(crate) ask: HistoryAsk,
}

/// Builds one bounded history turn from the tool arguments the provider supplied.
///
/// Nothing is taken from the session input: the question cannot state an owner scope, so there is
/// nothing for the provider to bind and nothing for this boundary to trust.
pub(crate) fn turn(arguments: Value) -> Result<LookupTurn, LookupError> {
    if serde_json::to_vec(&arguments)
        .map_err(|_| LookupError::Invalid)?
        .len()
        > crate::exo_lookup_wire::EXO_LOOKUP_TOOL_BYTES
    {
        return Err(LookupError::Bounds);
    }
    let arguments: HistoryArguments =
        serde_json::from_value(arguments).map_err(|_| LookupError::Invalid)?;
    let operation_id = arguments.operation_id().to_owned();
    if operation_id.is_empty()
        || operation_id.len() > 64
        || !operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(LookupError::Invalid);
    }
    let request = HistoryRequest {
        profile: HISTORY_AGENT_PROFILE.to_owned(),
        ask: arguments.into_ask()?,
    };
    let request = serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?;
    // The canonical request must survive the same strict decode it will be served through.
    crate::game_information_validation::decode_strict(&request)?;
    Ok(LookupTurn::History {
        operation_id,
        request,
    })
}

/// Maps one history refusal into the lookup vocabulary.
///
/// The mapping is exhaustive on purpose: a refusal this boundary has not classified cannot be
/// silently answered as a different kind of refusal.
fn refuse(error: SemanticHistoryError) -> LookupError {
    match error {
        SemanticHistoryError::InvalidLabel(_)
        | SemanticHistoryError::InvalidIdentity(_)
        | SemanticHistoryError::NonOpaqueIdentity(_)
        | SemanticHistoryError::InvalidField(_)
        | SemanticHistoryError::MissingSubject(_)
        | SemanticHistoryError::UnexpectedSubjectRole(_)
        | SemanticHistoryError::DuplicateSubjectRole(_)
        | SemanticHistoryError::WrongSubjectNamespace(_)
        | SemanticHistoryError::IdentityNamespaceCollision(_)
        | SemanticHistoryError::Coverage
        | SemanticHistoryError::Causality
        | SemanticHistoryError::ParentNotPreceding
        | SemanticHistoryError::Cycle
        | SemanticHistoryError::Traversal
        | SemanticHistoryError::Port
        | SemanticHistoryError::ImportedOrigin
        | SemanticHistoryError::ImportedStatesParent => LookupError::Invalid,
        SemanticHistoryError::Bounds | SemanticHistoryError::Capacity => LookupError::Bounds,
        SemanticHistoryError::Scope
        | SemanticHistoryError::Epoch
        | SemanticHistoryError::Branch
        | SemanticHistoryError::Lineage
        | SemanticHistoryError::Authority => LookupError::Scope,
        SemanticHistoryError::Retention => LookupError::MissingRetention,
        SemanticHistoryError::Privacy => LookupError::Retention,
        // A continuation from another question, or from a history that has since advanced, is not
        // the answer the caller is holding: it diverged rather than being merely malformed.
        SemanticHistoryError::Continuation
        | SemanticHistoryError::Sequence
        | SemanticHistoryError::Corrupt
        | SemanticHistoryError::MixedGeneration => LookupError::Divergence,
        // Another writer holds the store, or the durable medium refused this read.
        SemanticHistoryError::Locked | SemanticHistoryError::Persistence(_) => {
            LookupError::Reobserve
        }
    }
}
