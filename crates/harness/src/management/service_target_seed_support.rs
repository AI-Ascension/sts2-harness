// SPDX-License-Identifier: MIT

use super::super::super::contract::{
    TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA, TargetSeedSupportCatalogV2, TargetSeedSupportV2,
};
use super::super::seed_v2_support::{
    TARGET_SEED_SUPPORT_REVISION, durable_candidate_modes, ready_candidate_modes,
};
use super::super::support::authorize;
use super::*;

impl ManagementService {
    /// Returns the exact actor-scoped V1 target descriptors with an
    /// independent, non-authoritative seed-support view.
    pub fn target_seed_support_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetSeedSupportCatalogV2, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        let catalog = self.load_target_catalog(actor)?;
        let durable_modes = durable_candidate_modes(self);
        let targets = catalog
            .targets
            .iter()
            .map(|target| {
                Ok(TargetSeedSupportV2 {
                    target: target.clone(),
                    descriptor_digest: target.digest()?,
                    durable_candidate_binding_modes: durable_modes.clone(),
                    ready_candidate_modes: ready_candidate_modes(
                        self,
                        &target.availability,
                        &durable_modes,
                    ),
                    supported_launch_setups: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, ManagementError>>()?;
        let response = TargetSeedSupportCatalogV2 {
            schema_version: TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA.to_owned(),
            catalog_revision: catalog.catalog_revision.clone(),
            support_revision: TARGET_SEED_SUPPORT_REVISION.to_owned(),
            targets,
        };
        response
            .validate_against(&catalog)
            .map_err(ManagementError::from)?;
        Ok(response)
    }
}
