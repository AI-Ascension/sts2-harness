// SPDX-License-Identifier: MIT
//! Closed, bounded consumer validation. Game text is data, never authorization.

use serde_json::Value;
use std::{collections::BTreeSet, fmt, sync::OnceLock};

#[path = "game_information_validation_item.rs"]
mod item;
#[path = "game_information_validation_order.rs"]
mod order;
#[path = "game_information_validation_json.rs"]
mod strict;
#[cfg(test)]
#[path = "game_information_validation_tests.rs"]
mod tests;

pub(crate) const SCHEMA_DIGEST: &str =
    "376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9";
pub(crate) const MAX_MESSAGE_BYTES: usize = 262_144;

pub(crate) fn validate_item_order(
    items: &[Value],
    ordering: &Value,
) -> Result<(), ValidationError> {
    order::validate(items, ordering)
}

/// Sanitized failures never include producer text or private payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ValidationError {
    Json,
    Bounds,
    Schema,
    Identity,
    Scope,
    Accounting,
    Ordering,
    Fields,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "game information validation: {self:?}")
    }
}

impl std::error::Error for ValidationError {}

pub(crate) fn decode_strict(bytes: &[u8]) -> Result<Value, ValidationError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ValidationError::Bounds);
    }
    strict::decode(bytes).map_err(|_| ValidationError::Json)
}

fn encoded_len(value: &Value) -> Result<usize, ValidationError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| ValidationError::Json)
}

fn schema(value: &Value) -> Result<(), ValidationError> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    if encoded_len(value)? > MAX_MESSAGE_BYTES {
        return Err(ValidationError::Bounds);
    }
    let validator = VALIDATOR
        .get_or_init(|| {
            let source =
                include_str!("../../../protocol-artifact/game-information-query-v1/schema.json");
            if crate::sha256_hex(source) != SCHEMA_DIGEST {
                return Err(());
            }
            let schema: Value = serde_json::from_str(source).map_err(|_| ())?;
            jsonschema::validator_for(&schema).map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| ValidationError::Schema)?;
    if !validator.is_valid(value) || value["schema_digest"] != SCHEMA_DIGEST {
        return Err(ValidationError::Schema);
    }
    if value["error"]["reason"]
        .as_str()
        .is_some_and(|reason| reason.len() > 256)
    {
        return Err(ValidationError::Bounds);
    }
    Ok(())
}

fn array(value: &Value) -> Result<&[Value], ValidationError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ValidationError::Schema)
}

fn number(value: &Value) -> Result<usize, ValidationError> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(ValidationError::Schema)
}

fn same_fence(reference: &Value, binding: &Value) -> bool {
    ["instance_id", "run_id", "epoch"]
        .iter()
        .all(|key| reference[key] == binding["instance_ref"][key])
}

fn definition(reference: &Value, query: &Value) -> Result<(), ValidationError> {
    if reference["content_manifest_id"] != query["binding"]["content_manifest_id"]
        || reference["entity_kind"] != query["entity_kind"]
    {
        return Err(ValidationError::Identity);
    }
    Ok(())
}

fn query_semantics(query: &Value) -> Result<(), ValidationError> {
    if query["filters"]["display_name"]
        .as_str()
        .is_some_and(|text| text.len() > 1024)
    {
        return Err(ValidationError::Bounds);
    }
    let binding = &query["binding"];
    let live = binding["mode"] == "live";
    if (!live && binding["visibility_scope"] != "public")
        || (live && binding["visibility_scope"] != "player")
    {
        return Err(ValidationError::Scope);
    }
    if live
        && (binding["snapshot_ref"]["instance_ref"] != binding["instance_ref"]
            || query["parent_observation"]["instance_ref"] != binding["instance_ref"]
            || query["parent_observation"]["snapshot_ref"] != binding["snapshot_ref"]
            || query["parent_observation"]["state_generation"]
                != binding["snapshot_ref"]["state_generation"])
    {
        return Err(ValidationError::Identity);
    }
    let target = &query["target"];
    if !target["definition_ref"].is_null() {
        definition(&target["definition_ref"], query)?;
    }
    if !target["instance_ref"].is_null()
        && (!live
            || target["instance_ref"] != binding["instance_ref"]
            || target["instance_ref"]["entity_kind"] != query["entity_kind"])
    {
        return Err(ValidationError::Identity);
    }
    if matches!(query["query_kind"].as_str(), Some("get" | "detail"))
        && target["definition_ref"].is_null()
        && target["instance_ref"].is_null()
    {
        return Err(ValidationError::Identity);
    }
    for reference in array(&query["filters"]["definition_refs"])? {
        definition(reference, query)?;
    }
    if !live && !array(&query["filters"]["instance_ids"])?.is_empty() {
        return Err(ValidationError::Scope);
    }
    for name in ["namespaced_ids", "definition_refs", "instance_ids"] {
        let values = array(&query["filters"][name])?;
        let mut seen = BTreeSet::new();
        for value in values {
            if !seen.insert(value.to_string()) {
                return Err(ValidationError::Identity);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_request(request: &Value) -> Result<(), ValidationError> {
    schema(request)?;
    if request["kind"] != "query_request" {
        return Err(ValidationError::Schema);
    }
    query_semantics(&request["query"])
}

pub(crate) fn validate_capabilities(value: &Value) -> Result<(), ValidationError> {
    schema(value)?;
    if value["kind"] != "capabilities_response" {
        return Err(ValidationError::Schema);
    }
    Ok(())
}

/// Validate the entire envelope before a caller selects or projects any fields.
pub(crate) fn validate_response(request: &Value, response: &Value) -> Result<(), ValidationError> {
    validate_request(request)?;
    schema(response)?;
    if response["correlation_id"] != request["correlation_id"] {
        return Err(ValidationError::Identity);
    }
    if response["kind"] == "error_response" {
        if !response["query"].is_null() && response["query"] != request["query"] {
            return Err(ValidationError::Identity);
        }
        return Ok(());
    }
    if response["kind"] != "query_response" || response["query"] != request["query"] {
        return Err(ValidationError::Identity);
    }
    let query = &request["query"];
    let result = &response["result"];
    if result["parent_observation"] != query["parent_observation"]
        || (query["binding"]["mode"] == "live"
            && result["result_generation"] != query["binding"]["snapshot_ref"]["state_generation"])
    {
        return Err(ValidationError::Identity);
    }
    let page = &result["page"];
    if page["limits"] != query["limits"] {
        return Err(ValidationError::Bounds);
    }
    if !page["cursor_binding"].is_null() {
        let mut expected = query.clone();
        expected
            .as_object_mut()
            .ok_or(ValidationError::Schema)?
            .remove("cursor");
        if expected != page["cursor_binding"] {
            return Err(ValidationError::Identity);
        }
    }
    let items = array(&page["items"])?;
    let mut identities = BTreeSet::new();
    let mut max_item_bytes = 0;
    let mut text_bytes = 0;
    for item in items {
        item::validate(query, item, &mut text_bytes)?;
        let key = if query["binding"]["mode"] == "live" {
            &item["instance_ref"]
        } else {
            &item["definition_ref"]
        };
        if !identities.insert(key.to_string()) {
            return Err(ValidationError::Identity);
        }
        max_item_bytes = max_item_bytes.max(encoded_len(item)?);
    }
    order::validate(items, &page["ordering"])?;
    let mut bare_page = page.clone();
    bare_page
        .as_object_mut()
        .ok_or(ValidationError::Schema)?
        .remove("accounting");
    for (key, actual) in [
        ("item_count", items.len()),
        ("item_bytes", max_item_bytes),
        ("payload_bytes", encoded_len(&page["items"])?),
        ("page_bytes", encoded_len(&bare_page)?),
        ("text_bytes", text_bytes),
    ] {
        if number(&page["accounting"][key])? != actual {
            return Err(ValidationError::Accounting);
        }
    }
    for (key, actual) in [
        ("page_items", items.len()),
        ("item_bytes", max_item_bytes),
        ("page_bytes", encoded_len(&bare_page)?),
        ("text_bytes", text_bytes),
    ] {
        if actual > number(&query["limits"][key])? {
            return Err(ValidationError::Bounds);
        }
    }
    if (page["total_count_known"] == true && number(&page["total_count"])? < items.len())
        || (page["total_count_known"] == true
            && page["final_page"] == true
            && page["coverage"] == "complete"
            && query["cursor"].is_null()
            && number(&page["total_count"])? != items.len())
        || (matches!(
            page["coverage"].as_str(),
            Some("unavailable" | "not_observable")
        ) && !items.is_empty())
        || (page["final_page"] == false && items.is_empty())
    {
        return Err(ValidationError::Accounting);
    }
    Ok(())
}
