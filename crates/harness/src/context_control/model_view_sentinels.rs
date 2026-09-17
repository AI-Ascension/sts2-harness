// SPDX-License-Identifier: MIT

//! Verifying that no owner-only field survived a projection.
//!
//! The walk is expected to skip owner-only fields, but this sweep does not rely on that. It scans
//! the produced value by name and refuses if any owner-only field appears anywhere in the output,
//! so a future resolver or walk regression fails closed instead of publishing the legal-action
//! catalog or an unseen card pile.

use super::model_view_catalog::{FIELD_CATALOG, FieldProtection};
use super::model_view_error::ModelViewProjectionError;
use crate::exo::SandboxError;
use serde_json::Value;
use std::collections::BTreeSet;

/// Every owner-only field name, together with its declaring context.
#[must_use]
pub fn excluded_sentinel_paths() -> Vec<String> {
    FIELD_CATALOG
        .iter()
        .filter(|entry| entry.protection == FieldProtection::OwnerOnly)
        .map(|entry| format!("{}.{}", entry.context.as_str(), entry.name))
        .collect()
}

/// The rendered path list of the closed catalog, for owner/consumer metadata.
#[must_use]
pub fn catalog_paths() -> Vec<String> {
    FIELD_CATALOG
        .iter()
        .map(|entry| format!("{}.{}", entry.context.as_str(), entry.name))
        .collect()
}

/// Sweeps the projected value for any owner-only field name.
///
/// The sweep is name-based and structural rather than path-based: an owner-only name anywhere in
/// the output is a refusal, so a regression that placed one under an unexpected parent still fails
/// closed.
pub fn reject_excluded_sentinels(value: &Value) -> Result<(), ModelViewProjectionError> {
    let forbidden: BTreeSet<&str> = FIELD_CATALOG
        .iter()
        .filter(|entry| entry.protection == FieldProtection::OwnerOnly)
        .map(|entry| entry.name)
        .collect();
    sweep(value, &forbidden, "")
}

fn sweep(
    value: &Value,
    forbidden: &BTreeSet<&str>,
    path: &str,
) -> Result<(), ModelViewProjectionError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if forbidden.contains(key.as_str()) {
                    return Err(ModelViewProjectionError::ExcludedSentinelPresent {
                        path: child_path,
                    });
                }
                sweep(child, forbidden, &child_path)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                sweep(item, forbidden, path)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

/// Renders the fair-play validator's verdict for one value, for callers that report the exact gate.
pub fn fair_play_verdict(value: &Value) -> Result<(), SandboxError> {
    crate::exo::SanitizedObservation::new(value.clone()).map(|_| ())
}
