// SPDX-License-Identifier: MIT

//! Additive typed recipe contract v2.
//!
//! This module describes one fixed map snapshot read. It does not execute that
//! read, select a runtime route, or enable recipe collection in production.

use serde::{Deserialize, Serialize};
use sts2_protocol::RuntimeMapV1Snapshot;

use super::ids::{RecipeId, RecipeRevision};

pub use super::admission_v2::{RecipeAdmissionErrorV2, admit_recipe_v2};

/// The only schema version accepted by this contract module.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RecipeContractVersionV2 {
    /// The `ascension.recipe/v2` contract.
    #[serde(rename = "ascension.recipe/v2")]
    V2,
}

/// A recipe operation available in contract v2.
///
/// The tagged wire representation has no caller-controlled arguments or route.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeOperationV2 {
    /// Read the host's fixed, read-only map snapshot projection.
    MapSnapshot {},
}

/// Authored V2 definition before ID and revision admission.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeDefinitionV2 {
    /// Fixed V2 schema tag.
    pub schema_version: RecipeContractVersionV2,
    /// Recipe ID validated by [`admit_recipe_v2`].
    pub id: String,
    /// Recipe revision; zero is refused by admission.
    pub revision: u32,
    /// The one fixed read operation.
    pub operation: RecipeOperationV2,
}

impl RecipeDefinitionV2 {
    /// Creates a V2 definition with the fixed schema tag.
    #[must_use]
    pub fn new(id: impl Into<String>, revision: u32, operation: RecipeOperationV2) -> Self {
        Self {
            schema_version: RecipeContractVersionV2::V2,
            id: id.into(),
            revision,
            operation,
        }
    }
}

/// Effect-free admitted recipe identity and fixed operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedRecipeV2 {
    id: RecipeId,
    revision: RecipeRevision,
    operation: RecipeOperationV2,
}

impl AdmittedRecipeV2 {
    /// Builds the admitted value after its identity and revision are checked.
    pub(super) fn new(
        id: RecipeId,
        revision: RecipeRevision,
        operation: RecipeOperationV2,
    ) -> Self {
        Self {
            id,
            revision,
            operation,
        }
    }

    /// Returns the validated recipe ID.
    #[must_use]
    pub const fn id(&self) -> &RecipeId {
        &self.id
    }

    /// Returns the nonzero recipe revision.
    #[must_use]
    pub const fn revision(&self) -> RecipeRevision {
        self.revision
    }

    /// Returns the fixed operation selected by this admitted definition.
    #[must_use]
    pub const fn operation(&self) -> RecipeOperationV2 {
        self.operation
    }
}

/// Closed result domain for the one V2 map operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeOutputV2 {
    /// A typed map snapshot from the pinned `runtime-map-v1` protocol.
    MapSnapshot(RuntimeMapV1Snapshot),
}

#[cfg(test)]
#[path = "contract_v2_tests.rs"]
mod tests;
