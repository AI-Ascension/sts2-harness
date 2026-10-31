// SPDX-License-Identifier: MIT
//! Whole-profile schema and owner consistency checks; hashes remain producer evidence.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use sts2_protocol::GameInformationContentManifestV1Codec;

pub(super) fn operation(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

pub(super) fn response(
    bytes: &[u8],
    correlation: &str,
    binding: &LookupBinding,
) -> Result<Value, LookupError> {
    let value = decode(bytes, correlation)?;
    if value["kind"] == "content_manifest_response" {
        let catalog = &value["manifest"];
        if catalog["inventory_revision"] != binding.content_manifest_id
            || catalog["locale"] != binding.locale
        {
            return Err(LookupError::Scope);
        }
        catalog_consistency(catalog)?;
    }
    Ok(value)
}

pub(in crate::game_information) fn decode(
    bytes: &[u8],
    correlation: &str,
) -> Result<Value, LookupError> {
    if bytes.is_empty() || bytes.len() > MAX_RESPONSE_BYTES {
        return Err(LookupError::Bounds);
    }
    if !correlation
        .parse::<u64>()
        .ok()
        .is_some_and(|id| id > 0 && id.to_string() == correlation)
    {
        return Err(LookupError::Invalid);
    }
    let codec = GameInformationContentManifestV1Codec::new().map_err(|_| LookupError::Invalid)?;
    let value = codec.decode(bytes).map_err(|_| LookupError::Invalid)?;
    if value["correlation_id"] != correlation {
        return Err(LookupError::Scope);
    }
    Ok(value)
}

fn catalog_consistency(catalog: &Value) -> Result<(), LookupError> {
    let mut packages = BTreeMap::new();
    let mut orders = BTreeSet::new();
    for package in array(&catalog["packages"])? {
        let id = token(&package["package_id"])?;
        if !orders.insert(package["order"].as_u64().ok_or(LookupError::Invalid)?)
            || packages.insert(id, &package["package_version"]).is_some()
        {
            return Err(LookupError::Invalid);
        }
    }
    let mut counts = BTreeMap::new();
    for family in array(&catalog["families"])? {
        if counts
            .insert(token(&family["entity_kind"])?, (family, 0_u64))
            .is_some()
        {
            return Err(LookupError::Invalid);
        }
    }
    let mut identities = BTreeSet::new();
    for definition in array(&catalog["definitions"])? {
        let kind = token(&definition["entity_kind"])?;
        if !identities.insert((kind, token(&definition["namespaced_id"])?)) {
            return Err(LookupError::Invalid);
        }
        let (family, count) = counts.get_mut(kind).ok_or(LookupError::Invalid)?;
        *count += 1;
        if family["handled"] == false && definition["handled"] == true {
            return Err(LookupError::Invalid);
        }
        origin(definition, &packages)?;
    }
    if counts
        .values()
        .any(|(family, count)| family["definition_count"].as_u64() != Some(*count))
    {
        return Err(LookupError::Invalid);
    }
    Ok(())
}

fn origin(definition: &Value, packages: &BTreeMap<&str, &Value>) -> Result<(), LookupError> {
    let source = &definition["origin"];
    if let Some(id) = source["package_id"].as_str() {
        let package_version = packages.get(id).ok_or(LookupError::Invalid)?;
        if !package_version.is_null()
            && !source["package_version"].is_null()
            && **package_version != source["package_version"]
        {
            return Err(LookupError::Invalid);
        }
    } else if !source["package_version"].is_null() {
        return Err(LookupError::Invalid);
    }
    let mut overrides = BTreeSet::new();
    for reference in array(&definition["override_chain"])? {
        if !overrides.insert(token(reference)?) {
            return Err(LookupError::Invalid);
        }
    }
    Ok(())
}

fn array(value: &Value) -> Result<&[Value], LookupError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or(LookupError::Invalid)
}
fn token(value: &Value) -> Result<&str, LookupError> {
    value.as_str().ok_or(LookupError::Invalid)
}

pub(in crate::game_information) fn record(
    record: &ContentManifestRecord,
    binding: &LookupBinding,
) -> Result<(), LookupError> {
    if record.schema != RECORD_SCHEMA
        || !record.binding.same_owner(binding)
        || record.binding.snapshot.is_some()
        || !operation(&record.operation_id)
    {
        return Err(LookupError::Divergence);
    }
    if record.error.is_some() {
        if record.correlation_id.is_some()
            || record.source_sha256.is_some()
            || !record.sources.is_empty()
            || record.source_bytes != 0
            || record.view_sha256.is_some()
        {
            return Err(LookupError::Divergence);
        }
    } else if record.correlation_id.is_none()
        || record.source_sha256.is_none()
        || record.view_sha256.is_none()
        || record.source_bytes == 0
        || record.source_bytes > MAX_RESPONSE_BYTES
        || record.sources.len()
            != record
                .source_bytes
                .div_ceil(crate::context_memory::MAX_SOURCE_BYTES)
    {
        return Err(LookupError::MissingRetention);
    }
    Ok(())
}
