// SPDX-License-Identifier: MIT

//! Applying a closed model-view recipe to an already admitted observation (issue #110).
//!
//! ## Order is the whole point
//!
//! [`project_model_view`] revalidates the **complete** source observation through the fair-play
//! validator *before* it reads a single declared field, and it revalidates that same complete value
//! again *after* projection. Validation is therefore never narrowed to the selected subset: a
//! forbidden field the recipe happens to exclude still fails the whole call, because the source
//! must be admissible in full before any part of it may be shown.
//!
//! ## The source is never mutated
//!
//! The function takes the owner's value by reference and returns a new value. The owner's
//! observation, its legal-action catalog, and its provenance are untouched, so a projection cannot
//! narrow what the host retains or what legality checks later read.
//!
//! ## Excluded sentinels are checked, not assumed
//!
//! Rather than trusting the walk to have skipped the right fields, the output is swept for every
//! owner-only and unknown sentinel path. A present sentinel is a hard refusal, so a future resolver
//! regression fails closed instead of leaking.

use super::model_view::ModelViewProjection;
use super::model_view_error::ModelViewProjectionError;
use super::model_view_sentinels::reject_excluded_sentinels;
use super::model_view_walk::apply_field;
use crate::sha256_hex;
use serde_json::{Map, Value};

/// Upper bound on one projected model-view payload.
pub const MAX_MODEL_VIEW_BYTES: usize = 64 * 1024;

/// A source value that already passed fair-play validation at admission.
///
/// The wrapper exists so projection cannot be reached without the caller having validated: the only
/// public constructor runs [`SanitizedObservation::new`](crate::exo::SanitizedObservation::new) and
/// hands the admitted value to the projection, which revalidates it again.
#[derive(Clone, Debug)]
pub struct AdmittedSourceObservation(Value);

impl AdmittedSourceObservation {
    /// Validates a complete observation through the fair-play validator.
    pub fn admit(value: Value) -> Result<Self, ModelViewProjectionError> {
        crate::exo::SanitizedObservation::new(value.clone())
            .map_err(|_| ModelViewProjectionError::SourceInvalid)?;
        Ok(Self(value))
    }

    /// The complete admitted value, unchanged.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

/// The projected model-view bytes plus the witness a consumer revalidates against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedModelView {
    /// The projected value, containing only declared model-visible fields.
    pub value: Value,
    /// The canonical bytes of `value`.
    pub bytes: Vec<u8>,
    /// Digest of the complete admitted source, independent of the recipe.
    pub source_digest: String,
    /// Digest of the recipe that produced this projection.
    pub recipe_digest: String,
    /// Digest of the projected bytes.
    pub projected_digest: String,
    /// The rendered declared paths this projection contains.
    pub paths: Vec<String>,
}

impl PreparedModelView {
    /// Re-derives the projected digest from the current bytes.
    pub fn recompute_digest(&self) -> Result<String, ModelViewProjectionError> {
        let encoded =
            serde_json::to_vec(&self.value).map_err(|_| ModelViewProjectionError::Encode)?;
        Ok(sha256_hex(&encoded))
    }
}

/// An approval minted against one exact source and recipe revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelViewApproval {
    pub selector_id: String,
    pub source_digest: String,
    pub recipe_digest: String,
    pub projected_digest: String,
}

impl ModelViewApproval {
    /// Mints an approval bound to one prepared projection.
    #[must_use]
    pub fn mint(recipe: &ModelViewProjection, prepared: &PreparedModelView) -> Self {
        Self {
            selector_id: recipe.selector_id.clone(),
            source_digest: prepared.source_digest.clone(),
            recipe_digest: prepared.recipe_digest.clone(),
            projected_digest: prepared.projected_digest.clone(),
        }
    }

    /// Confirms this approval still describes the prepared projection.
    ///
    /// A changed source, a changed recipe, or changed bytes all fence the approval, so a revision
    /// edited after review cannot inherit the earlier verdict.
    pub fn verify(
        &self,
        recipe: &ModelViewProjection,
        prepared: &PreparedModelView,
    ) -> Result<(), ModelViewProjectionError> {
        // The recipe digest is re-derived from the *current* recipe, not read back from the
        // prepared witness, so a revision edited after approval cannot keep the earlier verdict.
        if self.selector_id != recipe.selector_id
            || self.source_digest != prepared.source_digest
            || self.recipe_digest != prepared.recipe_digest
            || self.recipe_digest != recipe.digest()?
            || self.projected_digest != prepared.projected_digest
            || prepared.recompute_digest()? != prepared.projected_digest
        {
            return Err(ModelViewProjectionError::ApprovalFenced);
        }
        Ok(())
    }
}

/// Validates the complete source, applies a closed recipe, and revalidates the source.
///
/// Fails before producing any bytes when the source is inadmissible, when a declared field is
/// missing or wrongly shaped, when a collection exceeds its declared bound, when the result is
/// oversized, or when any excluded sentinel survived into the output.
pub fn project_model_view(
    recipe: &ModelViewProjection,
    source: &AdmittedSourceObservation,
) -> Result<PreparedModelView, ModelViewProjectionError> {
    // 1. Complete-source fair-play validation, before any field is read.
    crate::exo::SanitizedObservation::new(source.0.clone())
        .map_err(|_| ModelViewProjectionError::SourceInvalid)?;

    let resolved = recipe.resolved_fields()?;
    let mut output = Map::new();
    for field in &resolved {
        apply_field(field, source.as_value(), &mut output)?;
    }
    let value = Value::Object(output);

    // 2. No owner-only or unknown sentinel may survive the walk.
    reject_excluded_sentinels(&value)?;

    let bytes = serde_json::to_vec(&value).map_err(|_| ModelViewProjectionError::Encode)?;
    if bytes.len() > MAX_MODEL_VIEW_BYTES {
        return Err(ModelViewProjectionError::OversizedOutput {
            bound: MAX_MODEL_VIEW_BYTES,
        });
    }

    // 3. The complete source is still admissible after projection.
    crate::exo::SanitizedObservation::new(source.0.clone())
        .map_err(|_| ModelViewProjectionError::SourceInvalid)?;

    let source_bytes =
        serde_json::to_vec(source.as_value()).map_err(|_| ModelViewProjectionError::Encode)?;
    let projected_digest = sha256_hex(&bytes);
    Ok(PreparedModelView {
        value,
        bytes,
        source_digest: sha256_hex(&source_bytes),
        recipe_digest: recipe.digest()?,
        projected_digest,
        paths: recipe.declared_paths(),
    })
}
