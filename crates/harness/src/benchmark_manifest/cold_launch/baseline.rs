// SPDX-License-Identifier: MIT

//! The immutable pristine baseline a cold-launch trial is provisioned from.
//!
//! A baseline is a declaration: it names the exact build, the launch profile and the
//! deliberately excluded telemetry files. Provisioning a separate writable clone from it is a
//! seam the gateway owns; this module only validates and compares baseline identities.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum bytes of a baseline, build, version or profile label.
pub const MAX_BASELINE_LABEL_BYTES: usize = 256;
/// Maximum deliberately excluded telemetry paths in one baseline.
pub const MAX_BASELINE_EXCLUSIONS: usize = 32;

/// The launch profile a trial must declare before it may start.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaunchProfile {
    /// Stable profile identifier.
    pub profile_id: String,
    /// Digest of the exact build this profile launched.
    pub build_digest: String,
    /// Game version the build reports.
    pub game_version: String,
}

/// Telemetry files intentionally excluded from baseline equality.
///
/// The list is closed and exact: it is not a prefix filter and never broadens to settings,
/// progress or unknown files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TelemetryExclusions {
    /// Excluded relative paths.
    pub paths: Vec<String>,
}

/// One immutable pristine baseline; every trial is provisioned from it, never from a prior trial.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PristineBaseline {
    /// Digest of the pristine baseline artifact itself.
    pub baseline_digest: String,
    /// Launch profile bound to this baseline.
    pub profile: LaunchProfile,
    /// Deliberately excluded telemetry files.
    pub exclusions: TelemetryExclusions,
}

/// A single reason two baselines are not interchangeable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineMismatch {
    /// The baseline artifact digests differ.
    BaselineDigest,
    /// The build digests differ.
    BuildDigest,
    /// The game versions differ.
    GameVersion,
    /// The profile identifiers differ.
    ProfileId,
}

/// Rejection reasons for a baseline declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselineError {
    /// A label is empty, oversized or contains a NUL separator.
    InvalidLabel,
    /// More than [`MAX_BASELINE_EXCLUSIONS`] paths were declared.
    TooManyExclusions,
}

impl fmt::Display for BaselineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLabel => "baseline label is invalid",
            Self::TooManyExclusions => "baseline declares too many exclusions",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for BaselineError {}

impl PristineBaseline {
    /// Validates the labels, the profile and the exclusion bound.
    ///
    /// # Errors
    ///
    /// Returns [`BaselineError::InvalidLabel`] for an empty, oversized or NUL-bearing label and
    /// [`BaselineError::TooManyExclusions`] when the exclusion list exceeds its bound.
    pub fn validate(&self) -> Result<(), BaselineError> {
        for value in [
            &self.baseline_digest,
            &self.profile.profile_id,
            &self.profile.build_digest,
            &self.profile.game_version,
        ] {
            if !label_ok(value) {
                return Err(BaselineError::InvalidLabel);
            }
        }
        if self.exclusions.paths.len() > MAX_BASELINE_EXCLUSIONS {
            return Err(BaselineError::TooManyExclusions);
        }
        for path in &self.exclusions.paths {
            if !label_ok(path) {
                return Err(BaselineError::InvalidLabel);
            }
        }
        Ok(())
    }

    /// Lists every reason `other` is not the same baseline, in a stable order.
    #[must_use]
    pub fn compare(&self, other: &Self) -> Vec<BaselineMismatch> {
        let checks = [
            (
                self.baseline_digest != other.baseline_digest,
                BaselineMismatch::BaselineDigest,
            ),
            (
                self.profile.build_digest != other.profile.build_digest,
                BaselineMismatch::BuildDigest,
            ),
            (
                self.profile.game_version != other.profile.game_version,
                BaselineMismatch::GameVersion,
            ),
            (
                self.profile.profile_id != other.profile.profile_id,
                BaselineMismatch::ProfileId,
            ),
        ];
        checks
            .into_iter()
            .filter_map(|(different, reason)| different.then_some(reason))
            .collect()
    }

    /// Reports whether two baselines are interchangeable.
    #[must_use]
    pub fn is_compatible(&self, other: &Self) -> bool {
        self.compare(other).is_empty()
    }
}

fn label_ok(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_BASELINE_LABEL_BYTES && !value.contains('\0')
}
