// SPDX-License-Identifier: MIT

//! Closed Exo capability and preflight contract (issue #139).
//!
//! A bridge reports exactly what it supports before any model or game effect. Preflight is a pure,
//! non-inferencing check: it never contacts a model. Unknown keys, unsupported schemas, wrong
//! revisions, swapped package bytes, unsupported decision kinds/projections/context modes, and
//! out-of-bound limits all fail closed.

use crate::exo::protocol::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};

/// The only accepted capability descriptor schema.
pub const EXO_CAPABILITY_SCHEMA: &str = "sts2.exo-capability-v1";
/// The only accepted bridge contract version for this revision of the contract.
pub const EXO_CONTRACT_VERSION: &str = "sts2.exo-bridge-v1";
/// Absolute exchange deadline bound, mirrored from the runtime settings.
pub const EXO_MAX_TURN_MILLIS: u32 = 120_000;
/// Absolute decision size bound, mirrored from the strict decision parser.
const EXO_MAX_DECISION_BYTES: usize = 8 * 1024;

const DECISION_KINDS: [&str; 5] = ["action", "plan", "wait", "reobserve", "recovery"];
const PROJECTIONS: [&str; 3] = ["standard", "map", "expert"];
const CONTEXT_MODES: [&str; 2] = ["fresh", "managed"];
const PLATFORMS: [&str; 1] = ["linux"];
const EVIDENCE_STATES: [&str; 3] = ["synthetic", "live", "unverified"];
const REQUIRED_DECISION_KINDS: [&str; 5] = ["action", "plan", "wait", "reobserve", "recovery"];

const TOP_LEVEL_KEYS: [&str; 11] = [
    "schema",
    "contract_version",
    "provider_revision",
    "package_digest",
    "decision_kinds",
    "projections",
    "context_modes",
    "platform",
    "limits",
    "lifecycle",
    "evidence",
];
const LIMIT_KEYS: [&str; 5] = [
    "max_request_bytes",
    "max_map_request_bytes",
    "max_decision_bytes",
    "max_turn_millis",
    "max_concurrency",
];
const LIFECYCLE_KEYS: [&str; 3] = ["cancellation", "restart_recovery", "idempotent_replay"];

#[path = "capability_error.rs"]
mod error;

pub use error::ExoPreflightError;

/// Trusted operator expectations. Derived from approved configuration, not from the bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExoPreflightExpectation<'a> {
    pub provider_revision: &'a str,
    pub package_digest: &'a str,
    pub platform: &'a str,
    pub required_projection: &'a str,
}

/// Nested resource limits declared by a bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoCapabilityLimits {
    pub max_request_bytes: u64,
    pub max_map_request_bytes: u64,
    pub max_decision_bytes: u64,
    pub max_turn_millis: u64,
    pub max_concurrency: u64,
}

/// Nested lifecycle support declared by a bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoLifecycleSupport {
    pub cancellation: bool,
    pub restart_recovery: bool,
    pub idempotent_replay: bool,
}

/// Validated capability descriptor. Only [preflight] constructs one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoCapabilityDescriptor {
    pub contract_version: String,
    pub provider_revision: String,
    pub package_digest: String,
    pub decision_kinds: Vec<String>,
    pub projections: Vec<String>,
    pub context_modes: Vec<String>,
    pub platform: String,
    pub limits: ExoCapabilityLimits,
    pub lifecycle: ExoLifecycleSupport,
    pub evidence: String,
}

/// Validates one bounded descriptor against trusted expectations. Never calls a model.
pub fn preflight(
    bytes: &[u8],
    expectation: &ExoPreflightExpectation<'_>,
) -> Result<ExoCapabilityDescriptor, ExoPreflightError> {
    if bytes.len() > 256 * 1024 {
        return Err(ExoPreflightError::Malformed);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ExoPreflightError::Malformed)?;
    let object = value.as_object().ok_or(ExoPreflightError::Malformed)?;

    for key in object.keys() {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            return Err(ExoPreflightError::UnknownField);
        }
    }
    for key in TOP_LEVEL_KEYS {
        if !object.contains_key(key) {
            return Err(ExoPreflightError::MissingField);
        }
    }

    let schema = string_field(object, "schema")?;
    if schema != EXO_CAPABILITY_SCHEMA {
        return Err(ExoPreflightError::UnsupportedSchema);
    }
    let contract_version = string_field(object, "contract_version")?;
    if contract_version != EXO_CONTRACT_VERSION {
        return Err(ExoPreflightError::UnsupportedContract);
    }

    let provider_revision = string_field(object, "provider_revision")?;
    if !valid_revision(provider_revision) {
        return Err(ExoPreflightError::UnsupportedValue);
    }
    if provider_revision != expectation.provider_revision {
        return Err(ExoPreflightError::WrongRevision);
    }

    let package_digest = string_field(object, "package_digest")?;
    if !valid_digest(package_digest) {
        return Err(ExoPreflightError::UnsupportedValue);
    }
    if package_digest != expectation.package_digest {
        return Err(ExoPreflightError::SwappedPackage);
    }

    let platform = string_field(object, "platform")?;
    if !PLATFORMS.contains(&platform) || platform != expectation.platform {
        return Err(ExoPreflightError::UnsupportedPlatform);
    }

    let decision_kinds = string_list(object, "decision_kinds")?;
    if decision_kinds
        .iter()
        .any(|kind| !DECISION_KINDS.contains(&kind.as_str()))
    {
        return Err(ExoPreflightError::UnsupportedDecisionKind);
    }
    if REQUIRED_DECISION_KINDS
        .iter()
        .any(|required| !decision_kinds.iter().any(|kind| kind == required))
    {
        return Err(ExoPreflightError::UnsupportedDecisionKind);
    }

    let projections = string_list(object, "projections")?;
    if projections
        .iter()
        .any(|projection| !PROJECTIONS.contains(&projection.as_str()))
    {
        return Err(ExoPreflightError::UnsupportedProjection);
    }
    if !projections
        .iter()
        .any(|projection| projection == expectation.required_projection)
    {
        return Err(ExoPreflightError::UnsupportedProjection);
    }

    let context_modes = string_list(object, "context_modes")?;
    if context_modes.is_empty()
        || context_modes
            .iter()
            .any(|mode| !CONTEXT_MODES.contains(&mode.as_str()))
        || !context_modes.iter().any(|mode| mode == "fresh")
    {
        return Err(ExoPreflightError::UnsupportedContextMode);
    }

    let evidence = string_field(object, "evidence")?;
    if !EVIDENCE_STATES.contains(&evidence) {
        return Err(ExoPreflightError::UnsupportedEvidence);
    }

    let limits = limits(object.get("limits"))?;
    if limits.max_request_bytes == 0
        || limits.max_request_bytes > EXO_MAX_STANDARD_REQUEST_BYTES as u64
        || limits.max_map_request_bytes == 0
        || limits.max_map_request_bytes > EXO_MAX_MAP_REQUEST_BYTES as u64
        || limits.max_decision_bytes == 0
        || limits.max_decision_bytes > EXO_MAX_DECISION_BYTES as u64
        || limits.max_turn_millis == 0
        || limits.max_turn_millis > EXO_MAX_TURN_MILLIS as u64
        || limits.max_concurrency == 0
    {
        return Err(ExoPreflightError::LimitExceeded);
    }

    let lifecycle = lifecycle(object.get("lifecycle"))?;

    Ok(ExoCapabilityDescriptor {
        contract_version: contract_version.to_owned(),
        provider_revision: provider_revision.to_owned(),
        package_digest: package_digest.to_owned(),
        decision_kinds,
        projections,
        context_modes,
        platform: platform.to_owned(),
        limits,
        lifecycle,
        evidence: evidence.to_owned(),
    })
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, ExoPreflightError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoPreflightError::UnsupportedValue)
}

fn string_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, ExoPreflightError> {
    let values = object
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or(ExoPreflightError::UnsupportedValue)?;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let text = value.as_str().ok_or(ExoPreflightError::UnsupportedValue)?;
        out.push(text.to_owned());
    }
    Ok(out)
}

fn limits(value: Option<&serde_json::Value>) -> Result<ExoCapabilityLimits, ExoPreflightError> {
    let object = value
        .and_then(serde_json::Value::as_object)
        .ok_or(ExoPreflightError::UnsupportedValue)?;
    for key in object.keys() {
        if !LIMIT_KEYS.contains(&key.as_str()) {
            return Err(ExoPreflightError::UnknownField);
        }
    }
    for key in LIMIT_KEYS {
        if !object.contains_key(key) {
            return Err(ExoPreflightError::MissingField);
        }
    }
    Ok(ExoCapabilityLimits {
        max_request_bytes: u64_field(object, "max_request_bytes")?,
        max_map_request_bytes: u64_field(object, "max_map_request_bytes")?,
        max_decision_bytes: u64_field(object, "max_decision_bytes")?,
        max_turn_millis: u64_field(object, "max_turn_millis")?,
        max_concurrency: u64_field(object, "max_concurrency")?,
    })
}

fn u64_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u64, ExoPreflightError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or(ExoPreflightError::UnsupportedValue)
}

fn lifecycle(value: Option<&serde_json::Value>) -> Result<ExoLifecycleSupport, ExoPreflightError> {
    let object = value
        .and_then(serde_json::Value::as_object)
        .ok_or(ExoPreflightError::UnsupportedValue)?;
    for key in object.keys() {
        if !LIFECYCLE_KEYS.contains(&key.as_str()) {
            return Err(ExoPreflightError::UnknownField);
        }
    }
    for key in LIFECYCLE_KEYS {
        if !object.contains_key(key) {
            return Err(ExoPreflightError::MissingField);
        }
    }
    Ok(ExoLifecycleSupport {
        cancellation: bool_field(object, "cancellation")?,
        restart_recovery: bool_field(object, "restart_recovery")?,
        idempotent_replay: bool_field(object, "idempotent_replay")?,
    })
}

fn bool_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<bool, ExoPreflightError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or(ExoPreflightError::UnsupportedValue)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

#[cfg(test)]
#[path = "capability_tests.rs"]
mod tests;
