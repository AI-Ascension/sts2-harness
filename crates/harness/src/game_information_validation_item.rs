// SPDX-License-Identifier: MIT
use super::{ValidationError, array, definition, same_fence};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn validate(
    query: &Value,
    item: &Value,
    text_bytes: &mut usize,
) -> Result<(), ValidationError> {
    definition(&item["definition_ref"], query)?;
    let live = query["binding"]["mode"] == "live";
    if (live
        && (item["instance_ref"].is_null()
            || item["instance_ref"] != query["binding"]["instance_ref"]
            || item["instance_ref"]["entity_kind"] != query["entity_kind"]))
        || (!live && !item["instance_ref"].is_null())
    {
        return Err(ValidationError::Identity);
    }
    for key in ["definition_ref", "instance_ref"] {
        if !query["target"][key].is_null() && query["target"][key] != item[key] {
            return Err(ValidationError::Identity);
        }
    }
    for (filter, actual) in [
        ("definition_refs", &item["definition_ref"]),
        ("namespaced_ids", &item["definition_ref"]["namespaced_id"]),
        ("instance_ids", &item["instance_ref"]["entity_id"]),
    ] {
        let selected = array(&query["filters"][filter])?;
        if !selected.is_empty() && !selected.contains(actual) {
            return Err(ValidationError::Identity);
        }
    }
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for field in array(&item["fields"])? {
        let name = field["name"].as_str().ok_or(ValidationError::Fields)?;
        if !seen.insert(name) || previous.is_some_and(|previous| previous >= name) {
            return Err(ValidationError::Fields);
        }
        previous = Some(name);
        if field["reason"]
            .as_str()
            .is_some_and(|reason| reason.len() > 256)
        {
            return Err(ValidationError::Bounds);
        }
        let source = &field["source"];
        if !source["ref"].is_null()
            && ((source["kind"] == "content_manifest"
                && source["ref"] != query["binding"]["content_manifest_id"])
                || (matches!(
                    source["kind"].as_str(),
                    Some("game_mod" | "gateway_projection")
                ) && source["ref"] != query["binding"]["instance_ref"]["instance_id"]))
        {
            return Err(ValidationError::Identity);
        }
        // Closed schema forbids seed/hidden-state keys and invented field names.
        // Localized names, descriptions and reasons never establish scope or authority.
        if !live
            && (field["kind"] == "instance_ref"
                || matches!(
                    field["source"]["kind"].as_str(),
                    Some("game_mod" | "gateway_projection")
                ))
        {
            return Err(ValidationError::Scope);
        }
        if field["availability"] != "available" {
            continue;
        }
        match field["kind"].as_str() {
            Some("text") => {
                let length = field["value"]
                    .as_str()
                    .ok_or(ValidationError::Fields)?
                    .len();
                if length > 1024 {
                    return Err(ValidationError::Bounds);
                }
                *text_bytes += length;
            }
            Some("text_list") => {
                for text in array(&field["value"])? {
                    let length = text.as_str().ok_or(ValidationError::Fields)?.len();
                    if length > 1024 {
                        return Err(ValidationError::Bounds);
                    }
                    *text_bytes += length;
                }
            }
            Some("definition_ref")
                if field["value"]["content_manifest_id"]
                    != query["binding"]["content_manifest_id"] =>
            {
                return Err(ValidationError::Identity);
            }
            Some("instance_ref") if !live || !same_fence(&field["value"], &query["binding"]) => {
                return Err(ValidationError::Identity);
            }
            _ => {}
        }
    }
    for requested in array(&query["fields"])? {
        if !seen.contains(requested.as_str().ok_or(ValidationError::Fields)?) {
            return Err(ValidationError::Fields);
        }
    }
    Ok(())
}
