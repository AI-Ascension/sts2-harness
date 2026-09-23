// SPDX-License-Identifier: MIT

//! The immutable identity a fork inherits from its source prefix.
//!
//! A binding names everything that must stay fixed for a fork to be a genuine continuation of the
//! same seeded run: the seed, the launch profile, the build, the compatibility revision and the
//! exact recorded prefix. Comparison is exact per field; nothing is widened to force equality.

use std::fmt;

use serde::Serialize;

use super::label_ok;

/// One immutable fork binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ForkBinding {
    /// Explicit seed bound to the recorded prefix.
    pub seed: String,
    /// Launch profile identifier the prefix was recorded under.
    pub profile_id: String,
    /// Digest of the exact build the prefix was recorded under.
    pub build_digest: String,
    /// Compatibility revision the prefix was recorded under.
    pub compatibility_digest: String,
    /// Digest of the recorded seeded replay prefix itself.
    pub prefix_digest: String,
}

/// A single reason two bindings are not interchangeable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkBindingMismatch {
    /// The seeds differ.
    Seed,
    /// The launch profile identifiers differ.
    Profile,
    /// The build digests differ.
    Build,
    /// The compatibility revisions differ.
    Compatibility,
    /// The recorded prefix digests differ.
    Prefix,
}

/// Rejection reasons for a binding declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkBindingError {
    /// A field is empty, oversized or contains a NUL separator.
    InvalidLabel,
}

impl fmt::Display for ForkBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLabel => "fork binding label is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ForkBindingError {}

impl ForkBinding {
    /// Validates every bound field against its byte bound.
    ///
    /// # Errors
    ///
    /// Returns [`ForkBindingError::InvalidLabel`] for an empty, oversized or NUL-bearing field.
    pub fn validate(&self) -> Result<(), ForkBindingError> {
        for value in [
            &self.seed,
            &self.profile_id,
            &self.build_digest,
            &self.compatibility_digest,
            &self.prefix_digest,
        ] {
            if !label_ok(value) {
                return Err(ForkBindingError::InvalidLabel);
            }
        }
        Ok(())
    }

    /// Lists every reason `other` is not the same binding, in a stable order.
    #[must_use]
    pub fn compare(&self, other: &Self) -> Vec<ForkBindingMismatch> {
        let checks = [
            (self.seed != other.seed, ForkBindingMismatch::Seed),
            (
                self.profile_id != other.profile_id,
                ForkBindingMismatch::Profile,
            ),
            (
                self.build_digest != other.build_digest,
                ForkBindingMismatch::Build,
            ),
            (
                self.compatibility_digest != other.compatibility_digest,
                ForkBindingMismatch::Compatibility,
            ),
            (
                self.prefix_digest != other.prefix_digest,
                ForkBindingMismatch::Prefix,
            ),
        ];
        checks
            .into_iter()
            .filter_map(|(different, reason)| different.then_some(reason))
            .collect()
    }

    /// Reports whether two bindings are interchangeable.
    #[must_use]
    pub fn is_compatible(&self, other: &Self) -> bool {
        self.compare(other).is_empty()
    }
}
