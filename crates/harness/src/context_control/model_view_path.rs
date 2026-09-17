// SPDX-License-Identifier: MIT

//! Static resolution of a declared field path against the closed catalog.
//!
//! Resolution never inspects a source value. It walks the declared segments through
//! [`FIELD_CATALOG`] and produces a [`ProjectedField`] that records, for every step, which context
//! the name belongs to. Projection then reads the source using that recorded plan, so no name is
//! ever looked up twice and no lookup can be influenced by observation content.

use super::model_view::{ALL_ITEMS_MARKER, PathSegment, ProjectedField, ReadStep, ViewFieldPath};
use super::model_view_catalog::{
    ElementKind, FIELD_CATALOG, FieldPresence, FieldProtection, FieldShape, FieldSpec,
    MAX_MODEL_VIEW_PATH_SEGMENTS, ViewContext, spec,
};
use super::model_view_error::ModelViewProjectionError;
use std::collections::BTreeSet;

/// Resolves every declared path, refusing duplicates and omitted required root fields.
pub(super) fn resolve_fields(
    fields: &[ViewFieldPath],
) -> Result<Vec<ProjectedField>, ModelViewProjectionError> {
    let mut resolved = Vec::with_capacity(fields.len());
    let mut seen = BTreeSet::new();
    for field in fields {
        let projected = resolve_path(&field.segments)?;
        let key = display_path(&projected.output_path);
        if !seen.insert(key.clone()) {
            return Err(ModelViewProjectionError::DuplicateOutput { path: key });
        }
        resolved.push(projected);
    }
    check_required_root_coverage(&resolved)?;
    Ok(resolved)
}

/// The root fields every recipe must reach: required by the source schema, and model-visible.
pub(crate) fn required_model_visible_root_fields() -> impl Iterator<Item = &'static FieldSpec> {
    FIELD_CATALOG.iter().filter(|entry| {
        entry.context == ViewContext::Root
            && entry.presence == FieldPresence::Required
            && entry.protection == FieldProtection::ModelVisible
    })
}

fn check_required_root_coverage(
    resolved: &[ProjectedField],
) -> Result<(), ModelViewProjectionError> {
    let reached: BTreeSet<&str> = resolved
        .iter()
        .filter_map(|field| field.output_path.first().map(String::as_str))
        .collect();
    for entry in required_model_visible_root_fields() {
        if !reached.contains(entry.name) {
            return Err(ModelViewProjectionError::RequiredFieldOmitted {
                field: entry.name.to_owned(),
            });
        }
    }
    Ok(())
}

fn resolve_path(segments: &[PathSegment]) -> Result<ProjectedField, ModelViewProjectionError> {
    if segments.is_empty() {
        return Err(ModelViewProjectionError::EmptyPath);
    }
    if segments.len() > MAX_MODEL_VIEW_PATH_SEGMENTS {
        return Err(ModelViewProjectionError::PathTooLong {
            bound: MAX_MODEL_VIEW_PATH_SEGMENTS,
        });
    }

    let mut context = ViewContext::Root;
    let mut output: Vec<String> = Vec::new();
    let mut steps: Vec<ReadStep> = Vec::new();
    let mut landing: Option<ProjectedField> = None;
    let mut indexed_bound: Option<usize> = None;
    let mut pending: Option<(usize, ViewContext)> = None;
    let mut index = 0usize;

    while index < segments.len() {
        match &segments[index] {
            PathSegment::AllItems => {
                let Some((bound, element_context)) = pending.take() else {
                    return Err(ModelViewProjectionError::MissingIndex {
                        path: display_path(&output),
                    });
                };
                if index + 1 == segments.len() {
                    return Err(ModelViewProjectionError::MissingIndex {
                        path: display_path(&output),
                    });
                }
                output.push(ALL_ITEMS_MARKER.to_owned());
                steps.push(ReadStep {
                    context: None,
                    name: ALL_ITEMS_MARKER.to_owned(),
                });
                indexed_bound = Some(bound);
                context = element_context;
                index += 1;
            }
            PathSegment::Named(name) => {
                // A collection that was named but not indexed may not continue by name.
                if pending.is_some() {
                    return Err(ModelViewProjectionError::MissingIndex {
                        path: display_path(&output),
                    });
                }
                if name.is_empty() {
                    return Err(ModelViewProjectionError::EmptySegment);
                }
                let entry =
                    spec(context, name).ok_or_else(|| ModelViewProjectionError::UnknownPath {
                        path: appended_path(&output, name),
                    })?;
                if entry.protection == FieldProtection::OwnerOnly {
                    return Err(ModelViewProjectionError::ProtectedPath {
                        path: appended_path(&output, name),
                    });
                }
                output.push(name.clone());
                steps.push(ReadStep {
                    context: Some(context),
                    name: name.clone(),
                });
                let terminal = index + 1 == segments.len();
                match entry.shape {
                    FieldShape::Scalar(_) => {
                        if !terminal {
                            return Err(ModelViewProjectionError::NotAnObject {
                                path: display_path(&output),
                            });
                        }
                        landing = Some(landing_for(
                            entry,
                            output.clone(),
                            steps.clone(),
                            None,
                            indexed_bound,
                        ));
                    }
                    FieldShape::Object(next) => {
                        if terminal {
                            return Err(ModelViewProjectionError::UnresolvedObject {
                                path: display_path(&output),
                            });
                        }
                        context = next;
                    }
                    FieldShape::Collection(element, bound) => match element {
                        ElementKind::Identity => {
                            if !terminal {
                                return Err(ModelViewProjectionError::NotAnObject {
                                    path: display_path(&output),
                                });
                            }
                            landing = Some(landing_for(
                                entry,
                                output.clone(),
                                steps.clone(),
                                Some(bound),
                                indexed_bound,
                            ));
                        }
                        ElementKind::Object(element_context) => {
                            if terminal {
                                // Naming a collection of objects without an index selects the whole
                                // collection, restricted to the model-visible members of each
                                // element. This is the only way to take a complete array.
                                landing = Some(landing_for(
                                    entry,
                                    output.clone(),
                                    steps.clone(),
                                    Some(bound),
                                    indexed_bound,
                                ));
                            } else {
                                pending = Some((bound, element_context));
                            }
                        }
                    },
                }
                index += 1;
            }
        }
    }

    landing.ok_or_else(|| {
        if pending.is_some() {
            ModelViewProjectionError::MissingIndex {
                path: display_path(&output),
            }
        } else {
            ModelViewProjectionError::UnresolvedObject {
                path: display_path(&output),
            }
        }
    })
}

fn landing_for(
    entry: &FieldSpec,
    output_path: Vec<String>,
    steps: Vec<ReadStep>,
    bound: Option<usize>,
    indexed_bound: Option<usize>,
) -> ProjectedField {
    ProjectedField {
        output_path,
        steps,
        shape: entry.shape,
        nullable: entry.nullable,
        bound,
        indexed_bound,
    }
}

/// Renders an output path, marking indexed collection levels.
#[must_use]
pub fn display_path(path: &[String]) -> String {
    let mut rendered = String::new();
    for segment in path {
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

fn appended_path(prefix: &[String], name: &str) -> String {
    let base = display_path(prefix);
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}.{name}")
    }
}

/// Renders a declared path as declared, before resolution.
#[must_use]
pub fn render_segments(segments: &[PathSegment]) -> String {
    let mut rendered = String::new();
    for segment in segments {
        match segment {
            PathSegment::AllItems => rendered.push_str("[*]"),
            PathSegment::Named(name) => {
                if !rendered.is_empty() {
                    rendered.push('.');
                }
                rendered.push_str(name);
            }
        }
    }
    rendered
}
