// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::contract::{MAX_JSON_BYTES, digest_value, validate_identifier};
use super::super::contract_authoring::{
    STUDIO_SCHEMA_VERSION, StudioDefinitionRecord, StudioDraftConflict, StudioDraftRecord,
};
use super::super::store::StoreError;

pub(super) fn draft_record(
    draft_id: &str,
    definition_id: &str,
    revision: u64,
    document: Value,
    layout: Value,
) -> Result<StudioDraftRecord, StoreError> {
    validate_id("draft_id", draft_id)?;
    validate_id("definition_id", definition_id)?;
    validate_payload(&document, &layout)?;
    let etag = mutation_digest(&document, &layout)?;
    Ok(StudioDraftRecord {
        schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
        draft_id: draft_id.to_owned(),
        definition_id: definition_id.to_owned(),
        revision,
        etag,
        document,
        layout,
        updated_at: format!("revision-{revision}"),
        conflict: None,
    })
}

pub(super) fn published_definition(
    draft: &StudioDraftRecord,
    digest: &str,
) -> Result<StudioDefinitionRecord, StoreError> {
    let object = draft
        .document
        .as_object()
        .ok_or_else(|| StoreError::new("definition_shape", "definition must be a JSON object"))?;
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::new("definition_shape", "definition version is missing"))?;
    let workflow_id = object
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::new("definition_shape", "workflow_id is missing"))?;
    Ok(StudioDefinitionRecord {
        schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
        id: digest.to_owned(),
        title: workflow_id.to_owned(),
        description: format!("Published revision {version}"),
        source: "published".to_owned(),
        version: version.to_owned(),
        definition_digest: digest.to_owned(),
        definition: draft.document.clone(),
        published_revision: draft.revision,
    })
}

pub(super) fn conflict_draft(mut current: StudioDraftRecord) -> StudioDraftRecord {
    current.conflict = Some(StudioDraftConflict {
        server_revision: current.revision,
        server_etag: current.etag.clone(),
        server_document: current.document.clone(),
        server_layout: current.layout.clone(),
    });
    current
}

pub(super) fn validate_draft(draft: &StudioDraftRecord) -> Result<(), StoreError> {
    if draft.schema_version != STUDIO_SCHEMA_VERSION || draft.revision != 0 {
        return Err(StoreError::new(
            "draft_shape",
            "new Studio drafts must use the authoring schema and revision zero",
        ));
    }
    validate_id("draft_id", &draft.draft_id)?;
    validate_id("definition_id", &draft.definition_id)?;
    validate_payload(&draft.document, &draft.layout)
}

pub(super) fn validate_payload(document: &Value, layout: &Value) -> Result<(), StoreError> {
    validate_json(document, "document")?;
    validate_json(layout, "layout")?;
    if !layout.is_object() {
        return Err(StoreError::new(
            "layout_shape",
            "Studio layout must be a JSON object",
        ));
    }
    reject_secret_like(document, "document")?;
    reject_secret_like(layout, "layout")
}

fn validate_json(value: &Value, name: &str) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| StoreError::new("json_encode", format!("{name}: {error}")))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(StoreError::new(
            "json_too_large",
            format!("{name} exceeds the management JSON bound"),
        ));
    }
    super::super::contract::decode_value(&bytes)
        .map(|_| ())
        .map_err(|error| StoreError::new("json_invalid", format!("{name}: {error}")))
}

fn reject_secret_like(value: &Value, path: &str) -> Result<(), StoreError> {
    match value {
        Value::Array(values) => values
            .iter()
            .enumerate()
            .try_for_each(|(index, child)| reject_secret_like(child, &format!("{path}[{index}]"))),
        Value::Object(object) => {
            for (key, child) in object {
                let lower = key.to_ascii_lowercase();
                if [
                    "token",
                    "secret",
                    "password",
                    "apikey",
                    "api_key",
                    "private_key",
                ]
                .iter()
                .any(|needle| lower == *needle || lower.contains(needle))
                {
                    return Err(StoreError::new(
                        "secret_like_field",
                        format!("authoring payload contains a secret-like field at {path}.{key}"),
                    ));
                }
                reject_secret_like(child, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

pub(super) fn mutation_digest(document: &Value, layout: &Value) -> Result<String, StoreError> {
    digest_value(&json!({"document": document, "layout": layout}))
        .map_err(|error| StoreError::new("mutation_digest", error.to_string()))
}

pub(super) fn definition_digest(document: &Value) -> Result<String, StoreError> {
    digest_value(document).map_err(|error| StoreError::new("definition_digest", error.to_string()))
}

pub(super) fn validate_id(name: &str, value: &str) -> Result<(), StoreError> {
    validate_identifier(name, value).map_err(|error| StoreError::new(error.code, error.message))
}

pub(super) fn validate_digest_text(value: &str) -> Result<(), StoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StoreError::new(
            "invalid_digest",
            "definition digest is not a lowercase SHA-256 value",
        ));
    }
    Ok(())
}
