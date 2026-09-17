// SPDX-License-Identifier: MIT

//! A closed, versioned model-view projection recipe (issue #110).
//!
//! A recipe is a *declared* selection of already-admitted fields plus a stable revision. It is not
//! an expression: [`ViewFieldPath`] holds typed segments that are resolved hop by hop against
//! [`FIELD_CATALOG`](super::model_view_catalog::FIELD_CATALOG), so nothing is interpreted at
//! projection time. Unknown and owner-only segments are refused while the recipe is built, which is
//! why a malformed or over-reaching view can never reach an inference call.
//!
//! ## Rules the resolver enforces
//!
//! - A scalar must terminate its path; a nested object must be descended into.
//! - An object-element collection must be explicitly indexed with [`PathSegment::AllItems`] and
//!   then name at least one member. There is no recursive wildcard and no implicit flattening.
//! - An identity collection is a terminal value; it has no members to name.
//! - Every required model-visible root field must be reached, so a recipe cannot quietly drop the
//!   observation's identity, player, or state aggregates.
//! - Every [`FieldProtection::OwnerOnly`] field is refused, including the legal-action catalog and
//!   the owner's unseen card piles.

use super::model_view_catalog::{FieldShape, MAX_MODEL_VIEW_FIELDS, ViewContext};
use super::model_view_error::ModelViewProjectionError;
use super::model_view_path::{render_segments, resolve_fields};
use super::types::valid_id;
use crate::sha256_hex;
use serde::{Deserialize, Serialize};

/// Schema identity for the versioned, closed model-view projection recipe.
pub const MODEL_VIEW_PROJECTION_SCHEMA: &str = "ascension.context-control.model-view-projection.v1";

/// Rendered in an output path where one collection level is indexed.
pub const ALL_ITEMS_MARKER: &str = "*";

/// One typed step of a declared field path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathSegment {
    /// A field name that must exist in the current structural context.
    Named(String),
    /// The single permitted collection index, valid only immediately after a collection field.
    AllItems,
}

impl PathSegment {
    /// A named segment.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }
}

/// One declared field path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewFieldPath {
    pub segments: Vec<PathSegment>,
}

impl ViewFieldPath {
    /// Builds a path from typed segments.
    #[must_use]
    pub fn new(segments: Vec<PathSegment>) -> Self {
        Self { segments }
    }
}

/// One resolved read step: the context a name is looked up in, and the name.
///
/// Carrying the context and presence with the resolved path is what lets projection distinguish an
/// absent optional field (omit it) from an absent required field (refuse), without re-resolving the
/// catalog at projection time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadStep {
    /// The structural context the name belongs to; `None` for a collection index marker.
    pub context: Option<ViewContext>,
    /// The field name, or [`ALL_ITEMS_MARKER`] for one collection index.
    pub name: String,
}

/// A resolved path: where it lands in the output and what shape that landing has.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedField {
    /// Output path, with [`ALL_ITEMS_MARKER`] at each indexed collection level.
    pub output_path: Vec<String>,
    /// The resolved read steps, aligned with `output_path`.
    pub steps: Vec<ReadStep>,
    /// The declared shape at the landing.
    pub shape: FieldShape,
    /// Whether the source may hold an explicit `null` here.
    pub nullable: bool,
    /// The declared element bound when the landing is a collection.
    pub bound: Option<usize>,
    /// The declared bound of the object collection this path indexes, when it indexes one.
    pub indexed_bound: Option<usize>,
}

/// The owner-configured half of a model-view projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelViewProjection {
    pub schema: String,
    /// Owner selector identity. A different selector is a different recipe.
    pub selector_id: String,
    /// Owner revision. Changing any binding or transform mints a new revision.
    pub revision: String,
    pub fields: Vec<ViewFieldPath>,
}

#[derive(Serialize)]
struct CanonicalRecipe<'a> {
    schema: &'a str,
    selector_id: &'a str,
    revision: &'a str,
    fields: &'a [ViewFieldPath],
}

impl ModelViewProjection {
    /// Builds a recipe and refuses it unless every path resolves against the closed catalog.
    pub fn new(
        selector_id: impl Into<String>,
        revision: impl Into<String>,
        fields: Vec<ViewFieldPath>,
    ) -> Result<Self, ModelViewProjectionError> {
        let candidate = Self {
            schema: MODEL_VIEW_PROJECTION_SCHEMA.to_owned(),
            selector_id: selector_id.into(),
            revision: revision.into(),
            fields,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Re-validates the envelope and every declared path.
    pub fn validate(&self) -> Result<(), ModelViewProjectionError> {
        if self.schema != MODEL_VIEW_PROJECTION_SCHEMA
            || !valid_id(&self.selector_id)
            || self.revision.is_empty()
        {
            return Err(ModelViewProjectionError::InvalidInput);
        }
        if self.fields.is_empty() || self.fields.len() > MAX_MODEL_VIEW_FIELDS {
            return Err(ModelViewProjectionError::TooManyFields {
                bound: MAX_MODEL_VIEW_FIELDS,
            });
        }
        let _ = self.resolved_fields()?;
        Ok(())
    }

    /// Resolves every declared path into its bounded output shape.
    ///
    /// Resolution is total and static: it never inspects a source value, so it cannot be influenced
    /// by observation content.
    pub fn resolved_fields(&self) -> Result<Vec<ProjectedField>, ModelViewProjectionError> {
        resolve_fields(&self.fields)
    }

    /// Stable digest over the canonical encoding of this recipe.
    ///
    /// The digest is recorded with a prepared projection and re-derived before inference, so a
    /// selector, source, or transform change fences every approval minted against the older
    /// revision.
    pub fn digest(&self) -> Result<String, ModelViewProjectionError> {
        let canonical = CanonicalRecipe {
            schema: &self.schema,
            selector_id: &self.selector_id,
            revision: &self.revision,
            fields: &self.fields,
        };
        let encoded =
            serde_json::to_vec(&canonical).map_err(|_| ModelViewProjectionError::Encode)?;
        Ok(sha256_hex(&encoded))
    }

    /// The rendered declared paths, for metadata and diagnostics.
    #[must_use]
    pub fn declared_paths(&self) -> Vec<String> {
        self.fields
            .iter()
            .map(|field| render_segments(&field.segments))
            .collect()
    }
}

/// A registry that refuses to rebind one selector identity to different fields.
///
/// The live fence re-derives a selector's revision from current configuration; a registry entry
/// that silently changed shape under the same identity would defeat that check, so a second
/// registration of an existing selector with a different digest is refused.
#[derive(Debug, Default)]
pub struct ModelViewSelectorRegistry {
    digests: std::collections::BTreeMap<String, String>,
}

impl ModelViewSelectorRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            digests: std::collections::BTreeMap::new(),
        }
    }

    /// Admits a recipe, or refuses a rebind of an existing selector identity.
    ///
    /// Registering the identical digest again is idempotent and returns `Ok`.
    pub fn admit(
        &mut self,
        selector_id: &str,
        digest: &str,
    ) -> Result<(), ModelViewProjectionError> {
        match self.digests.get(selector_id) {
            Some(existing) if existing != digest => {
                Err(ModelViewProjectionError::RepeatedSelector {
                    selector_id: selector_id.to_owned(),
                })
            }
            _ => {
                self.digests
                    .insert(selector_id.to_owned(), digest.to_owned());
                Ok(())
            }
        }
    }

    /// The digest currently bound to a selector identity, if any.
    #[must_use]
    pub fn digest_of(&self, selector_id: &str) -> Option<&str> {
        self.digests.get(selector_id).map(String::as_str)
    }

    /// Registered selector identities, ordered.
    #[must_use]
    pub fn selector_ids(&self) -> Vec<&str> {
        self.digests.keys().map(String::as_str).collect()
    }
}
