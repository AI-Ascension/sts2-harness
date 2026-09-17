// SPDX-License-Identifier: MIT

//! Reducing one collection element to its model-visible members.
//!
//! Selecting a whole collection of objects must not be a passthrough. An element is rebuilt from
//! the catalog, so a member the catalog does not declare — including an owner-only one — is dropped
//! rather than forwarded. Nested object members are reduced the same way, bounded by
//! [`MAX_ELEMENT_DEPTH`] so a cyclical source cannot drive unbounded recursion.

use super::model_view_catalog::{
    ElementKind, FIELD_CATALOG, FieldProtection, FieldShape, ViewContext,
};
use super::model_view_error::ModelViewProjectionError;
use serde_json::{Map, Value};

/// Deepest element nesting a whole-collection selection projects through.
const MAX_ELEMENT_DEPTH: usize = 3;

/// Reduces one source element to the model-visible members the catalog declares for its context.
///
/// Whole-collection selection must not be a passthrough: an element is rebuilt from the catalog, so
/// any field the catalog does not declare — including an owner-only one — cannot survive. Nested
/// object members are reduced the same way, bounded by [`MAX_ELEMENT_DEPTH`].
pub(super) fn project_model_visible_members(
    source: &Value,
    context: ViewContext,
    depth: usize,
) -> Result<Value, ModelViewProjectionError> {
    if depth > MAX_ELEMENT_DEPTH {
        return Err(ModelViewProjectionError::UnresolvedObject {
            path: context.as_str().to_owned(),
        });
    }
    let object =
        source
            .as_object()
            .ok_or_else(|| ModelViewProjectionError::SourceShapeMismatch {
                path: context.as_str().to_owned(),
            })?;
    let mut reduced = Map::new();
    for entry in FIELD_CATALOG
        .iter()
        .filter(|entry| entry.context == context)
    {
        if entry.protection == FieldProtection::OwnerOnly {
            continue;
        }
        let Some(value) = object.get(entry.name) else {
            continue;
        };
        if value.is_null() {
            if entry.nullable {
                reduced.insert(entry.name.to_owned(), Value::Null);
            }
            continue;
        }
        let projected = match entry.shape {
            FieldShape::Object(nested_context) => {
                project_model_visible_members(value, nested_context, depth + 1)?
            }
            FieldShape::Collection(ElementKind::Object(element_context), bound) => {
                let items = value.as_array().ok_or_else(|| {
                    ModelViewProjectionError::SourceShapeMismatch {
                        path: entry.name.to_owned(),
                    }
                })?;
                if items.len() > bound {
                    return Err(ModelViewProjectionError::CollectionBoundExceeded {
                        path: entry.name.to_owned(),
                        bound,
                    });
                }
                let mut projected = Vec::with_capacity(items.len());
                for item in items {
                    projected.push(project_model_visible_members(
                        item,
                        element_context,
                        depth + 1,
                    )?);
                }
                Value::Array(projected)
            }
            FieldShape::Collection(ElementKind::Identity, bound) => {
                let items = value.as_array().ok_or_else(|| {
                    ModelViewProjectionError::SourceShapeMismatch {
                        path: entry.name.to_owned(),
                    }
                })?;
                if items.len() > bound {
                    return Err(ModelViewProjectionError::CollectionBoundExceeded {
                        path: entry.name.to_owned(),
                        bound,
                    });
                }
                value.clone()
            }
            FieldShape::Scalar(_) => value.clone(),
        };
        reduced.insert(entry.name.to_owned(), projected);
    }
    Ok(Value::Object(reduced))
}
