// SPDX-License-Identifier: MIT

//! Identity-bound, latest-only observation selection slots (issue #112).
//!
//! A slot answers exactly one question per identity: *which observation is current for
//! this run/episode/agent and projection?* It holds at most one effective
//! [`ContextBoundary`]. Pinned values are additionally retained as attributed history, and every
//! supersession is recorded in a deterministic lineage, so replacing the selected observation
//! never erases which value was replaced by which.
//!
//! ## What the slot refuses
//!
//! The slot is the seam where arrival order stops mattering. A candidate is admitted only
//! when it is complete, coherent, bound to this slot's identity, and strictly newer than the
//! effective value on the owner's `(controller_epoch, gate_epoch)` order. Anything else is an
//! explicit refusal:
//!
//! - a candidate whose boundary carries another run/episode/agent, or another projection
//!   revision, is [`ObservationSlotRefusal::ForeignIdentity`];
//! - a candidate missing a digest, a state id, or a positive generation is
//!   [`ObservationSlotRefusal::PartialStateCatalog`];
//! - a candidate whose catalog was generated for a different state generation is
//!   [`ObservationSlotRefusal::StateCatalogMismatch`];
//! - a candidate older than the effective value is [`ObservationSlotRefusal::OutOfOrder`];
//! - a candidate that claims the same sequence as the effective value while disagreeing about
//!   state or digests is [`ObservationSlotRefusal::ConflictingSameSequence`].
//!
//! An identical duplicate of the effective value is idempotent: it is reported as unchanged
//! and produces no new lineage, so a replayed delivery cannot fork the history.
//!
//! ## Stale is not current
//!
//! `read` returns the effective value only while the slot is healthy. Once a required refresh
//! has failed, `read` returns [`ObservationSlotRefusal::RefreshRequired`] and the caller must
//! block or re-observe; the last-known value is never silently served as current. Arrival time
//! alone can never promote a value: only the owner's generation and gate order can.

use super::types::{ContextBoundary, valid_id};
use serde::{Deserialize, Serialize};

/// Stable schema identity of one latest-only observation selection slot.
pub const OBSERVATION_SLOT_SCHEMA: &str = "ascension.context-control.observation-slot.v1";

/// Upper bound on retained superseded values per slot.
pub const MAX_SLOT_HISTORY: usize = 32;

/// Upper bound on retained supersession lineage records per slot.
pub const MAX_SLOT_LINEAGE: usize = 64;

/// The identity a slot is bound to. Every field must match an admitted boundary exactly, so a
/// value produced for another run, episode, agent, source or projection cannot enter.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSlotKey {
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
    /// The owner-declared source kind, such as `host-observe`.
    pub source_kind: String,
    /// The projection revision that produced the value; bound to the boundary's adapter
    /// revision so a value projected by another reader is refused.
    pub projection_revision: String,
}

impl ObservationSlotKey {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        episode_id: impl Into<String>,
        agent_id: impl Into<String>,
        source_kind: impl Into<String>,
        projection_revision: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            episode_id: episode_id.into(),
            agent_id: agent_id.into(),
            source_kind: source_kind.into(),
            projection_revision: projection_revision.into(),
        }
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.run_id)
            && valid_id(&self.episode_id)
            && valid_id(&self.agent_id)
            && valid_id(&self.source_kind)
            && valid_id(&self.projection_revision)
    }

    fn binds(&self, boundary: &ContextBoundary) -> bool {
        self.run_id == boundary.run_id
            && self.episode_id == boundary.episode_id
            && self.agent_id == boundary.agent_id
            && self.projection_revision == boundary.adapter_revision
    }
}

/// Why a candidate could not replace, or a read could not serve, a slot value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationSlotRefusal {
    /// The key or boundary is malformed.
    InvalidIdentity,
    /// The boundary belongs to another run, episode, agent or projection revision.
    ForeignIdentity,
    /// A required identity, digest or generation is missing.
    PartialStateCatalog,
    /// The catalog was generated for a different state generation than the boundary.
    StateCatalogMismatch,
    /// The candidate is older than the effective value on the owner's order.
    OutOfOrder,
    /// The candidate claims the effective sequence while disagreeing about its content.
    ConflictingSameSequence,
    /// The value has expired at the observed instant.
    Expired,
    /// A required refresh failed; the last-known value must not be served as current.
    RefreshRequired,
    /// The slot already holds its declared maximum of retained history or lineage.
    Capacity,
}

/// One candidate delivered to a slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSlotAdmission {
    pub key: ObservationSlotKey,
    pub boundary: ContextBoundary,
    /// The state generation the accompanying catalog was produced for.
    pub catalog_generation: u64,
    pub admitted_at: String,
    pub expires_at: String,
    /// Whether the owner pins this value as attributed history rather than current.
    pub pinned: bool,
}

/// A value the slot has admitted; the effective one or a superseded historical one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedObservationSlot {
    pub schema: String,
    pub slot_id: String,
    pub key: ObservationSlotKey,
    pub boundary: ContextBoundary,
    pub admitted_at: String,
    pub expires_at: String,
    pub pinned: bool,
}

impl AdmittedObservationSlot {
    #[must_use]
    pub fn ordering(&self) -> (u64, u64) {
        (self.boundary.controller_epoch, self.boundary.gate_epoch)
    }
}

/// Why a supersession happened, recorded so the lineage explains itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupersessionReason {
    /// The first value admitted to an empty slot.
    Initial,
    /// A strictly newer owner generation/gate replaced the effective value.
    NewerGeneration,
}

/// One deterministic supersession step: which value was replaced by which, and under which
/// approval fence the replacement happened.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupersessionRecord {
    pub reason: SupersessionReason,
    pub superseded_state_id: Option<String>,
    pub superseded_observation_sha256: Option<String>,
    pub superseded_epoch: Option<u64>,
    pub superseded_gate: Option<u64>,
    pub admitted_state_id: String,
    pub admitted_observation_sha256: String,
    pub admitted_epoch: u64,
    pub admitted_gate: u64,
    /// The approval fence in force after this step.
    pub approval_fence: u64,
}

/// The observable result of an admission attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotAdmission {
    /// The empty slot accepted its first value.
    Inserted { approval_fence: u64 },
    /// The value replaced the effective value and was recorded in the lineage.
    Superseded { approval_fence: u64 },
    /// The value is identical to the effective value; nothing changed.
    Unchanged,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn boundary_identity_ok(boundary: &ContextBoundary) -> bool {
    valid_id(&boundary.run_id)
        && valid_id(&boundary.episode_id)
        && valid_id(&boundary.agent_id)
        && valid_id(&boundary.adapter_revision)
        && valid_id(&boundary.model_revision)
}

pub(crate) fn boundary_complete(boundary: &ContextBoundary) -> bool {
    boundary_identity_ok(boundary)
        && valid_id(&boundary.state_id)
        && boundary.generation > 0
        && boundary.control_version > 0
        && valid_digest(&boundary.observation_sha256)
        && valid_digest(&boundary.catalog_sha256)
        && valid_digest(&boundary.configuration_sha256)
        && valid_digest(&boundary.output_schema_sha256)
}

pub(crate) fn valid_expiry(admitted_at: &str, expires_at: &str) -> bool {
    valid_timestamp(admitted_at) && valid_timestamp(expires_at) && expires_at > admitted_at
}

pub(crate) fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    let digits = |start: usize, end: usize| bytes[start..end].iter().all(u8::is_ascii_digit);
    if !digits(0, 4)
        || !digits(5, 7)
        || !digits(8, 10)
        || !digits(11, 13)
        || !digits(14, 16)
        || !digits(17, 19)
    {
        return false;
    }
    let parse = |start: usize, end: usize| value[start..end].parse::<u32>().unwrap_or(u32::MAX);
    let (year, month, day) = (parse(0, 4), parse(5, 7), parse(8, 10));
    let (hour, minute, second) = (parse(11, 13), parse(14, 16), parse(17, 19));
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    (1..=12).contains(&month)
        && (1..=days).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60
}

pub(crate) fn slot_digest(key: &ObservationSlotKey) -> String {
    let bytes = serde_json::to_vec(key).unwrap_or_default();
    crate::sha256_hex(bytes)
}

impl ObservationSlotAdmission {
    pub(crate) fn validate(
        &self,
        slot_id: &str,
    ) -> Result<AdmittedObservationSlot, ObservationSlotRefusal> {
        if !self.key.valid() || !boundary_identity_ok(&self.boundary) {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
        if !boundary_complete(&self.boundary) {
            return Err(ObservationSlotRefusal::PartialStateCatalog);
        }
        if !self.key.binds(&self.boundary) {
            return Err(ObservationSlotRefusal::ForeignIdentity);
        }
        if self.catalog_generation == 0 {
            return Err(ObservationSlotRefusal::PartialStateCatalog);
        }
        if self.catalog_generation != self.boundary.generation {
            return Err(ObservationSlotRefusal::StateCatalogMismatch);
        }
        if !valid_expiry(&self.admitted_at, &self.expires_at) {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
        Ok(AdmittedObservationSlot {
            schema: OBSERVATION_SLOT_SCHEMA.to_owned(),
            slot_id: slot_id.to_owned(),
            key: self.key.clone(),
            boundary: self.boundary.clone(),
            admitted_at: self.admitted_at.clone(),
            expires_at: self.expires_at.clone(),
            pinned: self.pinned,
        })
    }
}

#[cfg(test)]
#[path = "observation_slot_tests.rs"]
mod tests;
