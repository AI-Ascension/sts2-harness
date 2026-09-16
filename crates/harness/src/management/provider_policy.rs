// SPDX-License-Identifier: MIT

//! Authenticated management attachment for the durable provider-session policy
//! owner. The owner is the only component allowed to retain policy bytes or
//! mutate adoption state; this adapter exposes bounded metadata to callers.

use std::sync::Arc;

use crate::provider_session::{ProviderSessionPolicyOwner, ProviderSessionPolicyOwnerError};

use super::contract::ProviderSessionPolicyViewValue;
use super::{AuthContext, ManagementError, RunSnapshot};

#[path = "provider_policy_projection.rs"]
mod projection;

/// Commands accepted by the authenticated, run-scoped policy management API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSessionPolicyOwnerCommand {
    Import {
        policy_bytes: Vec<u8>,
        expected_revision: u64,
    },
    Propose {
        proposal_id: String,
        source_sha256: String,
        target_policy_bytes: Vec<u8>,
        expected_revision: u64,
    },
    Approve {
        proposal_id: String,
        proposal_sha256: String,
        approval_ref: String,
        expected_revision: u64,
    },
    Adopt {
        proposal_id: String,
        proposal_sha256: String,
        approval_ref: String,
        expected_revision: u64,
    },
    AdoptImported {
        policy_sha256: String,
        expected_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSessionPolicyOwnerCommandResult {
    Imported { policy_sha256: String },
    Proposed { proposal_sha256: String },
    Approved,
    Adopted,
}

/// Explicit owner command boundary. Implementations must authenticate and
/// scope every operation to the supplied workflow snapshot.
pub trait ProviderSessionPolicyCommandPort: Send + Sync {
    fn current(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionPolicyViewValue, ManagementError>;

    fn execute(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _command: ProviderSessionPolicyOwnerCommand,
    ) -> Result<ProviderSessionPolicyOwnerCommandResult, ManagementError> {
        Err(ManagementError::unavailable(
            "provider_session_policy_commands_unavailable",
            "provider-session policy mutation commands are not attached",
        ))
    }
}

pub struct DurableProviderSessionPolicyCommandPort {
    owner: Arc<ProviderSessionPolicyOwner>,
}

impl DurableProviderSessionPolicyCommandPort {
    #[must_use]
    pub fn new(owner: Arc<ProviderSessionPolicyOwner>) -> Self {
        Self { owner }
    }

    fn authorize_scope(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
        scope: &str,
    ) -> Result<(), ManagementError> {
        if !actor.can(scope) {
            return Err(ManagementError::forbidden(
                "missing_scope",
                "authenticated actor lacks the required management scope",
            ));
        }
        if !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "run_scope_denied",
                "authenticated actor is not authorized for this workflow run",
            ));
        }
        if self.owner.scope().run_id != snapshot.workflow_run_id {
            return Err(ManagementError::conflict(
                "provider_session_policy_scope_mismatch",
                "provider-session policy owner is scoped to a different workflow run",
            ));
        }
        Ok(())
    }
}

impl ProviderSessionPolicyCommandPort for DurableProviderSessionPolicyCommandPort {
    fn current(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionPolicyViewValue, ManagementError> {
        self.authorize_scope(actor, snapshot, "workflow:read")?;
        let metadata = self.owner.metadata().map_err(owner_error)?;
        projection::view_value(metadata, snapshot)
    }

    fn execute(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
        command: ProviderSessionPolicyOwnerCommand,
    ) -> Result<ProviderSessionPolicyOwnerCommandResult, ManagementError> {
        self.authorize_scope(actor, snapshot, "workflow:control")?;
        if matches!(
            &command,
            ProviderSessionPolicyOwnerCommand::Import { .. }
                | ProviderSessionPolicyOwnerCommand::Propose { .. }
        ) && !actor.can("workflow:content:write")
        {
            return Err(ManagementError::forbidden(
                "missing_scope",
                "authenticated actor lacks the required policy-content upload scope",
            ));
        }
        let result = match command {
            ProviderSessionPolicyOwnerCommand::Import {
                policy_bytes,
                expected_revision,
            } => ProviderSessionPolicyOwnerCommandResult::Imported {
                policy_sha256: self
                    .owner
                    .import_at_revision(policy_bytes, expected_revision)
                    .map_err(owner_error)?,
            },
            ProviderSessionPolicyOwnerCommand::Propose {
                proposal_id,
                source_sha256,
                target_policy_bytes,
                expected_revision,
            } => ProviderSessionPolicyOwnerCommandResult::Proposed {
                proposal_sha256: self
                    .owner
                    .propose(
                        &proposal_id,
                        &source_sha256,
                        target_policy_bytes,
                        expected_revision,
                    )
                    .map_err(owner_error)?,
            },
            ProviderSessionPolicyOwnerCommand::Approve {
                proposal_id,
                proposal_sha256,
                approval_ref,
                expected_revision,
            } => {
                self.owner
                    .approve_at_revision(
                        &proposal_id,
                        &proposal_sha256,
                        &approval_ref,
                        expected_revision,
                    )
                    .map_err(owner_error)?;
                ProviderSessionPolicyOwnerCommandResult::Approved
            }
            ProviderSessionPolicyOwnerCommand::Adopt {
                proposal_id,
                proposal_sha256,
                approval_ref,
                expected_revision,
            } => {
                self.owner
                    .adopt(
                        &proposal_id,
                        &proposal_sha256,
                        &approval_ref,
                        expected_revision,
                    )
                    .map_err(owner_error)?;
                ProviderSessionPolicyOwnerCommandResult::Adopted
            }
            ProviderSessionPolicyOwnerCommand::AdoptImported {
                policy_sha256,
                expected_revision,
            } => {
                self.owner
                    .adopt_imported(&policy_sha256, expected_revision)
                    .map_err(owner_error)?;
                ProviderSessionPolicyOwnerCommandResult::Adopted
            }
        };
        Ok(result)
    }
}

pub struct UnavailableProviderSessionPolicyCommandPort;

impl ProviderSessionPolicyCommandPort for UnavailableProviderSessionPolicyCommandPort {
    fn current(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionPolicyViewValue, ManagementError> {
        Err(ManagementError::unavailable(
            "provider_session_policy_owner_unavailable",
            "durable provider-session policy owner is not attached",
        ))
    }
}

fn owner_error(error: ProviderSessionPolicyOwnerError) -> ManagementError {
    match error {
        ProviderSessionPolicyOwnerError::Invalid => ManagementError::invalid(
            "provider_session_policy_owner_invalid",
            "provider-session policy owner record is invalid",
        ),
        ProviderSessionPolicyOwnerError::Missing => ManagementError::invalid(
            "provider_session_policy_owner_missing",
            "provider-session policy owner record was not found",
        ),
        ProviderSessionPolicyOwnerError::Conflict => ManagementError::conflict(
            "provider_session_policy_owner_conflict",
            "provider-session policy owner revision or identity conflicts",
        ),
        ProviderSessionPolicyOwnerError::NotAdopted => ManagementError::capability(
            "provider_session_policy_not_adopted",
            "provider-session policy owner has no active adopted policy",
        ),
        ProviderSessionPolicyOwnerError::Busy => ManagementError::unavailable(
            "provider_session_policy_owner_busy",
            "provider-session policy journal already has an authoritative owner",
        ),
        ProviderSessionPolicyOwnerError::Store => ManagementError::store(
            "provider_session_policy_owner_store",
            "provider-session policy owner storage is unavailable",
        ),
    }
}
