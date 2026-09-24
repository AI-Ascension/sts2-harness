// SPDX-License-Identifier: MIT

//! Refusals a save-profile setup admission can report.

/// A bounded refusal from admitting an authored save-profile setup operation.
///
/// The variants are the fixed vocabulary a workflow router branches on. A
/// refusal never carries a supplied profile value, host path or game text: it
/// reports which structural property failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileSetupError {
    /// The setup schema or route contract version is not the one implemented.
    Incompatible,
    /// The request was structurally unusable (missing or malformed identity).
    InvalidRequest,
    /// The operation requires a permission the deployment does not grant.
    PermissionDenied,
    /// The operation names a profile but no profile identity was supplied.
    ProfileRequired,
    /// A read-only operation carried a profile or a baseline fence.
    DiscoveryMustBeEffectFree,
    /// Selection carried no baseline fence, or a disposable provision carried one.
    BaselineFenceMismatch,
    /// The active-run state refuses a mutation for this run.
    ActiveRunConflict,
    /// The authoritative readback did not match the admitted identity.
    ReadbackMismatch,
}

impl std::fmt::Display for ProfileSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Incompatible => "save-profile setup contract version is unsupported",
            Self::InvalidRequest => "save-profile setup request is structurally invalid",
            Self::PermissionDenied => {
                "save-profile operation requires a permission that is not granted"
            }
            Self::ProfileRequired => "save-profile operation requires a profile identity",
            Self::DiscoveryMustBeEffectFree => {
                "save-profile discovery must not name a profile or a baseline fence"
            }
            Self::BaselineFenceMismatch => {
                "save-profile baseline fence does not match the operation"
            }
            Self::ActiveRunConflict => "save-profile mutation conflicts with the active run state",
            Self::ReadbackMismatch => "save-profile readback does not match the admitted identity",
        };
        formatter.write_str(label)
    }
}

impl std::error::Error for ProfileSetupError {}
