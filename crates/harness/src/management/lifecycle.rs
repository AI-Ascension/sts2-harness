// SPDX-License-Identifier: MIT

//! Typed process-lifecycle execution port for the served-live composition.
//!
//! The harness never owns a game process. It submits one closed action to the
//! gateway's already-authenticated `sts2-gateway-process-lifecycle-v1` surface
//! and reconciles the answer by operation identity. Three properties are
//! structural rather than advisory:
//!
//! 1. **No caller-supplied effect.** A submission names only an opaque approved
//!    launch profile id, a stop mode, or the exact process identity a caller
//!    previously received. There is no command, path, URL, environment, or
//!    adoption input anywhere in this module, so an arbitrary effect cannot be
//!    expressed even by a buggy caller.
//! 2. **Exact instance identity.** The target instance is resolved from the
//!    run's own admitted target binding. A run without that binding fails
//!    closed, so there is no configured or default instance to fall back to.
//! 3. **Launch is not readiness.** [`LaunchAcknowledgement`] is deliberately
//!    not the type [`LifecycleReadiness`] accepts as evidence; see
//!    `lifecycle_readiness.rs`.

use serde::{Deserialize, Serialize};

use super::auth::AuthContext;
use super::contract::validate_identifier;
use super::service::ManagementError;

/// Versioned contract this port speaks. It is the gateway's identifier, not a
/// harness-local one, so a mismatch is detected rather than silently accepted.
pub const PROCESS_LIFECYCLE_CONTRACT: &str = "sts2-gateway-process-lifecycle-v1";

/// Schema of the harness-owned lifecycle command envelope.
pub const PROCESS_LIFECYCLE_COMMAND_SCHEMA_VERSION: &str = "ascension.process-lifecycle-command/v1";

/// Schema of the harness-owned lifecycle operation status projection.
pub const PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION: &str = "ascension.process-lifecycle-status/v1";

/// Largest accepted lifecycle command body.
pub const MAX_LIFECYCLE_COMMAND_BYTES: usize = 8 * 1024;
/// Longest accepted lifecycle command id.
pub const MAX_LIFECYCLE_COMMAND_ID_BYTES: usize = 128;
/// Largest accepted approved-profile catalog.
pub const MAX_APPROVED_PROFILES: usize = 64;

#[path = "lifecycle_action.rs"]
mod action;
#[path = "lifecycle_view.rs"]
mod view;

pub use action::{LaunchProfileId, LifecycleAction, LifecycleProcessIdentity, StopMode};
pub use view::{
    LifecycleFailure, LifecycleOperationState, LifecycleOperationView, LifecycleState,
    ProcessLifecycleCapability,
};

/// The instance identity one lifecycle command targets.
///
/// Resolved from the run's admitted target binding, never from configuration or
/// a caller default, so a run cannot silently act on a different instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleTarget {
    pub instance_id: String,
}

impl LifecycleTarget {
    /// Builds a target from an admitted instance identity.
    pub fn new(instance_id: impl Into<String>) -> Result<Self, ManagementError> {
        let instance_id = instance_id.into();
        validate_identifier("instance_id", &instance_id)?;
        Ok(Self { instance_id })
    }
}

/// The gateway-owned lifecycle surface.
///
/// Implementations own the transport and the gateway credential. Nothing in
/// this signature lets an implementation accept a command, a path, or a URL
/// from its caller.
pub trait ProcessLifecyclePort: Send + Sync {
    /// Reads the advertised contract, approved profiles, and authority epoch.
    fn capability(
        &self,
        actor: &AuthContext,
        target: &LifecycleTarget,
    ) -> Result<ProcessLifecycleCapability, ManagementError>;

    /// Submits exactly one action under one operation identity.
    fn submit(
        &self,
        actor: &AuthContext,
        target: &LifecycleTarget,
        command: &LifecycleCommand,
        action: &LifecycleAction,
    ) -> Result<LifecycleOperationView, ManagementError>;

    /// Reconciles a retained operation by its identity.
    fn lookup(
        &self,
        actor: &AuthContext,
        target: &LifecycleTarget,
        operation_id: u64,
    ) -> Result<LifecycleOperationView, ManagementError>;
}

/// A caller-supplied lifecycle command envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleCommand {
    pub schema_version: String,
    pub command_id: String,
    pub run_id: String,
    pub expected_revision: u64,
    pub actor_scope: String,
    pub operation_id: u64,
    pub authority_epoch: u64,
    pub action: LifecycleAction,
}

/// Result of one accepted or reconciled lifecycle command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleCommandResponse {
    pub schema_version: String,
    pub command_id: String,
    pub workflow_run_id: String,
    pub operation_id: u64,
    pub instance_id: String,
    pub run_revision: u64,
    pub classification: LifecycleClassification,
    pub operation_state: LifecycleOperationState,
    pub state: LifecycleState,
    pub authority_epoch: u64,
    pub reason_code: String,
    /// Whether an authoritative gameplay-readiness claim exists. Always
    /// `false` here: this surface reports process lifecycle only.
    pub gameplay_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<LifecycleFailure>,
}

/// Durable classification of one lifecycle command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleClassification {
    /// The gateway accepted the operation and reported a durable outcome.
    Accepted,
    /// The gateway may have applied the operation; reconcile by identity.
    Unknown,
    /// The gateway refused the operation before any effect.
    Rejected,
    /// A stop settled; later starts are fenced by stop dominance.
    Stopped,
}

impl LifecycleClassification {
    /// Stable label recorded in the durable journal and run events.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Unknown => "unknown",
            Self::Rejected => "rejected",
            Self::Stopped => "stopped",
        }
    }
}

/// Composed port used when no lifecycle owner is attached.
pub struct UnavailableProcessLifecyclePort;

impl ProcessLifecyclePort for UnavailableProcessLifecyclePort {
    fn capability(
        &self,
        _actor: &AuthContext,
        _target: &LifecycleTarget,
    ) -> Result<ProcessLifecycleCapability, ManagementError> {
        Err(unavailable())
    }

    fn submit(
        &self,
        _actor: &AuthContext,
        _target: &LifecycleTarget,
        _command: &LifecycleCommand,
        _action: &LifecycleAction,
    ) -> Result<LifecycleOperationView, ManagementError> {
        Err(unavailable())
    }

    fn lookup(
        &self,
        _actor: &AuthContext,
        _target: &LifecycleTarget,
        _operation_id: u64,
    ) -> Result<LifecycleOperationView, ManagementError> {
        Err(unavailable())
    }
}

fn unavailable() -> ManagementError {
    ManagementError::unavailable(
        "process_lifecycle_owner_unavailable",
        "no gateway process-lifecycle owner is attached to this composition",
    )
}

/// Validates the closed command envelope before any gateway call.
pub fn validate_lifecycle_command(
    command: &LifecycleCommand,
    expected_instance: &str,
) -> Result<(), ManagementError> {
    if command.schema_version != PROCESS_LIFECYCLE_COMMAND_SCHEMA_VERSION {
        return Err(ManagementError::invalid(
            "lifecycle_command_schema",
            "lifecycle command schema version is unsupported",
        ));
    }
    validate_identifier("command_id", &command.command_id)?;
    validate_identifier("run_id", &command.run_id)?;
    validate_identifier("actor_scope", &command.actor_scope)?;
    if command.command_id.len() > MAX_LIFECYCLE_COMMAND_ID_BYTES {
        return Err(ManagementError::invalid(
            "lifecycle_command_id_oversized",
            "lifecycle command id exceeds its bound",
        ));
    }
    if command.operation_id == 0 || command.authority_epoch == 0 {
        return Err(ManagementError::invalid(
            "lifecycle_operation_identity_invalid",
            "operation id and authority epoch must both be non-zero",
        ));
    }
    command.action.validate(expected_instance)
}
