// SPDX-License-Identifier: MIT

//! The fixed save-profile operation vocabulary and its accepted MCP mapping.
//!
//! Each operation maps to exactly one accepted tool name and one gateway route
//! shape. Nothing here is assembled from caller-supplied text: an operation
//! resolves to a fixed tool identifier and a route whose only interpolated
//! component is an identity the admission step already validated.

pub const PROFILE_ROUTE_REVISION: &str = "save-profile-v1-mcp";
pub const PROFILE_ROUTE_CONTRACT: &str = "gateway-save-profile-v1";

/// The separate permission an operation requires.
///
/// Discovery, selection and provisioning are distinct grants so that a
/// read-only deployment cannot select a save profile, and a consumer that may
/// select an existing profile cannot allocate a new one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileGrant {
    /// Listing, reading the current profile and reading a retained receipt.
    Discovery,
    /// Selecting one existing profile with an explicit baseline fence.
    Selection,
    /// Requesting one isolated disposable automation profile.
    Provisioning,
}

/// An authored save-profile setup operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileSetupOperation {
    /// List bounded save-profile summaries.
    List,
    /// Read the current save-profile summary.
    Current,
    /// Read one retained operation receipt by its identity.
    Status,
    /// Select one existing profile under a baseline fence.
    Select,
    /// Request one isolated disposable profile.
    CreateDisposable,
}

impl ProfileSetupOperation {
    /// The accepted MCP tool name for this operation.
    #[must_use]
    pub const fn tool(self) -> &'static str {
        match self {
            Self::List => "sts2.save_profile_list",
            Self::Current => "sts2.save_profile_current",
            Self::Status => "sts2.save_profile_status",
            Self::Select => "sts2.save_profile_select",
            Self::CreateDisposable => "sts2.save_profile_create_disposable",
        }
    }

    /// The permission this operation requires.
    #[must_use]
    pub const fn required_grant(self) -> ProfileGrant {
        match self {
            Self::List | Self::Current | Self::Status => ProfileGrant::Discovery,
            Self::Select => ProfileGrant::Selection,
            Self::CreateDisposable => ProfileGrant::Provisioning,
        }
    }

    /// Whether this operation changes owner state.
    #[must_use]
    pub const fn is_mutation(self) -> bool {
        matches!(self, Self::Select | Self::CreateDisposable)
    }

    /// The fixed route suffix this operation targets.
    #[must_use]
    pub const fn route_suffix(self) -> &'static str {
        match self {
            Self::List => "save-profiles",
            Self::Current => "save-profile/current",
            Self::Status => "save-profile/operations",
            Self::Select => "save-profile/select",
            Self::CreateDisposable => "save-profile/create-disposable",
        }
    }

    /// Whether this operation names one existing profile.
    #[must_use]
    pub const fn requires_profile_id(self) -> bool {
        matches!(self, Self::Select)
    }

    /// Whether this operation must carry an explicit baseline fence.
    ///
    /// Only selection is fenced. A disposable provision cannot require one,
    /// because the baseline it would name does not exist until the gateway
    /// allocates the profile and reports it back.
    #[must_use]
    pub const fn requires_baseline_fence(self) -> bool {
        matches!(self, Self::Select)
    }

    /// Whether this operation reconciles a retained receipt by identity.
    #[must_use]
    pub const fn requires_operation_id(self) -> bool {
        matches!(self, Self::Status)
    }

    /// The gateway route for this operation on one validated instance.
    ///
    /// `instance_id` and `operation_id` are validated identities before this is
    /// reached, so the only interpolation is a bounded, portable identifier.
    #[must_use]
    pub fn route_path(self, instance_id: &str, operation_id: Option<&str>) -> String {
        let suffix = self.route_suffix();
        match (self, operation_id) {
            (Self::Status, Some(operation_id)) => {
                format!("/v1/instances/{instance_id}/{suffix}/{operation_id}")
            }
            _ => format!("/v1/instances/{instance_id}/{suffix}"),
        }
    }
}
