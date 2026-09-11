// SPDX-License-Identifier: MIT

use serde_json::Value;

/// Rebuilds the fixed wire order after semantic validation.
///
/// Comparing this representation with the original text rejects duplicate members, reordered
/// members, alternate number spellings, insignificant whitespace, and an incorrect terminator.
pub(super) fn canonical_response(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut result = String::new();
    result.push('{');
    let mut first = true;
    append_value(
        &mut result,
        &mut first,
        "protocol_version",
        object.get("protocol_version")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "schema_digest",
        object.get("schema_digest")?,
    )?;
    append_object(
        &mut result,
        &mut first,
        "provenance",
        object.get("provenance")?,
        provenance,
    )?;
    append_value(
        &mut result,
        &mut first,
        "correlation_id",
        object.get("correlation_id")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "instance_id",
        object.get("instance_id")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "session_id",
        object.get("session_id")?,
    )?;
    append_value(&mut result, &mut first, "lease_id", object.get("lease_id")?)?;
    append_value(
        &mut result,
        &mut first,
        "lease_epoch",
        object.get("lease_epoch")?,
    )?;
    append_value(&mut result, &mut first, "kind", object.get("kind")?)?;
    append_value(
        &mut result,
        &mut first,
        "operation_id",
        object.get("operation_id")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "action_kind",
        object.get("action_kind")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "action_fingerprint",
        object.get("action_fingerprint")?,
    )?;
    append_value(&mut result, &mut first, "run_id", object.get("run_id")?)?;
    append_object(
        &mut result,
        &mut first,
        "location",
        object.get("location")?,
        location,
    )?;
    append_value(&mut result, &mut first, "actor_id", object.get("actor_id")?)?;
    append_value(
        &mut result,
        &mut first,
        "authority_id",
        object.get("authority_id")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "authority_epoch",
        object.get("authority_epoch")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "expected_host_generation",
        object.get("expected_host_generation")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "before_host_generation",
        object.get("before_host_generation")?,
    )?;
    append_value(
        &mut result,
        &mut first,
        "participant_ids",
        object.get("participant_ids")?,
    )?;
    append_value(&mut result, &mut first, "status", object.get("status")?)?;
    append_value(
        &mut result,
        &mut first,
        "evidence_scope",
        object.get("evidence_scope")?,
    )?;
    let receipt = object.get("receipt")?;
    if receipt.is_null() {
        append_value(&mut result, &mut first, "receipt", receipt)?;
    } else {
        append_object(&mut result, &mut first, "receipt", receipt, receipt_object)?;
    }
    append_value(
        &mut result,
        &mut first,
        "error_code",
        object.get("error_code")?,
    )?;
    result.push('}');
    result.push('\n');
    Some(result)
}

fn append_value(result: &mut String, first: &mut bool, name: &str, value: &Value) -> Option<()> {
    append_prefix(result, first, name)?;
    result.push_str(&serde_json::to_string(value).ok()?);
    Some(())
}

fn append_object(
    result: &mut String,
    first: &mut bool,
    name: &str,
    value: &Value,
    write: fn(&mut String, &Value) -> Option<()>,
) -> Option<()> {
    append_prefix(result, first, name)?;
    write(result, value)
}

fn append_prefix(result: &mut String, first: &mut bool, name: &str) -> Option<()> {
    if !*first {
        result.push(',');
    }
    *first = false;
    result.push_str(&serde_json::to_string(name).ok()?);
    result.push(':');
    Some(())
}

fn provenance(result: &mut String, value: &Value) -> Option<()> {
    let object = value.as_object()?;
    result.push('{');
    let mut first = true;
    append_value(result, &mut first, "artifact", object.get("artifact")?)?;
    append_value(result, &mut first, "source", object.get("source")?)?;
    append_value(result, &mut first, "generator", object.get("generator")?)?;
    result.push('}');
    Some(())
}

fn location(result: &mut String, value: &Value) -> Option<()> {
    let object = value.as_object()?;
    result.push('{');
    let mut first = true;
    append_value(result, &mut first, "act_index", object.get("act_index")?)?;
    append_value(result, &mut first, "room_id", object.get("room_id")?)?;
    let coordinate = object.get("coord")?;
    if coordinate.is_null() {
        append_value(result, &mut first, "coord", coordinate)?;
    } else {
        append_object(result, &mut first, "coord", coordinate, coordinate_object)?;
    }
    result.push('}');
    Some(())
}

fn coordinate_object(result: &mut String, value: &Value) -> Option<()> {
    let object = value.as_object()?;
    result.push('{');
    let mut first = true;
    append_value(result, &mut first, "col", object.get("col")?)?;
    append_value(result, &mut first, "row", object.get("row")?)?;
    result.push('}');
    Some(())
}

fn receipt_object(result: &mut String, value: &Value) -> Option<()> {
    let object = value.as_object()?;
    result.push('{');
    let mut first = true;
    append_value(result, &mut first, "status", object.get("status")?)?;
    append_value(
        result,
        &mut first,
        "after_host_generation",
        object.get("after_host_generation")?,
    )?;
    append_value(
        result,
        &mut first,
        "checkpoint_id",
        object.get("checkpoint_id")?,
    )?;
    append_value(
        result,
        &mut first,
        "state_digest",
        object.get("state_digest")?,
    )?;
    append_value(result, &mut first, "effect_id", object.get("effect_id")?)?;
    append_value(
        result,
        &mut first,
        "effect_kind",
        object.get("effect_kind")?,
    )?;
    append_value(result, &mut first, "error_code", object.get("error_code")?)?;
    result.push('}');
    Some(())
}
