// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::support::authorize;
use super::*;
use crate::context_memory::policy_owner::{PolicyCommand, SavedPolicyRef};

/// Authenticated management attachment for the selected-memory-policy owner.
/// The bearer is rechecked by the owner with its fixed trusted grant; request
/// bodies never select an owner grant.
pub trait MemoryPolicyOwnerManagementPort: Send + Sync {
    fn current(&self, bearer: Option<&str>) -> Result<Value, ManagementError>;

    fn inspect_policy(
        &self,
        bearer: Option<&str>,
        reference: &SavedPolicyRef,
    ) -> Result<String, ManagementError>;

    fn inspect_review(
        &self,
        bearer: Option<&str>,
        review_id: &str,
    ) -> Result<Value, ManagementError>;

    fn execute(
        &self,
        bearer: Option<&str>,
        command: PolicyCommand,
    ) -> Result<Value, ManagementError>;
}

pub struct UnavailableMemoryPolicyOwnerManagementPort;

impl MemoryPolicyOwnerManagementPort for UnavailableMemoryPolicyOwnerManagementPort {
    fn current(&self, _bearer: Option<&str>) -> Result<Value, ManagementError> {
        Err(unavailable())
    }

    fn inspect_policy(
        &self,
        _bearer: Option<&str>,
        _reference: &SavedPolicyRef,
    ) -> Result<String, ManagementError> {
        Err(unavailable())
    }

    fn inspect_review(
        &self,
        _bearer: Option<&str>,
        _review_id: &str,
    ) -> Result<Value, ManagementError> {
        Err(unavailable())
    }

    fn execute(
        &self,
        _bearer: Option<&str>,
        _command: PolicyCommand,
    ) -> Result<Value, ManagementError> {
        Err(unavailable())
    }
}

fn unavailable() -> ManagementError {
    ManagementError::unavailable(
        "memory_policy_owner_unavailable",
        "selected-memory policy management is not attached",
    )
}

impl ManagementService {
    pub fn with_memory_policy_owner_management_port(
        mut self,
        port: Arc<dyn MemoryPolicyOwnerManagementPort>,
    ) -> Self {
        self.memory_policy_owner = port;
        self
    }

    pub fn memory_policy_owner_current(
        &self,
        actor: &AuthContext,
        bearer: Option<&str>,
    ) -> Result<Value, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        self.memory_policy_owner.current(bearer)
    }

    pub fn memory_policy_owner_inspect_policy(
        &self,
        actor: &AuthContext,
        bearer: Option<&str>,
        reference: &SavedPolicyRef,
    ) -> Result<String, ManagementError> {
        authorize(actor, "workflow:content:read", None)?;
        self.memory_policy_owner.inspect_policy(bearer, reference)
    }

    pub fn memory_policy_owner_inspect_review(
        &self,
        actor: &AuthContext,
        bearer: Option<&str>,
        review_id: &str,
    ) -> Result<Value, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        self.memory_policy_owner.inspect_review(bearer, review_id)
    }

    pub fn memory_policy_owner_execute(
        &self,
        actor: &AuthContext,
        bearer: Option<&str>,
        command: PolicyCommand,
    ) -> Result<Value, ManagementError> {
        authorize(actor, "workflow:control", None)?;
        if matches!(
            &command,
            PolicyCommand::ProposeMigration { .. } | PolicyCommand::ProposeRevalidation { .. }
        ) {
            authorize(actor, "workflow:content:write", None)?;
        }
        self.memory_policy_owner.execute(bearer, command)
    }
}
