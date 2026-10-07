// SPDX-License-Identifier: MIT

//! Additive, closed target seed-support discovery contracts.
//!
//! Candidate reservation support is kept separate from launch setup. This
//! version has no launch setup variants, so a valid response advertises none.

use serde::{Deserialize, Serialize};

use super::super::contract_seed_v2::SeedModeV2;
use super::json::{ContractError, validate_digest, validate_identifier};
use super::target_admission::{
    MAX_TARGETS, TARGET_CATALOG_SCHEMA_VERSION, TargetCatalogResponse, TargetDescriptor,
};

pub const TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA: &str = "ascension.workflow-targets/v2";
pub const MAX_SEED_SUPPORT_TARGETS: usize = MAX_TARGETS;
pub const MAX_SEED_SUPPORT_MODES: usize = 2;
pub const MAX_SEED_LAUNCH_SETUPS_V2: usize = 0;

/// No launch setup is currently source-backed by the owner contract.
///
/// A future non-empty setup catalog requires a separately versioned schema;
/// this uninhabited enum makes this version's launch list strictly empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeedLaunchSetupV2 {}

impl Serialize for SeedLaunchSetupV2 {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for SeedLaunchSetupV2 {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(<D::Error as serde::de::Error>::custom(
            "launch setups are unavailable in target seed support v2",
        ))
    }
}

/// One exact V1 target descriptor plus independent candidate and launch support.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSeedSupportV2 {
    pub target: TargetDescriptor,
    pub descriptor_digest: String,
    pub durable_candidate_binding_modes: Vec<SeedModeV2>,
    pub ready_candidate_modes: Vec<SeedModeV2>,
    pub supported_launch_setups: Vec<SeedLaunchSetupV2>,
}

impl TargetSeedSupportV2 {
    pub fn validate(&self) -> Result<(), ContractError> {
        self.target.validate()?;
        validate_digest("descriptor_digest", &self.descriptor_digest)?;
        if self.target.digest()? != self.descriptor_digest {
            return Err(ContractError::new(
                "seed_support_descriptor_digest",
                "seed support descriptor digest does not match its target",
            ));
        }
        validate_unique_modes(
            "durable_candidate_binding_modes",
            &self.durable_candidate_binding_modes,
        )?;
        validate_unique_modes("ready_candidate_modes", &self.ready_candidate_modes)?;
        if self
            .ready_candidate_modes
            .iter()
            .any(|mode| !self.durable_candidate_binding_modes.contains(mode))
        {
            return Err(ContractError::new(
                "seed_support_ready_without_durable_support",
                "ready candidate modes must also be durably supported",
            ));
        }
        if self.supported_launch_setups.len() > MAX_SEED_LAUNCH_SETUPS_V2 {
            return Err(ContractError::new(
                "seed_support_launch_setup_unavailable",
                "this seed support schema does not define launch setups",
            ));
        }
        Ok(())
    }
}

/// V2 discovery correlates exact V1 descriptors without changing V1 bytes or
/// conflating the target catalog revision with seed policy revision.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSeedSupportCatalogV2 {
    pub schema_version: String,
    pub catalog_revision: String,
    pub support_revision: String,
    pub targets: Vec<TargetSeedSupportV2>,
}

impl TargetSeedSupportCatalogV2 {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA {
            return Err(ContractError::new(
                "seed_support_schema",
                "target seed support schema version is unsupported",
            ));
        }
        validate_identifier("catalog_revision", &self.catalog_revision)?;
        validate_identifier("support_revision", &self.support_revision)?;
        if self.targets.len() > MAX_SEED_SUPPORT_TARGETS {
            return Err(ContractError::new(
                "seed_support_capacity",
                "target seed support catalog exceeds the supported bound",
            ));
        }
        let mut instance_ids = std::collections::BTreeSet::new();
        for target in &self.targets {
            target.validate()?;
            if !instance_ids.insert(target.target.instance_id.as_str()) {
                return Err(ContractError::new(
                    "seed_support_duplicate_target",
                    "target seed support instance IDs must be unique",
                ));
            }
        }
        Ok(())
    }

    /// Require exact descriptor coverage of the current strict V1 catalog.
    pub fn validate_against(&self, catalog: &TargetCatalogResponse) -> Result<(), ContractError> {
        self.validate()?;
        catalog.validate()?;
        if catalog.schema_version != TARGET_CATALOG_SCHEMA_VERSION
            || self.catalog_revision != catalog.catalog_revision
            || self.targets.len() != catalog.targets.len()
        {
            return Err(ContractError::new(
                "seed_support_catalog_mismatch",
                "seed support catalog does not match the current target catalog",
            ));
        }
        for descriptor in &catalog.targets {
            let Some(support) = self
                .targets
                .iter()
                .find(|support| support.target.instance_id == descriptor.instance_id)
            else {
                return Err(ContractError::new(
                    "seed_support_catalog_mismatch",
                    "seed support catalog is missing a target descriptor",
                ));
            };
            if &support.target != descriptor || support.descriptor_digest != descriptor.digest()? {
                return Err(ContractError::new(
                    "seed_support_catalog_mismatch",
                    "seed support target does not exactly match the target catalog",
                ));
            }
        }
        Ok(())
    }
}

fn validate_unique_modes(field: &str, modes: &[SeedModeV2]) -> Result<(), ContractError> {
    if modes.len() > MAX_SEED_SUPPORT_MODES {
        return Err(ContractError::new(
            "seed_support_mode_capacity",
            format!("{field} exceeds the supported bound"),
        ));
    }
    for (index, mode) in modes.iter().enumerate() {
        if modes[index + 1..].contains(mode) {
            return Err(ContractError::new(
                "seed_support_duplicate_mode",
                format!("{field} must contain unique modes"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "contract_seed_support_v2_tests.rs"]
mod tests;
