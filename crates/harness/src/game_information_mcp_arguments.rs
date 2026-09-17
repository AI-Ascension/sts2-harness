// SPDX-License-Identifier: MIT
use super::*;
use serde_json::{Map, json};

pub(super) fn arguments(context: &LookupMcpContext, request: &Value) -> Result<Value, LookupError> {
    let query = &request["query"];
    if query["binding"]["mode"] == "live"
        && query["binding"]["instance_ref"]["instance_id"] != context.instance_id
    {
        return Err(LookupError::Scope);
    }
    let mut args = Map::new();
    for (key, value) in [
        ("instance_id", &context.instance_id),
        ("mcp_session_id", &context.mcp_session_id),
        ("lease_id", &context.lease_id),
    ] {
        args.insert(key.to_owned(), json!(value));
    }
    args.insert("lease_epoch".to_owned(), json!(context.lease_epoch));
    for key in ["content_manifest_id", "locale", "visibility_scope"] {
        args.insert(key.to_owned(), query["binding"][key].clone());
    }
    for key in [
        "entity_kind",
        "projection",
        "detail_level",
        "fields",
        "cursor",
    ] {
        args.insert(key.to_owned(), query[key].clone());
    }
    for key in ["page_items", "item_bytes", "page_bytes", "text_bytes"] {
        args.insert(key.to_owned(), query["limits"][key].clone());
    }
    for key in [
        "display_name",
        "namespaced_ids",
        "definition_refs",
        "instance_ids",
    ] {
        args.insert(key.to_owned(), query["filters"][key].clone());
    }
    add_target_arguments(query, &mut args);
    Ok(Value::Object(args))
}

fn add_target_arguments(query: &Value, args: &mut Map<String, Value>) {
    if matches!(
        query["query_kind"].as_str(),
        Some("get" | "detail" | "availability")
    ) {
        args.insert(
            "definition_ref".to_owned(),
            query["target"]["definition_ref"].clone(),
        );
    }
    if matches!(
        query["query_kind"].as_str(),
        Some("detail" | "availability")
    ) {
        args.insert(
            "instance_ref".to_owned(),
            query["binding"]["instance_ref"].clone(),
        );
        args.insert(
            "snapshot_ref".to_owned(),
            query["binding"]["snapshot_ref"].clone(),
        );
        args.insert(
            "parent_observation".to_owned(),
            query["parent_observation"].clone(),
        );
    }
    if query["query_kind"] == "availability" {
        args.insert("mode".to_owned(), query["binding"]["mode"].clone());
    }
}
