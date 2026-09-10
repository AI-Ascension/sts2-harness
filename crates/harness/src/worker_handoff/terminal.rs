// SPDX-License-Identifier: MIT

use serde::ser::{SerializeMap, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::request::{TUPLE, digest, string, tuple};
use super::{HandoffError, MAX_INTEGER, WorkerRequest, json};

const RESULT: &[&str] = &[
    "status",
    "checkpoint_sequence",
    "terminal_ref",
    "result_digest",
];
const MAX_TERMINAL_BYTES: usize = 16_384;

/// A terminal record is not an ambiguous or interrupted execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Completed,
    Failed,
}

/// Result fields supplied only after the harness durably records completion.
pub struct TerminalCompletion {
    pub status: TerminalStatus,
    pub checkpoint_sequence: u64,
    pub terminal_ref: String,
    pub result_digest: String,
}

/// Closed terminal tuple. It proves structural identity, not storage durability.
#[derive(Clone)]
pub struct TerminalRecord {
    fields: Map<String, Value>,
}

impl TerminalRecord {
    /// Construct a receipt bound to a validated request's complete execution tuple.
    pub fn new(request: &WorkerRequest, result: TerminalCompletion) -> Result<Self, HandoffError> {
        let mut fields = Map::new();
        for key in TUPLE {
            fields.insert(
                (*key).to_owned(),
                request.fields().get(*key).ok_or(HandoffError)?.clone(),
            );
        }
        fields.insert(
            "status".into(),
            Value::from(match result.status {
                TerminalStatus::Completed => "completed",
                TerminalStatus::Failed => "failed",
            }),
        );
        fields.insert(
            "checkpoint_sequence".into(),
            Value::from(result.checkpoint_sequence),
        );
        fields.insert("terminal_ref".into(), Value::from(result.terminal_ref));
        fields.insert("result_digest".into(), Value::from(result.result_digest));
        Self::validated(fields)
    }

    /// Decode a retained terminal without trusting a storage row's shape or numbers.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        if bytes.len() > MAX_TERMINAL_BYTES {
            return Err(HandoffError);
        }
        let Value::Object(fields) = json::decode(bytes)? else {
            return Err(HandoffError);
        };
        Self::validated(fields)
    }

    fn validated(fields: Map<String, Value>) -> Result<Self, HandoffError> {
        tuple(&fields)?;
        if fields.len() != TUPLE.len() + RESULT.len()
            || fields
                .keys()
                .any(|key| !TUPLE.contains(&key.as_str()) && !RESULT.contains(&key.as_str()))
            || !matches!(string(&fields, "status")?, "completed" | "failed")
            || fields
                .get("checkpoint_sequence")
                .and_then(Value::as_u64)
                .is_none_or(|n| n > MAX_INTEGER)
        {
            return Err(HandoffError);
        }
        let reference = string(&fields, "terminal_ref")?;
        if reference.is_empty()
            || reference.len() > 1024
            || reference.bytes().any(|b| b < 32 || b == 127)
        {
            return Err(HandoffError);
        }
        digest(&fields, "result_digest")?;
        Ok(Self { fields })
    }

    /// Canonical terminal material, with tuple fields first in the artifact's
    /// declared order and then status/checkpoint/reference/result digest.
    pub fn encode(&self) -> Result<Vec<u8>, HandoffError> {
        let bytes = serde_json::to_vec(&OrderedTerminal(&self.fields)).map_err(|_| HandoffError)?;
        if bytes.len() > MAX_TERMINAL_BYTES {
            return Err(HandoffError);
        }
        Ok(bytes)
    }

    /// SHA-256 of the entire canonical terminal, never merely `result_digest`.
    pub fn acknowledgment_digest(&self) -> Result<String, HandoffError> {
        Ok(format!("{:x}", Sha256::digest(self.encode()?)))
    }

    /// Exact retained tuple match; no identity is rewritten after a reboot.
    pub fn matches(&self, request: &WorkerRequest) -> bool {
        TUPLE
            .iter()
            .all(|key| self.fields.get(*key) == request.fields().get(*key))
    }

    pub(super) fn value(&self) -> Value {
        Value::Object(self.fields.clone())
    }
}

struct OrderedTerminal<'a>(&'a Map<String, Value>);

impl serde::Serialize for OrderedTerminal<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(TUPLE.len() + RESULT.len()))?;
        for key in TUPLE.iter().chain(RESULT) {
            let value = self
                .0
                .get(*key)
                .ok_or_else(|| serde::ser::Error::custom("terminal field"))?;
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
