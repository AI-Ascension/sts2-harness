// SPDX-License-Identifier: MIT

//! The closed lifecycle action vocabulary.
//!
//! One command carries exactly one action, and no action carries a command, a
//! path, a URL, an environment, or an adoption input. The only identity a
//! caller can name is an opaque approved profile id or the exact process
//! identity an earlier authoritative answer already reported.

use serde::{Deserialize, Serialize};

use crate::management::ManagementError;
use crate::management::contract::validate_identifier;

/// One opaque approved launch-profile identity.
///
/// The value is meaningless on its own: the gateway resolves it against the
/// catalog it configured, and an id it did not approve is refused with
/// `422 process_lifecycle_profile_rejected`. The harness therefore cannot name
/// an executable, a path, or an argument list.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LaunchProfileId(u64);

impl LaunchProfileId {
    /// Builds a profile identity, rejecting the reserved zero value.
    pub fn new(value: u64) -> Result<Self, ManagementError> {
        if value == 0 {
            return Err(ManagementError::invalid(
                "lifecycle_profile_id_invalid",
                "approved launch profile id must be non-zero",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the opaque numeric identity.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for LaunchProfileId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Stop semantics. Closed: no signal number or command reaches the gateway.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    /// Ask the process to exit.
    Graceful,
    /// Terminate without asking.
    Force,
}

impl StopMode {
    /// Returns the exact wire label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Force => "force",
        }
    }
}

/// The exact process identity a caller echoes back for `attach_existing`.
///
/// Every field was received from an earlier authoritative answer. This is not
/// an adoption mechanism: the gateway independently compares the identity with
/// the one it retained for that operation and refuses a foreign or unowned one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleProcessIdentity {
    pub instance_id: String,
    pub process: u64,
    pub pid: u64,
    pub birth_id: u64,
    pub install_id: u64,
    pub executable_id: u64,
    pub image_id: u64,
    pub namespace_id: u64,
}

impl LifecycleProcessIdentity {
    fn validate(&self, expected_instance: &str) -> Result<(), ManagementError> {
        validate_identifier("instance_id", &self.instance_id)?;
        if self.instance_id != expected_instance {
            return Err(ManagementError::forbidden(
                "lifecycle_instance_mismatch",
                "attach identity belongs to a different instance",
            ));
        }
        if self.process == 0 || self.pid == 0 || self.birth_id == 0 || self.namespace_id == 0 {
            return Err(ManagementError::invalid(
                "lifecycle_process_identity_invalid",
                "attach identity is incomplete",
            ));
        }
        Ok(())
    }
}

/// The one closed action a lifecycle command may carry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleAction {
    /// Start a new process from an approved profile.
    LaunchNew { profile_id: LaunchProfileId },
    /// Restart the instance from an approved profile.
    Restart { profile_id: LaunchProfileId },
    /// Stop the instance.
    Stop { mode: StopMode },
    /// Re-attach the exact identity a previous operation reported.
    AttachExisting { identity: LifecycleProcessIdentity },
}

impl LifecycleAction {
    /// Stable reason code recorded in durable intent and run events.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::LaunchNew { .. } => "launch_new",
            Self::Restart { .. } => "restart",
            Self::Stop { .. } => "stop",
            Self::AttachExisting { .. } => "attach_existing",
        }
    }

    /// True when the action can change which process is serving the instance.
    ///
    /// Stop dominance uses this: a stop is never reordered behind a later
    /// start, and a start is never replayed after a stop settled.
    #[must_use]
    pub const fn is_effect_bearing(&self) -> bool {
        matches!(self, Self::LaunchNew { .. } | Self::Restart { .. })
    }

    pub(super) fn validate(&self, instance_id: &str) -> Result<(), ManagementError> {
        match self {
            Self::AttachExisting { identity } => identity.validate(instance_id),
            _ => Ok(()),
        }
    }
}
