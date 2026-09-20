// SPDX-License-Identifier: MIT

//! Execution fences bound to a prepared input.
//!
//! Every axis that could change what the approved bytes mean is bound here.  Any drift makes a
//! held approval stale before a write, so approved material is never dispatched into a different
//! state, catalog, profile, authority, history, compaction or policy environment.

use super::dispatch_error::DispatchError;
use super::valid_identity;

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// One bound axis that drifted away from a held approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriftAxis {
    /// Adapter identity changed.
    Adapter,
    /// Model identity changed.
    Model,
    /// Model or adapter configuration changed.
    Configuration,
    /// Authoritative game state changed.
    State,
    /// Catalog identity changed.
    Catalog,
    /// Profile identity changed.
    Profile,
    /// Authority or authentication identity changed.
    Auth,
    /// Conversation or history identity changed.
    History,
    /// Compaction identity changed.
    Compaction,
    /// Policy identity or version changed.
    Policy,
    /// Controller epoch changed.
    Controller,
    /// Gate epoch changed.
    Gate,
    /// Lease epoch changed.
    Lease,
    /// Revocation epoch changed.
    Revocation,
}

impl DriftAxis {
    /// Stable wire label for the drifted axis.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adapter => "adapter",
            Self::Model => "model",
            Self::Configuration => "configuration",
            Self::State => "state",
            Self::Catalog => "catalog",
            Self::Profile => "profile",
            Self::Auth => "auth",
            Self::History => "history",
            Self::Compaction => "compaction",
            Self::Policy => "policy",
            Self::Controller => "controller_epoch",
            Self::Gate => "gate_epoch",
            Self::Lease => "lease_epoch",
            Self::Revocation => "revocation_epoch",
        }
    }
}

/// Execution fences bound to a prepared input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchFences {
    /// Adapter id the approval was bound to.
    pub adapter_id: String,
    /// Model identity the approval was bound to.
    pub model_id: String,
    /// Digest of the model and adapter configuration.
    pub configuration_digest: String,
    /// Digest of the authoritative state envelope the material was prepared against.
    pub state_digest: String,
    /// Digest of the catalog identity the material was prepared against.
    pub catalog_digest: String,
    /// Digest of the profile identity the material was prepared against.
    pub profile_digest: String,
    /// Digest of the authority the material was prepared under.
    pub auth_digest: String,
    /// Digest of the retained history the material was prepared against.
    pub history_digest: String,
    /// Digest of the compaction identity the material was prepared against.
    pub compaction_digest: String,
    /// Policy identity the approval was bound to.
    pub policy_id: String,
    /// Policy version the approval was bound to.
    pub policy_version: u64,
    /// Controller epoch the approval was bound to.
    pub controller_epoch: u64,
    /// Gate epoch the approval was bound to.
    pub gate_epoch: u64,
    /// Lease epoch the approval was bound to.
    pub lease_epoch: u64,
    /// Revocation epoch the approval was bound to.
    pub revocation_epoch: u64,
}

impl DispatchFences {
    /// Validates every identity and digest before the fences are bound.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::InvalidBinding`] when an identity or digest is malformed.
    pub fn validate(&self) -> Result<(), DispatchError> {
        if !valid_identity(&self.adapter_id)
            || !valid_identity(&self.model_id)
            || !valid_identity(&self.policy_id)
            || !valid_digest(&self.configuration_digest)
            || !valid_digest(&self.state_digest)
            || !valid_digest(&self.catalog_digest)
            || !valid_digest(&self.profile_digest)
            || !valid_digest(&self.auth_digest)
            || !valid_digest(&self.history_digest)
            || !valid_digest(&self.compaction_digest)
        {
            return Err(DispatchError::InvalidBinding);
        }
        Ok(())
    }

    /// The first drifted axis in a fixed order, or `None` when every bound axis still matches.
    pub fn drift(&self, current: &Self) -> Option<DriftAxis> {
        if self.adapter_id != current.adapter_id {
            return Some(DriftAxis::Adapter);
        }
        if self.model_id != current.model_id {
            return Some(DriftAxis::Model);
        }
        if self.configuration_digest != current.configuration_digest {
            return Some(DriftAxis::Configuration);
        }
        if self.state_digest != current.state_digest {
            return Some(DriftAxis::State);
        }
        if self.catalog_digest != current.catalog_digest {
            return Some(DriftAxis::Catalog);
        }
        if self.profile_digest != current.profile_digest {
            return Some(DriftAxis::Profile);
        }
        if self.auth_digest != current.auth_digest {
            return Some(DriftAxis::Auth);
        }
        if self.history_digest != current.history_digest {
            return Some(DriftAxis::History);
        }
        if self.compaction_digest != current.compaction_digest {
            return Some(DriftAxis::Compaction);
        }
        if self.policy_id != current.policy_id || self.policy_version != current.policy_version {
            return Some(DriftAxis::Policy);
        }
        if self.controller_epoch != current.controller_epoch {
            return Some(DriftAxis::Controller);
        }
        if self.gate_epoch != current.gate_epoch {
            return Some(DriftAxis::Gate);
        }
        if self.lease_epoch != current.lease_epoch {
            return Some(DriftAxis::Lease);
        }
        if self.revocation_epoch != current.revocation_epoch {
            return Some(DriftAxis::Revocation);
        }
        None
    }
}
