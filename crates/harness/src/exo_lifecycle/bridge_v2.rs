// SPDX-License-Identifier: MIT

//! Opt-in receipt-carrying bridge wire. V1 readers remain unchanged.

use super::NativeIdentity;
use crate::exo::parse_strict_value;
use crate::{Decision, ExoWireError, parse_bridge_decision};
use serde::Deserialize;
use serde_json::Value;

pub const EXO_LIFECYCLE_WIRE_V2: &str = "sts2.exo-bridge-wire-v2";
const MAX_RESPONSE_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExoLifecycleResponse {
    pub wire_version: String,
    pub request_id: String,
    pub turn_id: String,
    pub outcome: LifecycleOutcome,
    pub decision: Option<Value>,
    pub error_code: Option<String>,
    pub native: Option<NativeIdentity>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOutcome {
    Decision,
    Cancelled,
    Failed,
}

/// Parses exactly one v2 terminal envelope and rejects a response whose receipt does not bind the
/// host request/turn pair. A caller must retain `native` in its durable owner before reuse.
pub fn parse_lifecycle_response(
    bytes: &[u8],
    request_id: &str,
    turn_id: &str,
) -> Result<(Decision, NativeIdentity), ExoWireError> {
    if bytes.is_empty() || bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ExoWireError::TooLarge);
    }
    let value = parse_strict_value(bytes)?;
    let response: ExoLifecycleResponse =
        serde_json::from_value(value).map_err(|_| ExoWireError::InvalidShape)?;
    if response.wire_version != EXO_LIFECYCLE_WIRE_V2 {
        return Err(ExoWireError::VersionMismatch);
    }
    if response.request_id != request_id || response.turn_id != turn_id {
        return Err(ExoWireError::IdentityMismatch);
    }
    if response.outcome != LifecycleOutcome::Decision || response.error_code.is_some() {
        return Err(ExoWireError::RemoteFailure);
    }
    let native = response.native.ok_or(ExoWireError::InvalidIdentity)?;
    if !native.valid() {
        return Err(ExoWireError::InvalidIdentity);
    }
    let decision = response.decision.ok_or(ExoWireError::InvalidShape)?;
    let decision = serde_json::to_vec(&decision).map_err(|_| ExoWireError::InvalidShape)?;
    Ok((parse_bridge_decision(&decision)?, native))
}
