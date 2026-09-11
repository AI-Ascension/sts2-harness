// SPDX-License-Identifier: MIT

use super::analysis::MapAnalysisError;
use super::bundle::BundleHistory;
use super::canonical::{CanonicalError, reject_duplicate_keys};
use super::graph::MapGraphError;
use sha2::{Digest as _, Sha256};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapBundleError {
    InvalidField(&'static str),
    InvalidDigest(&'static str),
    DigestMismatch(&'static str),
    IdentityMismatch,
    TooLarge(&'static str),
    UnsupportedVersion(String),
    Serialization,
    Canonical(CanonicalError),
    Analysis(MapAnalysisError),
    Graph(MapGraphError),
    SnapshotSchema(&'static str),
    Storage(String),
}

impl fmt::Display for MapBundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid bundle field {field}"),
            Self::InvalidDigest(field) => write!(formatter, "invalid digest in {field}"),
            Self::DigestMismatch(field) => write!(formatter, "digest mismatch in {field}"),
            Self::IdentityMismatch => {
                formatter.write_str("bundle identity does not match analysis")
            }
            Self::TooLarge(kind) => write!(formatter, "{kind} exceeds its bound"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported bundle version {version}")
            }
            Self::Serialization => formatter.write_str("bundle serialization failed"),
            Self::Canonical(error) => error.fmt(formatter),
            Self::Analysis(error) => error.fmt(formatter),
            Self::Graph(error) => error.fmt(formatter),
            Self::SnapshotSchema(field) => {
                write!(formatter, "snapshot schema field {field} is invalid")
            }
            Self::Storage(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for MapBundleError {}

pub(crate) fn check_digest(
    name: &'static str,
    expected: &str,
    bytes: &[u8],
) -> Result<(), MapBundleError> {
    if !is_digest(expected) {
        return Err(MapBundleError::InvalidDigest(name));
    }
    let actual = crate::hex_bytes(Sha256::digest(bytes));
    if expected != actual {
        return Err(MapBundleError::DigestMismatch(name));
    }
    Ok(())
}

pub(crate) fn verify_optional_digest(
    name: &'static str,
    expected: &Option<String>,
    reference: Option<&String>,
    bytes: Option<&[u8]>,
) -> Result<(), MapBundleError> {
    match (expected, reference, bytes) {
        (None, None, None) => Ok(()),
        (Some(digest), Some(_), Some(value)) => check_digest(name, digest, value),
        _ => Err(MapBundleError::DigestMismatch(name)),
    }
}

pub(crate) fn verify_required_digest(
    name: &'static str,
    expected: &Option<String>,
    reference: &String,
    bytes: Option<&[u8]>,
) -> Result<(), MapBundleError> {
    let Some(digest) = expected else {
        return Err(MapBundleError::InvalidDigest(name));
    };
    let Some(value) = bytes else {
        return Err(MapBundleError::DigestMismatch(name));
    };
    validate_file_reference(reference)?;
    check_digest(name, digest, value)
}

pub(crate) fn validate_file_reference(value: &str) -> Result<(), MapBundleError> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(MapBundleError::InvalidField("file reference"));
    }
    Ok(())
}

pub(crate) fn validate_snapshot_document(
    bytes: &[u8],
    schema_profile: &str,
    history: &BundleHistory,
    expected_map_instance: &str,
    expected_act: &str,
) -> Result<(), MapBundleError> {
    reject_duplicate_keys(bytes).map_err(MapBundleError::Canonical)?;
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| MapBundleError::SnapshotSchema("json"))?;
    let object = value
        .as_object()
        .ok_or(MapBundleError::SnapshotSchema("object"))?;
    for field in ["generation", "projection_version", "nodes", "edges"] {
        if !object.contains_key(field) {
            return Err(MapBundleError::SnapshotSchema(field));
        }
    }
    if !object.contains_key("source_state_id") && !object.contains_key("state_id") {
        return Err(MapBundleError::SnapshotSchema("source_state_id"));
    }
    if !object.contains_key("legal_bindings") && !object.contains_key("bindings") {
        return Err(MapBundleError::SnapshotSchema("legal_bindings"));
    }
    let state_id = object
        .get("source_state_id")
        .or_else(|| object.get("state_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or(MapBundleError::SnapshotSchema("source_state_id"))?;
    if state_id != history.source_state_id {
        return Err(MapBundleError::IdentityMismatch);
    }
    let generation = object
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .ok_or(MapBundleError::SnapshotSchema("generation"))?;
    if generation != history.generation {
        return Err(MapBundleError::IdentityMismatch);
    }
    if let Some(map_instance) = object
        .get("map_instance")
        .or_else(|| object.get("map_instance_id"))
        .and_then(serde_json::Value::as_str)
        && map_instance != expected_map_instance
    {
        return Err(MapBundleError::IdentityMismatch);
    }
    let snapshot_act = object
        .get("act")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            object
                .get("act_id")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value.to_string())
        });
    if let Some(snapshot_act) = snapshot_act
        && snapshot_act != expected_act
    {
        return Err(MapBundleError::IdentityMismatch);
    }
    if schema_profile == "runtime-map-v1" {
        let schema_ok = object
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == "sts2.visible-map-v1")
            || object
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == "visible-map-v1");
        let profile_ok = object
            .get("profile")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == schema_profile)
            || object
                .get("projection_version")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == schema_profile);
        if !schema_ok || !profile_ok {
            return Err(MapBundleError::SnapshotSchema("schema/profile"));
        }
        if object.get("freshness").and_then(serde_json::Value::as_str) != Some("current") {
            return Err(MapBundleError::SnapshotSchema("freshness"));
        }
        if !object.contains_key("map_instance") && !object.contains_key("map_instance_id") {
            return Err(MapBundleError::SnapshotSchema("map_instance"));
        }
        if !object.contains_key("act") && !object.contains_key("act_id") {
            return Err(MapBundleError::SnapshotSchema("act"));
        }
    }
    Ok(())
}

pub(crate) fn validate_json_object(
    bytes: &[u8],
    field: &'static str,
) -> Result<(), MapBundleError> {
    reject_duplicate_keys(bytes).map_err(MapBundleError::Canonical)?;
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| MapBundleError::InvalidField(field))?;
    if !value.is_object() {
        return Err(MapBundleError::InvalidField(field));
    }
    Ok(())
}

pub(crate) fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
