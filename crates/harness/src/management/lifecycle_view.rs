// SPDX-License-Identifier: MIT

//! Typed projections of the gateway's lifecycle answers.
//!
//! Each projection revalidates its own scope: a foreign contract, operation,
//! or instance is refused rather than reconciled, so a stale or wrong-scope
//! answer can never be recorded as this command's outcome.

use serde::{Deserialize, Serialize};

use crate::management::ManagementError;

use super::{
    LaunchProfileId, LifecycleProcessIdentity, MAX_APPROVED_PROFILES, PROCESS_LIFECYCLE_CONTRACT,
};

/// Gateway-reported lifecycle state of the instance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum LifecycleState {
    Created,
    Starting,
    Ready,
    Busy,
    Degraded,
    Stopping,
    Stopped,
    Failed,
    Unknown,
    Expired,
}

/// Gateway-reported state of the submitted operation itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum LifecycleOperationState {
    IntentRecorded,
    Starting,
    Started,
    Attached,
    Stopping,
    Restarting,
    Stopped,
    Failed,
    Blocked,
    Rejected,
    Unknown,
}

/// Typed failure the gateway attached to a retained operation.
///
/// The gateway stores a failure label rather than a message, so no host path,
/// process argument, or private payload can arrive through this field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFailure {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One authoritative lifecycle answer, keyed by its operation identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleOperationView {
    pub contract: String,
    pub operation_id: u64,
    pub instance_id: String,
    pub state: LifecycleState,
    pub operation_state: LifecycleOperationState,
    pub process: Option<LifecycleProcessIdentity>,
    pub authority_epoch: u64,
    pub failure: Option<LifecycleFailure>,
}

impl LifecycleOperationView {
    /// Validates the answer against the request that produced it.
    ///
    /// A mismatched operation, instance, or contract is refused rather than
    /// reconciled, so a stale or foreign answer can never be recorded as this
    /// command's outcome.
    pub fn validate(
        &self,
        expected_operation: u64,
        expected_instance: &str,
    ) -> Result<(), ManagementError> {
        if self.contract != PROCESS_LIFECYCLE_CONTRACT
            || self.operation_id != expected_operation
            || self.instance_id != expected_instance
            || self.authority_epoch == 0
        {
            return Err(ManagementError::conflict(
                "lifecycle_response_scope_mismatch",
                "lifecycle answer is not bound to the submitted operation",
            ));
        }
        Ok(())
    }
}

/// Advertised lifecycle capability of the deployment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProcessLifecycleCapability {
    pub contract: String,
    /// Whether effects are possible. `false` means every submission is refused.
    pub available: bool,
    pub profiles: Vec<LaunchProfileId>,
    /// Current authority epoch, or `None` when the gateway has no epoch yet.
    pub authority_epoch: Option<u64>,
    pub instance_id: String,
    pub unavailable_reason: Option<String>,
}

impl ProcessLifecycleCapability {
    /// Validates the advertised capability against the instance the run
    /// admitted.
    ///
    /// A foreign contract, a mismatched instance, or an unbounded profile
    /// catalog is refused rather than reported, so a wrong-instance capability
    /// answer can never be mistaken for this run's surface.
    pub fn validate(&self, expected_instance: &str) -> Result<(), ManagementError> {
        if self.contract != PROCESS_LIFECYCLE_CONTRACT || self.instance_id != expected_instance {
            return Err(ManagementError::conflict(
                "lifecycle_capability_scope_mismatch",
                "lifecycle capability is not bound to the admitted instance",
            ));
        }
        if self.profiles.len() > MAX_APPROVED_PROFILES {
            return Err(ManagementError::unavailable(
                "lifecycle_capability_unbounded",
                "lifecycle capability advertises more profiles than the harness accepts",
            ));
        }
        Ok(())
    }
}
