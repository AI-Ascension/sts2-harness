// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::legal_actions::{ActionKind, EpisodeLegalActionSet};

#[path = "map_envelope.rs"]
mod envelope;
#[path = "map_graph.rs"]
mod graph;
#[path = "map_snapshot.rs"]
mod snapshot;

pub(crate) const RUNTIME_MAP_PROFILE: &str = "runtime-map-v1";
pub(crate) const RUNTIME_MAP_SCHEMA_DIGEST: &str =
    "ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b";
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub(crate) const MAP_CONTEXT_WIRE_FIXED_BYTES: usize = 208;
const MAX_ID_BYTES: usize = 128;
const MAX_HOST_ACTION_ID_BYTES: usize = 512;
const MAX_TEXT_BYTES: usize = 128;
const MAX_NODES: usize = 256;
const MAX_EDGES: usize = 1_024;
const MAX_BINDINGS: usize = 256;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;
const VISIBLE_MAP_SCHEMA: &str = "visible-map-v1";

/// A complete, current, host-authored map projection bound to one observation catalog.
///
/// The map remains separate from `SanitizedObservation`: map visibility is an explicit provider
/// context opt-in, while action authority stays in the host legal-action catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MapDecisionContext {
    snapshot: Value,
    snapshot_digest: String,
    state_id: String,
    generation: u64,
}

impl MapDecisionContext {
    pub(crate) fn from_mcp_value(
        value: &Value,
        state_id: &str,
        generation: u64,
        actions: &EpisodeLegalActionSet,
    ) -> Result<Self, MapError> {
        let object = value.as_object().ok_or(MapError::InvalidEnvelope)?;
        envelope::validate(object)?;
        let snapshot = object.get("snapshot").ok_or(MapError::InvalidSnapshot)?;
        let context = Self::from_snapshot(snapshot)?;
        context.validate_binding(state_id, generation, actions)?;
        Ok(context)
    }

    /// Parses the compact map wrapper carried inside an Exo decision request.
    pub(crate) fn from_exo_value(
        value: &Value,
        state_id: &str,
        generation: u64,
        legal_action_ids: &[String],
    ) -> Result<Self, MapError> {
        let object = value.as_object().ok_or(MapError::InvalidContext)?;
        let expected = ["profile", "schema_digest", "snapshot_digest", "snapshot"];
        if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
            return Err(MapError::InvalidContext);
        }
        if object.get("profile").and_then(Value::as_str) != Some(RUNTIME_MAP_PROFILE)
            || object.get("schema_digest").and_then(Value::as_str)
                != Some(RUNTIME_MAP_SCHEMA_DIGEST)
        {
            return Err(MapError::UnsupportedVersion);
        }
        let context =
            Self::from_snapshot(object.get("snapshot").ok_or(MapError::InvalidSnapshot)?)?;
        if object.get("snapshot_digest").and_then(Value::as_str) != Some(context.snapshot_digest())
        {
            return Err(MapError::InvalidDigest);
        }
        if context.state_id != state_id || context.generation != generation {
            return Err(MapError::CatalogMismatch);
        }
        let bindings = context
            .snapshot
            .get("bindings")
            .and_then(Value::as_array)
            .ok_or(MapError::InvalidBinding)?;
        let ids = bindings
            .iter()
            .map(|binding| {
                binding
                    .get("host_action_id")
                    .and_then(Value::as_str)
                    .ok_or(MapError::InvalidBinding)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let expected_ids = legal_action_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if ids != expected_ids || ids.len() != legal_action_ids.len() {
            return Err(MapError::CatalogMismatch);
        }
        Ok(context)
    }

    fn from_snapshot(snapshot: &Value) -> Result<Self, MapError> {
        let bytes = serde_json::to_vec(snapshot).map_err(|_| MapError::InvalidSnapshot)?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(MapError::SnapshotTooLarge);
        }
        snapshot::validate(snapshot)?;
        let canonical = canonical_snapshot(snapshot)?;
        let canonical_bytes =
            serde_json::to_vec(&canonical).map_err(|_| MapError::InvalidDigest)?;
        let object = snapshot.as_object().ok_or(MapError::InvalidSnapshot)?;
        let state_id = object
            .get("state_id")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidIdentity)?
            .to_owned();
        let generation = object
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or(MapError::InvalidGeneration)?;
        let snapshot_digest = format!("{:x}", Sha256::digest(&canonical_bytes));
        Ok(Self {
            snapshot: canonical,
            snapshot_digest,
            state_id,
            generation,
        })
    }

    fn validate_binding(
        &self,
        state_id: &str,
        generation: u64,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), MapError> {
        if self.state_id != state_id || self.generation != generation {
            return Err(MapError::CatalogMismatch);
        }
        let action_ids = actions
            .actions()
            .iter()
            .map(|action| {
                if action.kind() != ActionKind::SelectMapNode {
                    return Err(MapError::InvalidBinding);
                }
                Ok(action.action_id())
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let bindings = self
            .snapshot
            .get("bindings")
            .and_then(Value::as_array)
            .ok_or(MapError::InvalidBinding)?;
        let binding_ids = bindings
            .iter()
            .map(|binding| {
                binding
                    .get("host_action_id")
                    .and_then(Value::as_str)
                    .ok_or(MapError::InvalidBinding)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if binding_ids != action_ids || binding_ids.len() != bindings.len() {
            return Err(MapError::CatalogMismatch);
        }
        Ok(())
    }

    pub(crate) fn snapshot_digest(&self) -> &str {
        &self.snapshot_digest
    }

    pub(crate) fn to_wire(&self) -> Value {
        serde_json::json!({
            "profile": RUNTIME_MAP_PROFILE,
            "schema_digest": RUNTIME_MAP_SCHEMA_DIGEST,
            "snapshot_digest": self.snapshot_digest,
            "snapshot": self.snapshot,
        })
    }
}

fn canonical_snapshot(snapshot: &Value) -> Result<Value, MapError> {
    let mut object = snapshot
        .as_object()
        .ok_or(MapError::InvalidSnapshot)?
        .clone();
    if let Some(nodes) = object.get_mut("nodes").and_then(Value::as_array_mut) {
        nodes.sort_by(|left, right| {
            left.get("id")
                .and_then(Value::as_str)
                .cmp(&right.get("id").and_then(Value::as_str))
        });
    }
    if let Some(edges) = object.get_mut("edges").and_then(Value::as_array_mut) {
        edges.sort_by(|left, right| {
            (
                left.get("from").and_then(Value::as_str),
                left.get("to").and_then(Value::as_str),
            )
                .cmp(&(
                    right.get("from").and_then(Value::as_str),
                    right.get("to").and_then(Value::as_str),
                ))
        });
    }
    if let Some(terminals) = object
        .get_mut("terminal_node_ids")
        .and_then(Value::as_array_mut)
    {
        terminals.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
    if let Some(bindings) = object.get_mut("bindings").and_then(Value::as_array_mut) {
        bindings.sort_by(|left, right| {
            (
                left.get("graph_node_id").and_then(Value::as_str),
                left.get("host_action_id").and_then(Value::as_str),
            )
                .cmp(&(
                    right.get("graph_node_id").and_then(Value::as_str),
                    right.get("host_action_id").and_then(Value::as_str),
                ))
        });
    }
    Ok(Value::Object(object))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MapError {
    InvalidContext,
    InvalidEnvelope,
    InvalidSnapshot,
    SnapshotTooLarge,
    SnapshotNotCurrent,
    UnsupportedVersion,
    InvalidIdentity,
    InvalidGeneration,
    InvalidNode,
    InvalidEdge,
    InvalidBinding,
    InvalidDigest,
    CatalogMismatch,
}

impl std::fmt::Display for MapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidContext => "map context is malformed",
            Self::InvalidEnvelope => "map response envelope is malformed",
            Self::InvalidSnapshot => "map snapshot is malformed",
            Self::SnapshotTooLarge => "map snapshot exceeds its size bound",
            Self::SnapshotNotCurrent => "map snapshot is not complete and current",
            Self::UnsupportedVersion => "map profile or schema version is unsupported",
            Self::InvalidIdentity => "map identity is invalid",
            Self::InvalidGeneration => "map generation is invalid",
            Self::InvalidNode => "map node is invalid",
            Self::InvalidEdge => "map edge is invalid",
            Self::InvalidBinding => "map action binding is invalid",
            Self::InvalidDigest => "map snapshot digest is invalid",
            Self::CatalogMismatch => "map bindings do not match the host action catalog",
        })
    }
}

impl std::error::Error for MapError {}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn valid_identity_limit(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

#[cfg(test)]
#[path = "map_tests.rs"]
mod tests;
