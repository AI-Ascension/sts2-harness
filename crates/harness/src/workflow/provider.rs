// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::ids::{
    CapabilityId, ContextId, Digest, FieldId, ProviderExecutionId, RegistryId, SemanticVersion,
};
use super::values::ScalarValue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id: RegistryId,
    pub version: SemanticVersion,
    pub digest: Digest,
    pub capabilities: BTreeSet<CapabilityId>,
    pub max_input_bytes: usize,
    pub max_output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRegistryError {
    InvalidLimit,
    Duplicate,
    Missing,
    CapabilityDenied,
}

impl std::fmt::Display for ProviderRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimit => "provider profile limit is invalid",
            Self::Duplicate => "provider profile is already registered",
            Self::Missing => "provider profile is unavailable",
            Self::CapabilityDenied => "provider profile lacks a required capability",
        })
    }
}

impl std::error::Error for ProviderRegistryError {}

#[derive(Debug, Default, Clone)]
pub struct ProviderRegistry {
    profiles: BTreeMap<(RegistryId, SemanticVersion), ProviderProfile>,
}

impl ProviderRegistry {
    pub fn register(&mut self, profile: ProviderProfile) -> Result<(), ProviderRegistryError> {
        if profile.max_input_bytes == 0
            || profile.max_input_bytes > 128 * 1024
            || profile.max_output_tokens == 0
            || profile.max_output_tokens > 2_000_000
        {
            return Err(ProviderRegistryError::InvalidLimit);
        }
        let key = (profile.id.clone(), profile.version.clone());
        if self.profiles.insert(key, profile).is_some() {
            return Err(ProviderRegistryError::Duplicate);
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        id: &RegistryId,
        version: &SemanticVersion,
        digest: &Digest,
        required: &BTreeSet<CapabilityId>,
    ) -> Result<&ProviderProfile, ProviderRegistryError> {
        let profile = self
            .profiles
            .get(&(id.clone(), version.clone()))
            .ok_or(ProviderRegistryError::Missing)?;
        if &profile.digest != digest || !required.is_subset(&profile.capabilities) {
            return Err(ProviderRegistryError::CapabilityDenied);
        }
        Ok(profile)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderInput {
    pub execution_id: ProviderExecutionId,
    pub context_ref: ContextId,
    pub context_digest: Digest,
    pub fields: BTreeMap<FieldId, ScalarValue>,
}

impl ProviderInput {
    pub fn new(
        execution_id: ProviderExecutionId,
        context_ref: ContextId,
        fields: BTreeMap<FieldId, ScalarValue>,
    ) -> Result<Self, ProviderInputError> {
        let bytes = serde_json::to_vec(&fields).map_err(|_| ProviderInputError::Serialization)?;
        if bytes.is_empty() || bytes.len() > 128 * 1024 {
            return Err(ProviderInputError::TooLarge);
        }
        Ok(Self {
            execution_id,
            context_ref,
            context_digest: Digest::sha256(&bytes),
            fields,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderInputError {
    Serialization,
    TooLarge,
}

impl std::fmt::Display for ProviderInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Serialization => "provider input could not be serialized",
            Self::TooLarge => "provider input exceeds its byte bound",
        })
    }
}

impl std::error::Error for ProviderInputError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationState {
    Reserved,
    Completed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReservation {
    pub execution_id: ProviderExecutionId,
    pub reserved_units: u64,
    pub actual_units: Option<u64>,
    pub state: ReservationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    InvalidLimit,
    Duplicate,
    Capacity,
    Missing,
    Conflict,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimit => "provider budget limit is invalid",
            Self::Duplicate => "provider execution identity is already reserved",
            Self::Capacity => "provider budget has no remaining capacity",
            Self::Missing => "provider reservation is missing",
            Self::Conflict => "provider reservation transition conflicts",
        })
    }
}

impl std::error::Error for BudgetError {}

#[derive(Debug, Clone)]
pub struct BudgetLedger {
    limit: u64,
    reserved: u64,
    reservations: BTreeMap<ProviderExecutionId, BudgetReservation>,
}

impl BudgetLedger {
    pub fn new(limit: u64) -> Result<Self, BudgetError> {
        if limit == 0 || limit > 2_000_000 {
            return Err(BudgetError::InvalidLimit);
        }
        Ok(Self {
            limit,
            reserved: 0,
            reservations: BTreeMap::new(),
        })
    }

    pub fn reserve(
        &mut self,
        execution_id: ProviderExecutionId,
        units: u64,
    ) -> Result<BudgetReservation, BudgetError> {
        if units == 0 || units > self.limit {
            return Err(BudgetError::InvalidLimit);
        }
        if let Some(existing) = self.reservations.get(&execution_id) {
            return if existing.reserved_units == units {
                Ok(existing.clone())
            } else {
                Err(BudgetError::Duplicate)
            };
        }
        let next = self
            .reserved
            .checked_add(units)
            .ok_or(BudgetError::Capacity)?;
        if next > self.limit {
            return Err(BudgetError::Capacity);
        }
        let reservation = BudgetReservation {
            execution_id: execution_id.clone(),
            reserved_units: units,
            actual_units: None,
            state: ReservationState::Reserved,
        };
        self.reserved = next;
        self.reservations.insert(execution_id, reservation.clone());
        Ok(reservation.clone())
    }

    pub fn complete(
        &mut self,
        execution_id: &ProviderExecutionId,
        actual_units: u64,
    ) -> Result<BudgetReservation, BudgetError> {
        self.finish(
            execution_id,
            ReservationState::Completed,
            Some(actual_units),
        )
    }

    pub fn fail(
        &mut self,
        execution_id: &ProviderExecutionId,
        actual_units: Option<u64>,
    ) -> Result<BudgetReservation, BudgetError> {
        self.finish(execution_id, ReservationState::Failed, actual_units)
    }

    pub fn mark_unknown(
        &mut self,
        execution_id: &ProviderExecutionId,
    ) -> Result<BudgetReservation, BudgetError> {
        let reservation = self
            .reservations
            .get_mut(execution_id)
            .ok_or(BudgetError::Missing)?;
        if reservation.state != ReservationState::Reserved {
            return Err(BudgetError::Conflict);
        }
        reservation.state = ReservationState::Unknown;
        Ok(reservation.clone())
    }

    #[must_use]
    pub const fn reserved_units(&self) -> u64 {
        self.reserved
    }

    fn finish(
        &mut self,
        execution_id: &ProviderExecutionId,
        state: ReservationState,
        actual_units: Option<u64>,
    ) -> Result<BudgetReservation, BudgetError> {
        let reservation = self
            .reservations
            .get_mut(execution_id)
            .ok_or(BudgetError::Missing)?;
        if reservation.state != ReservationState::Reserved
            || actual_units.is_some_and(|units| units == 0 || units > reservation.reserved_units)
        {
            return Err(BudgetError::Conflict);
        }
        reservation.state = state;
        reservation.actual_units = actual_units;
        Ok(reservation.clone())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{
        BudgetError, BudgetLedger, ProviderProfile, ProviderRegistry, ProviderRegistryError,
        ReservationState,
    };
    use crate::workflow::{
        CapabilityId, ContextId, Digest, ProviderExecutionId, RegistryId, SemanticVersion,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn profile() -> ProviderProfile {
        ProviderProfile {
            id: RegistryId::new("provider.synthetic").expect("provider id"),
            version: SemanticVersion::new("1.0.0").expect("version"),
            digest: Digest::sha256(b"profile"),
            capabilities: BTreeSet::from([CapabilityId::new("analysis.map").expect("capability")]),
            max_input_bytes: 4096,
            max_output_tokens: 512,
        }
    }

    #[test]
    fn registry_requires_the_pinned_profile_and_capability() {
        let profile = profile();
        let mut registry = ProviderRegistry::default();
        registry.register(profile.clone()).expect("register");
        let required = BTreeSet::from([CapabilityId::new("analysis.map").expect("capability")]);
        assert!(
            registry
                .resolve(&profile.id, &profile.version, &profile.digest, &required)
                .is_ok()
        );
        assert_eq!(
            registry.register(profile),
            Err(ProviderRegistryError::Duplicate)
        );
    }

    #[test]
    fn reserved_budget_is_retained_when_provider_outcome_is_unknown() {
        let mut ledger = BudgetLedger::new(10).expect("budget");
        let execution = ProviderExecutionId::new("execution-1").expect("execution");
        ledger.reserve(execution.clone(), 4).expect("reserve");
        assert_eq!(
            ledger.mark_unknown(&execution).expect("unknown").state,
            ReservationState::Unknown
        );
        assert_eq!(ledger.reserved_units(), 4);
        assert_eq!(
            ledger.reserve(
                ProviderExecutionId::new("execution-2").expect("execution"),
                7,
            ),
            Err(BudgetError::Capacity)
        );
    }

    #[test]
    fn typed_context_input_has_a_bounded_digest() {
        let input = super::ProviderInput::new(
            ProviderExecutionId::new("execution-1").expect("execution"),
            ContextId::new("context-1").expect("context"),
            BTreeMap::new(),
        )
        .expect("input");
        assert_eq!(input.context_digest, Digest::sha256(b"{}"));
    }
}
