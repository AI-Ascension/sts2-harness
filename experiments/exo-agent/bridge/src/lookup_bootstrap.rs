// SPDX-License-Identifier: MIT
//! Validation and framing for the explicitly selected native bootstrap profile.

use crate::lookup_wire::{self as wire, Payload};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BootstrapArguments {
    operation_id: String,
    definition_ref: Value,
    instance_ref: Option<Value>,
}

pub(super) fn payload(arguments: Value) -> Result<Payload, &'static str> {
    let bootstrap: BootstrapArguments =
        serde_json::from_value(arguments).map_err(|_| "exo_lookup_arguments")?;
    if !wire::valid_id(&bootstrap.operation_id)
        || bootstrap.operation_id.len() > 64
        || !valid_definition_ref(&bootstrap.definition_ref)
        || !bootstrap
            .instance_ref
            .as_ref()
            .is_none_or(valid_instance_ref)
    {
        return Err("exo_lookup_arguments");
    }
    Ok(Payload::Bootstrap {
        arguments: serde_json::to_value(bootstrap).map_err(|_| "exo_lookup_arguments")?,
    })
}

fn valid_identity(value: &Value) -> bool {
    value.as_str().is_some_and(wire::valid_id)
}

fn valid_definition_ref(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 4
        && object.get("content_manifest_id").is_some_and(valid_identity)
        && object
            .get("entity_kind")
            .and_then(Value::as_str)
            .is_some_and(valid_entity_kind)
        && object.get("namespaced_id").is_some_and(valid_identity)
        && object
            .get("variant")
            .is_some_and(|variant| variant.is_null() || valid_identity(variant))
}

fn valid_instance_ref(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 5
        && object.get("entity_id").is_some_and(valid_identity)
        && object
            .get("entity_kind")
            .and_then(Value::as_str)
            .is_some_and(valid_entity_kind)
        && object
            .get("epoch")
            .and_then(Value::as_u64)
            .is_some_and(|epoch| (1..=u64::from(u32::MAX)).contains(&epoch))
        && object.get("instance_id").is_some_and(valid_identity)
        && object.get("run_id").is_some_and(valid_identity)
}

fn valid_entity_kind(kind: &str) -> bool {
    matches!(
        kind,
        "card"
            | "character"
            | "enemy"
            | "event"
            | "map_node"
            | "potion"
            | "power"
            | "relic"
            | "room"
            | "status"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup_wire::{self as wire, Frame};
    use exoharness::ToolRequest;
    use serde_json::json;

    fn make_request(instance_ref: Value) -> Result<ToolRequest, serde_json::Error> {
        serde_json::from_value(json!({
            "function_name":"sts2_lookup_bootstrap",
            "arguments":{
                "operation_id":"bootstrap-1",
                "definition_ref":{
                    "content_manifest_id":"content-1","entity_kind":"card",
                    "namespaced_id":"ironclad:strike","variant":null
                },
                "instance_ref":instance_ref
            }
        }))
    }

    #[test]
    fn bootstrap_is_forwarded_on_additive_wire_with_exact_selector_bounds()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = make_request(json!({
            "entity_id":"card-17","entity_kind":"card","epoch":7,
            "instance_id":"instance-1","run_id":"run-42"
        }))?;
        let payload = super::payload(Value::Object(request.arguments.clone()))?;
        assert_eq!(
            wire::version_for_payload(&payload),
            wire::BOOTSTRAP_VERSION
        );
        assert!(matches!(payload, Payload::Bootstrap { .. }));

        let mut foreign = request.clone();
        foreign.arguments["definition_ref"]["namespaced_id"] = json!("*");
        assert!(super::payload(Value::Object(foreign.arguments)).is_err());

        let wildcard = make_request(json!({}))?;
        assert!(super::payload(Value::Object(wildcard.arguments)).is_err());
        Ok(())
    }

    #[test]
    fn bootstrap_tool_round_trips_through_v2_duplex_feedback()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = make_request(Value::Null)?;
        let payload = super::payload(Value::Object(request.arguments))?;
        let frame = Frame {
            wire_version: wire::version_for_payload(&payload).into(),
            request_id: "request".into(),
            turn_id: "turn".into(),
            sequence: 1,
            payload,
        };
        let encoded = serde_json::to_vec(&frame)?;
        let decoded = wire::decode_frame(&encoded)?;
        assert!(matches!(decoded.payload, Payload::Bootstrap { .. }));

        let response = Frame {
            wire_version: wire::BOOTSTRAP_VERSION.into(),
            request_id: "request".into(),
            turn_id: "turn".into(),
            sequence: 1,
            payload: Payload::Feedback {
                value: json!({"kind":"bootstrap_response","visible_entities":[]}),
            },
        };
        let value = wire::feedback_for_version(
            &serde_json::to_vec(&response)?,
            "request",
            "turn",
            1,
            wire::BOOTSTRAP_VERSION,
        )?;
        assert_eq!(value["kind"], "bootstrap_response");
        Ok(())
    }
}
