// SPDX-License-Identifier: MIT
//! Additive private duplex wire; the frozen terminal-only Exo wire is unchanged.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::game_information::{LookupAgentInput, LookupError, LookupFeedback, LookupTurn};

pub const EXO_LOOKUP_WIRE: &str = "sts2.exo-lookup-wire-v1";
/// Additive frame pin for the bootstrap-capable provider bridge. The closed
/// terminal lookup wire above remains byte-for-byte compatible.
pub const EXO_LOOKUP_BOOTSTRAP_WIRE: &str = "sts2.exo-lookup-wire-v2-bootstrap";
pub const EXO_LOOKUP_FRAME_BYTES: usize = 196_608;
pub const EXO_LOOKUP_TOOL_BYTES: usize = 16_384;
pub const EXO_LOOKUP_FEEDBACK_BYTES: usize = 7_000;
pub const EXO_LOOKUP_CHUNK_BYTES: usize = 3_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExoLookupFrame {
    pub wire_version: String,
    pub request_id: String,
    pub turn_id: String,
    pub sequence: u64,
    pub payload: ExoLookupPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExoLookupPayload {
    Start {
        request: Value,
        optional_byte_budget: usize,
    },
    Query {
        arguments: Value,
    },
    Bootstrap {
        arguments: Value,
    },
    ReadRetained {
        record_ordinal: usize,
        offset: usize,
    },
    Feedback {
        value: Value,
    },
    Decision {
        action_id: String,
    },
    Failure {
        code: String,
    },
}

impl ExoLookupFrame {
    pub fn parse(bytes: &[u8]) -> Result<Self, LookupError> {
        if bytes.len() > EXO_LOOKUP_FRAME_BYTES {
            return Err(LookupError::Bounds);
        }
        let value = crate::game_information_validation::decode_strict(bytes)?;
        let frame: Self = serde_json::from_value(value).map_err(|_| LookupError::Invalid)?;
        if !matches!(
            frame.wire_version.as_str(),
            EXO_LOOKUP_WIRE | EXO_LOOKUP_BOOTSTRAP_WIRE
        ) || frame.sequence > 33
            || !valid_id(&frame.request_id)
            || !valid_id(&frame.turn_id)
        {
            return Err(LookupError::Invalid);
        }
        if matches!(frame.payload, ExoLookupPayload::Bootstrap { .. })
            && frame.wire_version != EXO_LOOKUP_BOOTSTRAP_WIRE
        {
            return Err(LookupError::Invalid);
        }
        if matches!(&frame.payload, ExoLookupPayload::Start { optional_byte_budget, .. }
            if *optional_byte_budget == 0 || *optional_byte_budget > 65_536)
        {
            return Err(LookupError::Bounds);
        }
        Ok(frame)
    }

    pub fn encode(&self) -> Result<Vec<u8>, LookupError> {
        let mut bytes = serde_json::to_vec(self).map_err(|_| LookupError::Invalid)?;
        Self::parse(&bytes)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn assert_identity(
        &self,
        request: &str,
        turn: &str,
        sequence: u64,
    ) -> Result<(), LookupError> {
        if self.request_id != request || self.turn_id != turn || self.sequence != sequence {
            return Err(LookupError::Scope);
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArguments {
    operation_id: String,
    mode: String,
    query: QueryParts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapArguments {
    operation_id: String,
    definition_ref: Value,
    instance_ref: Option<Value>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QueryParts {
    query_kind: String,
    entity_kind: String,
    target: QueryTarget,
    filters: Value,
    projection: String,
    detail_level: String,
    fields: Vec<String>,
    limits: Value,
    cursor: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QueryTarget {
    definition_ref: Value,
}

/// The provider cannot set owner scope, snapshot or transport identity.
pub(crate) fn query_turn(
    arguments: Value,
    input: &LookupAgentInput<'_>,
) -> Result<LookupTurn, LookupError> {
    if serde_json::to_vec(&arguments)
        .map_err(|_| LookupError::Invalid)?
        .len()
        > EXO_LOOKUP_TOOL_BYTES
    {
        return Err(LookupError::Bounds);
    }
    let args: QueryArguments =
        serde_json::from_value(arguments).map_err(|_| LookupError::Invalid)?;
    if !valid_id(&args.operation_id)
        || args.operation_id.len() > 64
        || !matches!(args.mode.as_str(), "static" | "live")
    {
        return Err(LookupError::Invalid);
    }
    let live = args.mode == "live";
    if (args.query.query_kind == "detail" && !live)
        || (live && !matches!(args.query.query_kind.as_str(), "detail" | "availability"))
    {
        return Err(LookupError::Invalid);
    }
    let snapshot = if live {
        input
            .binding
            .snapshot
            .clone()
            .ok_or(LookupError::Reobserve)?
    } else {
        Value::Null
    };
    let instance = snapshot["instance_ref"].clone();
    let mut query = serde_json::to_value(args.query).map_err(|_| LookupError::Invalid)?;
    query["target"]["instance_ref"] = instance.clone();
    query["binding"] = json!({"mode":args.mode,"content_manifest_id":input.binding.content_manifest_id,
        "locale":input.binding.locale,"visibility_scope":if live {"player"} else {"public"},
        "instance_ref":instance,"snapshot_ref":snapshot});
    query["parent_observation"] = if live {
        json!({"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":snapshot["state_generation"]})
    } else {
        Value::Null
    };
    let request = json!({"protocol_version":crate::game_information::PROFILE,
        "schema_digest":crate::game_information::SCHEMA_DIGEST,"correlation_id":"pending",
        "provenance":{"artifact":"sts2-protocol/game-information-query-v1","source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"},
        "kind":"query_request","query":query,"result":null,"capabilities":null,"error":null});
    crate::game_information_validation::validate_request(&request)?;
    Ok(LookupTurn::Query {
        operation_id: args.operation_id,
        request: serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
    })
}

pub(crate) fn bootstrap_turn(arguments: Value) -> Result<LookupTurn, LookupError> {
    if serde_json::to_vec(&arguments)
        .map_err(|_| LookupError::Invalid)?
        .len()
        > EXO_LOOKUP_TOOL_BYTES
    {
        return Err(LookupError::Bounds);
    }
    let args: BootstrapArguments =
        serde_json::from_value(arguments).map_err(|_| LookupError::Invalid)?;
    if !valid_id(&args.operation_id) || args.operation_id.len() > 64 {
        return Err(LookupError::Invalid);
    }
    let request = crate::game_information_binding::game_information_bootstrap::request(
        "pending",
        Value::Null,
        args.definition_ref,
        args.instance_ref,
    );
    Ok(LookupTurn::Bootstrap {
        operation_id: args.operation_id,
        request: serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
    })
}

/// Compact bounded feedback survives upstream's 8,000-character tool-result wrapper.
pub(crate) fn feedback_value(
    feedback: &LookupFeedback,
    budget: usize,
) -> Result<Value, LookupError> {
    let maximum = budget.min(EXO_LOOKUP_FEEDBACK_BYTES);
    if maximum == 0 {
        return Err(LookupError::Bounds);
    }
    let value = match feedback {
        LookupFeedback::Data {
            record_ordinal,
            delivery,
        } => {
            let full = json!({"record_ordinal":record_ordinal,"data":delivery.data});
            if serde_json::to_vec(&full)
                .map_err(|_| LookupError::Invalid)?
                .len()
                <= maximum
            {
                full
            } else {
                json!({"record_ordinal":record_ordinal,"data":{"authority":"untrusted_game_information_data",
                "delivery":"retained","byte_length":delivery.record.source_bytes,
                "source_sha256":delivery.record.source_sha256}})
            }
        }
        LookupFeedback::Bootstrap {
            record_ordinal,
            response,
        } => {
            let value = json!({"record_ordinal":record_ordinal,"bootstrap":response});
            if serde_json::to_vec(&value)
                .map_err(|_| LookupError::Invalid)?
                .len()
                > maximum
            {
                return Err(LookupError::Bounds);
            }
            value
        }
        LookupFeedback::Bytes {
            record_ordinal,
            offset,
            total_bytes,
            bytes,
        } => {
            if *record_ordinal >= 256
                || *total_bytes > 65_536
                || offset > total_bytes
                || bytes.len() > total_bytes - offset
            {
                return Err(LookupError::Bounds);
            }
            let mut count = bytes.len().min(EXO_LOOKUP_CHUNK_BYTES);
            loop {
                let next = offset.checked_add(count).ok_or(LookupError::Bounds)?;
                let value = json!({"record_ordinal":record_ordinal,"offset":offset,"next_offset":next,
                    "total_bytes":total_bytes,"encoding":"hex","bytes":crate::hex_bytes(&bytes[..count]),
                    "authority":"untrusted_game_information_data"});
                let size = serde_json::to_vec(&value)
                    .map_err(|_| LookupError::Invalid)?
                    .len();
                if size <= maximum {
                    if count == 0 && !bytes.is_empty() {
                        return Err(LookupError::Bounds);
                    }
                    break value;
                }
                count = count
                    .checked_sub((size - maximum).div_ceil(2))
                    .ok_or(LookupError::Bounds)?;
            }
        }
        LookupFeedback::Error(error) => json!({"error":error}),
        LookupFeedback::Start => return Err(LookupError::Invalid),
    };
    if serde_json::to_vec(&value)
        .map_err(|_| LookupError::Invalid)?
        .len()
        > maximum
    {
        return Err(LookupError::Bounds);
    }
    Ok(value)
}
