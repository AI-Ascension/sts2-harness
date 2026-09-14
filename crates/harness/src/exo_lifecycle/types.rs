// SPDX-License-Identifier: MIT

use crate::provider_session::SessionScope;
use crate::{DecisionReference, ExecutionLineage, ProviderReservation};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_LIFECYCLE_ENTRIES: usize = 128;
pub const MAX_INPUT_BYTES: usize = 131_072;

/// Sanitized errors; no path, private input, native output or credential is carried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    Invalid,
    Capacity,
    Unavailable,
    Busy,
    Stale,
    Fenced,
    Held,
    Unknown,
    Corrupt,
    Io,
    Poisoned,
    Unsupported,
    LegacyOwnerNotQuiesced,
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Exo lifecycle {self:?}")
    }
}
impl std::error::Error for LifecycleError {}

/// Explicit owner-provisioned v2 destination. The legacy destination is never overwritten.
#[derive(Clone, Debug)]
pub struct JournalConfig {
    pub directory: PathBuf,
    pub legacy_path: Option<PathBuf>,
    pub store_id: String,
    pub scope: SessionScope,
    pub owner_binding_digest: String,
}

impl JournalConfig {
    pub(crate) fn validate(&self) -> Result<(), LifecycleError> {
        if !self.directory.is_absolute()
            || self.directory.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
            || !self.scope.valid()
            || !id(&self.store_id)
            || !digest(&self.owner_binding_digest)
            || self.legacy_path.as_ref().is_some_and(|path| {
                !path.is_absolute()
                    || path.starts_with(&self.directory)
                    || path.components().any(|part| {
                        !matches!(
                            part,
                            std::path::Component::RootDir | std::path::Component::Normal(_)
                        )
                    })
            })
        {
            return Err(LifecycleError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityVector {
    pub owner_epoch: u64,
    pub auth_epoch: u64,
    pub session_epoch: u64,
    pub history_epoch: u64,
    pub compaction_epoch: u64,
    pub revocation_epoch: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub state_id: String,
    pub generation: u64,
    pub catalog_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationManifest {
    pub scope: SessionScope,
    pub execution_id: String,
    pub episode_attempt_id: String,
    pub trajectory_id: String,
    pub provider_attempt_id: String,
    pub reservation_id: String,
    pub binding_id: String,
    pub operation_id: String,
    pub prepared_id: String,
    pub request_id: String,
    pub host_turn_id: String,
    pub input_digest: String,
    pub input_length: usize,
    pub config_digest: String,
    pub package_digest: String,
    pub profile_digest: String,
    pub model_revision: String,
    pub reserved_units: u64,
    pub authority: AuthorityVector,
}

impl InvocationManifest {
    pub(crate) fn validate(&self) -> Result<(), LifecycleError> {
        let a = &self.authority;
        if !self.scope.valid()
            || self.input_length == 0
            || self.input_length > MAX_INPUT_BYTES
            || self.reserved_units == 0
            || a.owner_epoch == 0
            || a.auth_epoch == 0
            || a.session_epoch == 0
            || a.lease_epoch == 0
            || [
                &self.execution_id,
                &self.episode_attempt_id,
                &self.trajectory_id,
                &self.provider_attempt_id,
                &self.reservation_id,
                &self.binding_id,
                &self.operation_id,
                &self.prepared_id,
                &self.request_id,
                &self.host_turn_id,
                &self.model_revision,
                &a.lease_id,
                &a.state_id,
            ]
            .iter()
            .any(|v| !id(v))
            || [
                &self.input_digest,
                &self.config_digest,
                &self.package_digest,
                &self.profile_digest,
                &a.catalog_digest,
            ]
            .iter()
            .any(|v| !digest(v))
        {
            return Err(LifecycleError::Invalid);
        }
        Ok(())
    }

    pub(crate) fn lineage(&self) -> Result<ExecutionLineage, LifecycleError> {
        ExecutionLineage::new(
            &self.scope.run_id,
            &self.scope.episode_id,
            &self.episode_attempt_id,
            &self.trajectory_id,
        )
        .map_err(|_| LifecycleError::Invalid)
    }

    pub(crate) fn decision(&self) -> Result<DecisionReference, LifecycleError> {
        DecisionReference::new(
            self.lineage()?,
            &self.execution_id,
            &self.input_digest,
            &self.model_revision,
            &self.config_digest,
        )
        .map_err(|_| LifecycleError::Invalid)
    }

    pub(crate) fn reservation(&self) -> Result<ProviderReservation, LifecycleError> {
        ProviderReservation::new(
            self.lineage()?,
            &self.reservation_id,
            &self.execution_id,
            &self.provider_attempt_id,
            self.reserved_units,
        )
        .map_err(|_| LifecycleError::Invalid)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Prepared,
    Admitted,
    Sent,
    Unknown,
    Completed,
    Fenced,
    FailedBeforeSend,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIdentity {
    pub agent_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub event_cursor: String,
}

impl NativeIdentity {
    pub(crate) fn valid(&self) -> bool {
        [
            &self.agent_id,
            &self.conversation_id,
            &self.session_id,
            &self.turn_id,
            &self.event_cursor,
        ]
        .iter()
        .all(|v| id(v))
    }
}

/// Metadata only. The existing ExecutionStore owns the sole result payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEntry {
    pub schema: String,
    pub manifest: InvocationManifest,
    pub phase: LifecyclePhase,
    pub possible_write: bool,
    pub claim_epoch: u64,
    pub permit_revision: Option<u64>,
    pub native: Option<NativeIdentity>,
    pub result_ref: Option<String>,
    pub result_digest: Option<String>,
}

impl LifecycleEntry {
    pub(crate) fn prepared(manifest: InvocationManifest, claim_epoch: u64) -> Self {
        Self {
            schema: "sts2.exo-lifecycle-entry.v1".into(),
            manifest,
            phase: LifecyclePhase::Prepared,
            possible_write: false,
            claim_epoch,
            permit_revision: None,
            native: None,
            result_ref: None,
            result_digest: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), LifecycleError> {
        self.manifest.validate()?;
        if self.schema != "sts2.exo-lifecycle-entry.v1"
            || self.claim_epoch == 0
            || self.claim_epoch != self.manifest.authority.owner_epoch
            || self.possible_write != self.permit_revision.is_some()
            || self.permit_revision == Some(0)
            || self.native.as_ref().is_some_and(|v| !v.valid())
            || self.result_ref.is_some() != self.result_digest.is_some()
            || self.result_ref.as_ref().is_some_and(|v| !id(v))
            || self.result_digest.as_ref().is_some_and(|v| !digest(v))
            || (self.phase == LifecyclePhase::Completed && self.result_ref.is_none())
            || (matches!(
                self.phase,
                LifecyclePhase::Prepared
                    | LifecyclePhase::Admitted
                    | LifecyclePhase::FailedBeforeSend
            ) && self.possible_write)
            || (matches!(
                self.phase,
                LifecyclePhase::Sent | LifecyclePhase::Unknown | LifecyclePhase::Completed
            ) && !self.possible_write)
        {
            return Err(LifecycleError::Corrupt);
        }
        Ok(())
    }
}

pub(crate) fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
}
pub(crate) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
