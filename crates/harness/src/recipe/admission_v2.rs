// SPDX-License-Identifier: MIT

//! Pure admission for the closed recipe contract v2.

use std::fmt::{Display, Formatter};

use super::contract_v2::{AdmittedRecipeV2, RecipeDefinitionV2};
use super::ids::{RecipeId, RecipeRevision};

/// A recipe contract v2 admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeAdmissionErrorV2 {
    /// The recipe ID is empty, too long, or contains a prohibited character.
    InvalidRecipeId,
    /// Recipe revisions start at one.
    ZeroRevision,
}

impl Display for RecipeAdmissionErrorV2 {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRecipeId => "recipe v2 ID is invalid",
            Self::ZeroRevision => "recipe v2 revision must be nonzero",
        })
    }
}

impl std::error::Error for RecipeAdmissionErrorV2 {}

/// Validates the ID and revision without performing a read or selecting a route.
///
/// # Errors
///
/// Returns an error when the recipe ID is invalid or its revision is zero.
pub fn admit_recipe_v2(
    definition: RecipeDefinitionV2,
) -> Result<AdmittedRecipeV2, RecipeAdmissionErrorV2> {
    let id = RecipeId::new(definition.id).ok_or(RecipeAdmissionErrorV2::InvalidRecipeId)?;
    if definition.revision == 0 {
        return Err(RecipeAdmissionErrorV2::ZeroRevision);
    }

    let revision = RecipeRevision::new(definition.revision);
    Ok(AdmittedRecipeV2::new(id, revision, definition.operation))
}
