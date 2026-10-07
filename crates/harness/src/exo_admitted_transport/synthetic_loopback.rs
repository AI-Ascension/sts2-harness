// SPDX-License-Identifier: MIT

use crate::exo::{
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoDecisionKind, ExoIdentityError,
    ExoLimits, ExoPlatform, ExoProfile, ExoRestrictedError, ExoRuntime, ExoTransport,
    ExoTransportError, ExoTrustedConfiguration, ExoWireError,
};
use crate::exo_bridge_configuration::SyntheticLoopbackInspection;
use crate::{
    EXO_CAPABILITY_SCHEMA, EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoIdentity,
    ExoPrivateStatePolicy, ExoProcessConfig, ExoProcessTransport,
};

use super::strict_envelope::{Correlation, StrictAdmission, StrictEnvelope};

const SYNTHETIC_DECISIONS: [ExoDecisionKind; 4] = [
    ExoDecisionKind::Action,
    ExoDecisionKind::Plan,
    ExoDecisionKind::Wait,
    ExoDecisionKind::Reobserve,
];

/// Source-derived synthetic admission data, separate from the production preflight report.
///
/// This record reports only the exact identity and bounded standard/fresh one-shot profile. It
/// does not contain a model-call claim or promote lifecycle, recovery, idempotency, or native
/// capabilities from the descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticExoAdmissionReport {
    pub identity: ExoIdentity,
    pub platform: ExoPlatform,
    pub profile: ExoProfile,
    pub context_mode: ExoContextMode,
    pub runtime: ExoRuntime,
    pub limits: ExoLimits,
}

/// A validated synthetic launch whose only transport is the retained concrete bridge process.
pub struct SyntheticExoAdmissionPlan {
    process: ExoProcessConfig,
    admission: StrictAdmission,
    correlation: Correlation,
    report: SyntheticExoAdmissionReport,
}

impl SyntheticExoAdmissionPlan {
    /// Validates an opaque file inspection against the independent descriptor and trusted profile.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the identity, profile, private-state policy, capability set
    /// or host-owned correlation IDs do not match the inspected launch.
    pub fn new(
        descriptor: &ExoCapabilityDescriptor,
        trusted: &ExoTrustedConfiguration,
        inspection: SyntheticLoopbackInspection,
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Result<Self, SyntheticExoAdmissionError> {
        let (process, identity, private_state) = inspection.into_parts();
        let admission = validate_admission(descriptor, trusted, &identity, &private_state)?;
        let correlation = Correlation::new(model_execution_id, request_id, turn_id)
            .map_err(SyntheticExoAdmissionError::Wire)?;
        let report = SyntheticExoAdmissionReport {
            identity: identity.clone(),
            platform: ExoPlatform::LinuxX86_64,
            profile: ExoProfile::Standard,
            context_mode: ExoContextMode::Fresh,
            runtime: ExoRuntime::Responses,
            limits: ExoLimits::reviewed(),
        };
        Ok(Self {
            process,
            admission,
            correlation,
            report,
        })
    }

    /// Builds the concrete process transport without starting a child or contacting a provider.
    #[must_use]
    pub fn into_transport(self) -> SyntheticExoAdmittedTransport {
        SyntheticExoAdmittedTransport {
            core: StrictEnvelope::new(
                ExoProcessTransport::new(self.process),
                self.admission,
                self.correlation,
            ),
            report: self.report,
        }
    }
}

/// Synthetic-only strict one-shot fence over the inspected bridge process.
pub struct SyntheticExoAdmittedTransport {
    core: StrictEnvelope<ExoProcessTransport>,
    report: SyntheticExoAdmissionReport,
}

impl SyntheticExoAdmittedTransport {
    /// Returns the source-derived synthetic profile bound at plan construction.
    #[must_use]
    pub fn admission(&self) -> &SyntheticExoAdmissionReport {
        &self.report
    }
}

impl ExoTransport for SyntheticExoAdmittedTransport {
    fn exchange(
        &mut self,
        bytes: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.core
            .exchange(bytes, max_response_bytes, timeout_millis)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.core.close()
    }
}

fn validate_admission(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
    identity: &ExoIdentity,
    private_state: &ExoPrivateStatePolicy,
) -> Result<StrictAdmission, SyntheticExoAdmissionError> {
    validate_identity(descriptor, trusted, identity)?;
    validate_profile(descriptor, trusted)?;
    trusted
        .restricted
        .validate()
        .map_err(SyntheticExoAdmissionError::Restricted)?;
    if &trusted.restricted.state != private_state {
        return Err(SyntheticExoAdmissionError::PrivateStateMismatch);
    }
    let decision_kinds = supported_decisions(descriptor)?;
    Ok(StrictAdmission {
        identity: identity.clone(),
        profile: ExoProfile::Standard,
        limits: ExoLimits::reviewed(),
        decision_kinds,
    })
}

fn validate_identity(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
    identity: &ExoIdentity,
) -> Result<(), SyntheticExoAdmissionError> {
    identity
        .validate_synthetic_loopback()
        .map_err(SyntheticExoAdmissionError::Identity)?;
    if !identity.is_complete()
        || identity.source_revision != EXO_SOURCE_REVISION
        || identity.contract_version != EXO_CONTRACT_VERSION
        || identity.provider.as_deref() != Some("openai")
        || &descriptor.identity != identity
        || &trusted.identity != identity
    {
        return Err(SyntheticExoAdmissionError::IdentityMismatch);
    }
    if descriptor.schema_version != EXO_CAPABILITY_SCHEMA
        || descriptor.contract_version != EXO_CONTRACT_VERSION
    {
        return Err(SyntheticExoAdmissionError::DescriptorMismatch);
    }
    Ok(())
}

fn validate_profile(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
) -> Result<(), SyntheticExoAdmissionError> {
    if trusted.platform != ExoPlatform::LinuxX86_64
        || trusted.profile != ExoProfile::Standard
        || trusted.context_mode != ExoContextMode::Fresh
        || trusted.runtime != ExoRuntime::Responses
        || trusted.limits != ExoLimits::reviewed()
        || descriptor.limits != ExoLimits::reviewed()
        || descriptor.profile_support.standard != ExoCapabilityState::Supported
        || !descriptor.context_modes.contains(&ExoContextMode::Fresh)
        || has_duplicates(&descriptor.context_modes)
        || !descriptor.platforms.contains(&ExoPlatform::LinuxX86_64)
        || has_duplicates(&descriptor.platforms)
    {
        return Err(SyntheticExoAdmissionError::ProfileMismatch);
    }
    Ok(())
}

fn supported_decisions(
    descriptor: &ExoCapabilityDescriptor,
) -> Result<Vec<ExoDecisionKind>, SyntheticExoAdmissionError> {
    if descriptor.decision_kinds.is_empty()
        || !descriptor.decision_kinds.contains(&ExoDecisionKind::Action)
        || has_duplicates(&descriptor.decision_kinds)
    {
        return Err(SyntheticExoAdmissionError::DecisionKindsMismatch);
    }
    Ok(SYNTHETIC_DECISIONS
        .into_iter()
        .filter(|kind| descriptor.decision_kinds.contains(kind))
        .collect())
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

/// Typed synthetic refusal reasons; no path, config contents or provider material is attached.
#[derive(Debug)]
pub enum SyntheticExoAdmissionError {
    Identity(ExoIdentityError),
    Restricted(ExoRestrictedError),
    Wire(ExoWireError),
    IdentityMismatch,
    DescriptorMismatch,
    ProfileMismatch,
    DecisionKindsMismatch,
    PrivateStateMismatch,
}

impl std::fmt::Display for SyntheticExoAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(error) => error.fmt(formatter),
            Self::Restricted(error) => error.fmt(formatter),
            Self::Wire(error) => error.fmt(formatter),
            Self::IdentityMismatch => {
                formatter.write_str("synthetic Exo identities do not match inspection")
            }
            Self::DescriptorMismatch => {
                formatter.write_str("synthetic Exo descriptor version is unsupported")
            }
            Self::ProfileMismatch => {
                formatter.write_str("synthetic Exo profile is outside reviewed bounds")
            }
            Self::DecisionKindsMismatch => {
                formatter.write_str("synthetic Exo decision capability set is invalid")
            }
            Self::PrivateStateMismatch => {
                formatter.write_str("synthetic Exo private-state policy does not match")
            }
        }
    }
}

impl std::error::Error for SyntheticExoAdmissionError {}

#[cfg(test)]
#[path = "synthetic_loopback_tests.rs"]
mod tests;
