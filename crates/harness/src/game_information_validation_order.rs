// SPDX-License-Identifier: MIT
use super::{ValidationError, array};
use serde_json::Value;

pub(super) fn validate(items: &[Value], ordering: &Value) -> Result<(), ValidationError> {
    let key = ordering["key"].as_str().ok_or(ValidationError::Ordering)?;
    if (key == "display_name") != (ordering["algorithm"] == "unicode_scalar_values") {
        return Err(ValidationError::Ordering);
    }
    let mut previous: Option<Vec<String>> = None;
    for item in items {
        let identity = &item["definition_ref"];
        let identity_parts = |value: &Value, keys: &[&str]| {
            keys.iter()
                .map(|key| {
                    value[key]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value[key].to_string())
                })
                .collect::<Vec<_>>()
        };
        let parts = match key {
            "definition_ref" => identity_parts(
                identity,
                &[
                    "content_manifest_id",
                    "entity_kind",
                    "namespaced_id",
                    "variant",
                ],
            ),
            "instance_ref" if !item["instance_ref"].is_null() => identity_parts(
                &item["instance_ref"],
                &["instance_id", "run_id", "epoch", "entity_kind", "entity_id"],
            ),
            "namespaced_id" => identity_parts(identity, &["namespaced_id", "variant"]),
            "display_name" => {
                let field = array(&item["fields"])?
                    .iter()
                    .find(|field| {
                        field["name"] == "display_name"
                            && field["availability"] == "available"
                            && field["kind"] == "text"
                    })
                    .ok_or(ValidationError::Ordering)?;
                vec![
                    field["value"]
                        .as_str()
                        .ok_or(ValidationError::Ordering)?
                        .to_owned(),
                ]
            }
            _ => return Err(ValidationError::Ordering),
        };
        if let Some(previous) = &previous {
            let invalid = if ordering["direction"] == "ascending" {
                previous > &parts
            } else {
                previous < &parts
            };
            if invalid {
                return Err(ValidationError::Ordering);
            }
        }
        previous = Some(parts);
    }
    Ok(())
}
