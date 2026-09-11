// SPDX-License-Identifier: MIT

//! Bounded, canonical decision bytes used when a durable provider result is replayed.

use serde_json::json;
use sts2_harness::{Decision, parse_decision};

const MAX_DECISION_RESULT_BYTES: usize = 8 * 1024;

pub(super) fn encode(decision: &Decision) -> Result<(Vec<u8>, String), String> {
    let value = match decision {
        Decision::Plan {
            action_ids,
            rationale,
        } => json!({
            "decision": "plan",
            "action_ids": action_ids,
            "rationale": rationale,
        }),
        Decision::Action {
            action_id,
            rationale,
            confidence,
        } => {
            let mut value = json!({
                "decision": "action",
                "action_id": action_id,
                "rationale": rationale,
            });
            if let Some(confidence) = confidence {
                value["confidence"] = json!(confidence);
            }
            value
        }
        Decision::Wait { rationale } => json!({
            "decision": "wait",
            "rationale": rationale,
        }),
        Decision::Reobserve { rationale } => json!({
            "decision": "reobserve",
            "rationale": rationale,
        }),
        Decision::Recovery {
            kind,
            operation_id,
            rationale,
        } => {
            let mut value = json!({
                "decision": "recovery",
                "recovery_kind": kind,
                "rationale": rationale,
            });
            if let Some(operation_id) = operation_id {
                value["operation_id"] = json!(operation_id);
            }
            value
        }
    };
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| format!("cannot encode runtime-v3 decision result: {error}"))?;
    if bytes.is_empty() || bytes.len() > MAX_DECISION_RESULT_BYTES {
        return Err(String::from(
            "runtime-v3 decision result exceeds its bounded replay size",
        ));
    }
    // Encode through the production parser as well, so only the exact bounded decision contract
    // can become durable replay data.
    parse_decision(&bytes)
        .map_err(|error| format!("runtime-v3 decision result is not replayable: {error}"))?;
    let digest = sts2_harness::sha256_hex(&bytes);
    Ok((bytes, digest))
}

pub(super) fn decode(payload: &[u8], digest: &str) -> Result<Decision, String> {
    if payload.is_empty() || payload.len() > MAX_DECISION_RESULT_BYTES {
        return Err(String::from(
            "stored runtime-v3 decision result has an invalid size",
        ));
    }
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || sts2_harness::sha256_hex(payload) != digest
    {
        return Err(String::from(
            "stored runtime-v3 decision result digest does not match its bytes",
        ));
    }
    parse_decision(payload)
        .map_err(|error| format!("stored runtime-v3 decision result is invalid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_every_decision_variant() -> Result<(), String> {
        let decisions = [
            Decision::Plan {
                action_ids: vec![String::from("combat.end-turn")],
                rationale: String::from("plan"),
            },
            Decision::Action {
                action_id: String::from("combat.end-turn"),
                rationale: String::from("act"),
                confidence: Some(90),
            },
            Decision::Wait {
                rationale: String::from("wait"),
            },
            Decision::Reobserve {
                rationale: String::from("observe"),
            },
            Decision::Recovery {
                kind: String::from("reconcile"),
                operation_id: Some(String::from("operation-1")),
                rationale: String::from("recover"),
            },
        ];
        for expected in decisions {
            let (payload, digest) = encode(&expected)?;
            assert_eq!(decode(&payload, &digest)?, expected);
        }
        Ok(())
    }

    #[test]
    fn decode_rejects_tampering_and_unknown_fields() -> Result<(), String> {
        let (mut payload, digest) = encode(&Decision::Wait {
            rationale: String::from("wait"),
        })?;
        payload[2] ^= 1;
        assert!(decode(&payload, &digest).is_err());
        assert!(
            decode(
                br#"{"decision":"wait","rationale":"wait","extra":1}"#,
                &sts2_harness::sha256_hex(br#"{"decision":"wait","rationale":"wait","extra":1}"#,),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn encode_never_retains_json_values() -> Result<(), String> {
        let (payload, _) = encode(&Decision::Action {
            action_id: String::from("combat.end-turn"),
            rationale: String::from("act"),
            confidence: None,
        })?;
        let value: serde_json::Value =
            serde_json::from_slice(&payload).map_err(|error| error.to_string())?;
        assert_eq!(value["decision"], "action");
        assert_eq!(value["action_id"], "combat.end-turn");
        Ok(())
    }
}
