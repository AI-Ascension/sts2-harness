// SPDX-License-Identifier: MIT

//! Owner-issued bounded lifetime scope for one set of context items (issue #111).
//!
//! A scope answers one question: *which logical invocations may see these items?* It is issued by
//! the continuity owner for a single agent/episode/run (optionally pinned to one branch) and carries
//! an explicit applicability plus a wall-clock ceiling. Wall-clock expiry alone does not implement
//! turn-scoped inclusion, so the ceiling is an additional bound on top of the logical window rather
//! than the mechanism itself.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::lifetime_error::ContextLifetimeError;
use super::types::valid_id;

/// Schema identifier for a durable lifetime scope.
pub const CONTEXT_LIFETIME_SCHEMA: &str = "ascension.context-control.lifetime.v1";

/// Upper bound on a declared `next-N` window.
pub const MAX_LIFETIME_NEXT_N: u32 = 64;

/// Upper bound on the item ids one scope may govern.
pub const MAX_LIFETIME_ITEMS: usize = 64;

/// Upper bound on the distinct scopes one run's lifetime state may hold.
///
/// Without a count bound a run could accumulate scopes until the persisted image exceeded its byte
/// bound, leaving a legitimately grown window permanently unpersistable.
pub const MAX_LIFETIME_SCOPES: usize = 64;

/// Upper bound on the manifests one run's lifetime state may hold.
pub const MAX_LIFETIME_MANIFESTS: usize = 512;

/// Byte budget for one run's persisted lifetime image.
///
/// This is the *effective* bound: a scope or admission is refused before it would push the run's
/// image past this budget, so any state reachable through this API is guaranteed to be persistable.
/// The count bounds above are backstops; a window with unusually large items reaches this budget
/// first, and a window with small items reaches a count bound first. Bounding only the counts would
/// let a window grow until `persist_lifetime` refused it forever.
pub const MAX_LIFETIME_STATE_BYTES: usize = 1024 * 1024;

/// Conservative multiplier applied to a record's canonical body when charging the byte budget.
///
/// A manifest is serialized twice inside the image: once as its own fields and once as the numeric
/// array in `bytes`. The array costs at most four characters per byte, so charging `5 * body + 256`
/// bounds the real cost from above and keeps the guarantee without re-serializing the whole image on
/// every admission.
pub(super) const LIFETIME_BODY_CHARGE_FACTOR: usize = 5;

/// Fixed per-record overhead charged on top of the body, covering field names and digests.
pub(super) const LIFETIME_RECORD_OVERHEAD: usize = 256;

/// The agent/episode/run (and optionally branch) a scope was issued for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationOwnerScope {
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
}

impl InvocationOwnerScope {
    /// Validates every declared identity.
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.run_id)
            && valid_id(&self.episode_id)
            && valid_id(&self.agent_id)
            && self
                .branch_id
                .as_ref()
                .is_none_or(|branch| valid_id(branch))
    }

    /// Whether a scope issued for `self` may be consumed by an invocation owned by `other`.
    ///
    /// Run, episode and agent must match exactly, so a sibling cannot inherit scope. A scope that
    /// pinned a branch authorizes only that branch; a scope issued without a branch identity (the
    /// live seam does not always reach one) authorizes the same agent in any branch.
    #[must_use]
    pub fn covers_owner(&self, other: &Self) -> bool {
        self.run_id == other.run_id
            && self.episode_id == other.episode_id
            && self.agent_id == other.agent_id
            && self
                .branch_id
                .as_ref()
                .is_none_or(|branch| other.branch_id.as_ref() == Some(branch))
    }
}

/// How many logical invocations a scope's items remain applicable to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifetimeApplicability {
    /// Exactly the one logical invocation admitted against this scope.
    CurrentInvocation,
    /// The admitted invocation and the next `bound - 1` distinct logical invocations.
    NextN { bound: u32 },
}

impl LifetimeApplicability {
    /// The number of distinct logical invocations this applicability admits.
    #[must_use]
    pub const fn capacity(self) -> u32 {
        match self {
            Self::CurrentInvocation => 1,
            Self::NextN { bound } => bound,
        }
    }

    /// Refuses a zero or oversized window.
    pub const fn validate(self) -> Result<(), ContextLifetimeError> {
        match self {
            Self::CurrentInvocation => Ok(()),
            Self::NextN { bound: 0 } => Err(ContextLifetimeError::EmptyBound),
            Self::NextN { bound } if bound > MAX_LIFETIME_NEXT_N => {
                Err(ContextLifetimeError::InvalidInput)
            }
            Self::NextN { .. } => Ok(()),
        }
    }
}

/// An owner-issued bounded lifetime scope over one set of context item ids.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextLifetimeScope {
    pub schema: String,
    pub scope_id: String,
    pub owner: InvocationOwnerScope,
    pub applicability: LifetimeApplicability,
    /// The item ids whose applicability this scope governs, ordered and unique.
    pub items: Vec<String>,
    /// The logical instant the owner issued this scope.
    pub issued_at: u64,
    /// Wall-clock ceiling: an additional bound on top of the logical window.
    pub ceiling: u64,
}

impl ContextLifetimeScope {
    /// Validates the schema, identities, window and declared items.
    pub fn validate(&self) -> Result<(), ContextLifetimeError> {
        if self.schema != CONTEXT_LIFETIME_SCHEMA || !valid_id(&self.scope_id) {
            return Err(ContextLifetimeError::InvalidInput);
        }
        if !self.owner.valid() {
            return Err(ContextLifetimeError::InvalidOwner);
        }
        self.applicability.validate()?;
        if self.ceiling <= self.issued_at {
            return Err(ContextLifetimeError::InvertedCeiling);
        }
        self.validate_items()
    }

    fn validate_items(&self) -> Result<(), ContextLifetimeError> {
        if self.items.is_empty() || self.items.len() > MAX_LIFETIME_ITEMS {
            return Err(ContextLifetimeError::InvalidInput);
        }
        let mut seen = BTreeSet::new();
        if !self
            .items
            .iter()
            .all(|item| valid_id(item) && seen.insert(item.as_str()))
        {
            return Err(ContextLifetimeError::InvalidInput);
        }
        Ok(())
    }

    /// Digest of the scope exactly as issued, recorded in every manifest it produces.
    pub fn digest(&self) -> Result<String, ContextLifetimeError> {
        let bytes = serde_json::to_vec(self).map_err(|_| ContextLifetimeError::Encode)?;
        Ok(crate::sha256_hex(bytes))
    }

    /// Canonical body length, used to charge this scope against the run's persisted-image budget.
    pub fn body_len(&self) -> Result<usize, ContextLifetimeError> {
        serde_json::to_vec(self)
            .map(|bytes| bytes.len())
            .map_err(|_| ContextLifetimeError::Encode)
    }
}

/// The logical invocation an admission is made on behalf of.
///
/// `invocation_id` is the *logical* identity: a transport retry carries the same `invocation_id`
/// with a different `attempt`, so a retry can never be mistaken for a new logical invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalInvocationIdentity {
    pub owner: InvocationOwnerScope,
    pub invocation_id: String,
    #[serde(default)]
    pub attempt: u32,
}

impl LogicalInvocationIdentity {
    /// Validates the owner and the logical invocation identity.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.owner.valid() && valid_id(&self.invocation_id)
    }
}
