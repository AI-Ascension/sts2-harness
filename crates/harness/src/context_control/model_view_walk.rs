// SPDX-License-Identifier: MIT

//! Reading a resolved projection plan out of an admitted source.
//!
//! Every step here follows the plan [`model_view_path`](super::model_view_path) produced, so no
//! field name is resolved at read time and the walk cannot be steered by source content. Two
//! behaviours are deliberate:
//!
//! - an absent optional field is omitted rather than invented as a null, which keeps "absent" and
//!   "explicitly null" distinguishable, and
//! - naming a collection of objects without an index rebuilds every element from the catalog, so a
//!   whole-array selection is a filtered projection rather than a passthrough.

use super::model_view::{ALL_ITEMS_MARKER, ProjectedField, ReadStep};
use super::model_view_catalog::{ElementKind, FieldShape};
use super::model_view_elements::project_model_visible_members;
use super::model_view_error::ModelViewProjectionError;
use serde_json::{Map, Value};

pub(super) fn apply_field(
    field: &ProjectedField,
    source: &Value,
    output: &mut Map<String, Value>,
) -> Result<(), ModelViewProjectionError> {
    if field.output_path.is_empty() {
        return Err(ModelViewProjectionError::EmptyPath);
    }
    // `read_along` returns a subtree rooted at the first declared segment, so sibling paths that
    // share a parent must merge instead of replacing one another.
    if let Some(value) = read_along(source, &field.steps, &[], field)? {
        merge_into(output, value);
    }
    Ok(())
}

fn merge_into(target: &mut Map<String, Value>, value: Value) {
    let Value::Object(incoming) = value else {
        return;
    };
    for (key, child) in incoming {
        match (target.get_mut(&key), child) {
            (Some(Value::Object(existing)), Value::Object(nested)) => {
                merge_into(existing, Value::Object(nested))
            }
            // Two declared paths may index the same collection, for example both `card_id` and
            // `name` of one hand. Each contributes the same-length array holding only its own
            // member, so the elements are merged position by position. Replacing instead would
            // silently drop every member but the last one declared, and the surviving set would
            // depend on declaration order.
            (Some(Value::Array(existing)), Value::Array(incoming)) => {
                merge_arrays(existing, incoming);
            }
            (_, child) => {
                target.insert(key, child);
            }
        }
    }
}

/// Merges one projected collection level into another, element by element.
///
/// A length mismatch cannot arise from the walk, because every path indexes the same source
/// collection under the same declared bound. If it ever did, the longer array is retained rather
/// than truncated, so a mismatch cannot silently drop selected elements.
fn merge_arrays(target: &mut Vec<Value>, incoming: Vec<Value>) {
    if target.len() != incoming.len() {
        if incoming.len() > target.len() {
            *target = incoming;
        }
        return;
    }
    for (existing, child) in target.iter_mut().zip(incoming) {
        match (existing, child) {
            (Value::Object(existing), Value::Object(nested)) => {
                merge_into(existing, Value::Object(nested));
            }
            (existing, child) => *existing = child,
        }
    }
}

/// Walks one resolved read path through the source, producing the projected subtree.
///
/// `emitted` is the output path already written, so diagnostics name the exact source path.
/// `Ok(None)` means the path selected nothing: an optional field that is absent from the source is
/// omitted rather than invented as a null, which keeps absence and an explicit null distinguishable.
fn read_along(
    source: &Value,
    remaining: &[ReadStep],
    emitted: &[String],
    field: &ProjectedField,
) -> Result<Option<Value>, ModelViewProjectionError> {
    let Some((step, rest)) = remaining.split_first() else {
        return select_leaf(source, field, &display_emitted(emitted));
    };
    if step.name == ALL_ITEMS_MARKER {
        let path = display_emitted(emitted);
        let items =
            source
                .as_array()
                .ok_or_else(|| ModelViewProjectionError::SourceShapeMismatch {
                    path: path.to_owned(),
                })?;
        let bound = field.indexed_bound.unwrap_or(items.len());
        if items.len() > bound {
            return Err(ModelViewProjectionError::CollectionBoundExceeded {
                path: path.to_owned(),
                bound,
            });
        }
        let mut projected = Vec::with_capacity(items.len());
        for item in items {
            // An element that lacks an optional member keeps its position with an empty selection
            // rather than being dropped, so the projected collection never changes cardinality.
            let value = read_along(item, rest, emitted, field)?
                .unwrap_or_else(|| Value::Object(Map::new()));
            projected.push(value);
        }
        return Ok(Some(Value::Array(projected)));
    }
    let context = step.context.ok_or(ModelViewProjectionError::EmptyPath)?;
    let entry = super::model_view_catalog::spec(context, &step.name).ok_or_else(|| {
        ModelViewProjectionError::UnknownPath {
            path: appended(emitted, &step.name),
        }
    })?;
    let path = appended(emitted, &step.name);
    let object =
        source
            .as_object()
            .ok_or_else(|| ModelViewProjectionError::SourceShapeMismatch {
                path: path.to_owned(),
            })?;
    let Some(child) = object.get(&step.name) else {
        // An absent optional field is omitted from the output. An absent required field is a
        // refusal, because the recipe declared the source guarantees it.
        return match entry.presence {
            super::model_view_catalog::FieldPresence::Optional => Ok(None),
            super::model_view_catalog::FieldPresence::Required => {
                Err(ModelViewProjectionError::MissingSourceField { path })
            }
        };
    };
    let mut child_emitted = emitted.to_vec();
    child_emitted.push(step.name.clone());
    // The child's projection is either the selected leaf itself or a subtree one level deeper;
    // either way this frame contributes exactly its own name, so sibling paths merge cleanly.
    // An explicit null is wrapped at this frame too, so it lands on its own path instead of
    // replacing the parent object that a sibling path already populated.
    let inner = if child.is_null() {
        if entry.nullable {
            Some(Value::Null)
        } else {
            return Err(ModelViewProjectionError::SourceShapeMismatch { path });
        }
    } else if rest.is_empty() {
        select_leaf(child, field, &path)?
    } else {
        read_along(child, rest, &child_emitted, field)?
    };
    let Some(inner) = inner else {
        // Nothing was selected beneath this container, so the container itself is omitted.
        return Ok(None);
    };
    let mut nested = Map::new();
    nested.insert(step.name.clone(), inner);
    Ok(Some(Value::Object(nested)))
}

/// Selects the value at the end of a resolved path.
///
/// A path that ends on an object collection takes every element, rebuilt from the catalog so only
/// model-visible members survive. Any other landing is carried through after a shape check.
fn select_leaf(
    source: &Value,
    field: &ProjectedField,
    path: &str,
) -> Result<Option<Value>, ModelViewProjectionError> {
    match field.shape {
        FieldShape::Collection(ElementKind::Object(element_context), bound) => {
            let items =
                source
                    .as_array()
                    .ok_or_else(|| ModelViewProjectionError::SourceShapeMismatch {
                        path: path.to_owned(),
                    })?;
            if items.len() > bound {
                return Err(ModelViewProjectionError::CollectionBoundExceeded {
                    path: path.to_owned(),
                    bound,
                });
            }
            let mut projected = Vec::with_capacity(items.len());
            for item in items {
                projected.push(project_model_visible_members(item, element_context, 0)?);
            }
            Ok(Some(Value::Array(projected)))
        }
        _ => check_shape(source, field, path).map(Some),
    }
}

fn display_emitted(emitted: &[String]) -> String {
    let mut rendered = String::new();
    for segment in emitted {
        if segment == ALL_ITEMS_MARKER {
            rendered.push_str("[*]");
            continue;
        }
        if !rendered.is_empty() {
            rendered.push('.');
        }
        rendered.push_str(segment);
    }
    rendered
}

fn appended(prefix: &[String], name: &str) -> String {
    let base = display_emitted(prefix);
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}.{name}")
    }
}

fn check_shape(
    value: &Value,
    field: &ProjectedField,
    path: &str,
) -> Result<Value, ModelViewProjectionError> {
    if value.is_null() {
        return if field.nullable {
            Ok(Value::Null)
        } else {
            Err(ModelViewProjectionError::SourceShapeMismatch {
                path: path.to_owned(),
            })
        };
    }
    let matches = match field.shape {
        FieldShape::Scalar(_) => !value.is_object() && !value.is_array(),
        FieldShape::Object(_) => value.is_object(),
        FieldShape::Collection(ElementKind::Identity, _) => value.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| !item.is_object() && !item.is_array())
        }),
        FieldShape::Collection(ElementKind::Object(_), _) => value.is_array(),
    };
    if !matches {
        return Err(ModelViewProjectionError::SourceShapeMismatch {
            path: path.to_owned(),
        });
    }
    if let (FieldShape::Collection(_, bound), Some(items)) = (field.shape, value.as_array())
        && items.len() > bound
    {
        return Err(ModelViewProjectionError::CollectionBoundExceeded {
            path: path.to_owned(),
            bound,
        });
    }
    Ok(value.clone())
}
