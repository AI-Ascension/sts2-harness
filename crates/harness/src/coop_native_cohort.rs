// SPDX-License-Identifier: MIT

use super::artifact::{COOP_NATIVE_PROTOCOL_VERSION, COOP_NATIVE_SCHEMA_DIGEST};
use super::identity::{CoopNativeOperationId, CoopNativePeerId};
use super::wire::{
    CoopNativeChecksumStatus, CoopNativeEnvelope, CoopNativeKind, CoopNativeObservation,
    CoopNativePeerRole, CoopNativeStatus,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One canonical local peer and its current v1 observation/catalog pair.
///
/// Gateway route credentials are transport secrets. This pure component neither receives nor stores
/// them; every peer comparison is between canonical `peer:` identities from v1 envelopes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopNativeCohortRoute {
    canonical_peer: CoopNativePeerId,
    observation: CoopNativeEnvelope,
    catalog: CoopNativeEnvelope,
}

impl CoopNativeCohortRoute {
    #[must_use]
    pub fn new(
        canonical_peer: CoopNativePeerId,
        observation: CoopNativeEnvelope,
        catalog: CoopNativeEnvelope,
    ) -> Self {
        Self {
            canonical_peer,
            observation,
            catalog,
        }
    }

    #[must_use]
    pub fn canonical_peer(&self) -> &CoopNativePeerId {
        &self.canonical_peer
    }

    #[must_use]
    pub fn observation(&self) -> &CoopNativeEnvelope {
        &self.observation
    }

    #[must_use]
    pub fn catalog(&self) -> &CoopNativeEnvelope {
        &self.catalog
    }
}

/// A scheduled local action bound to the only route and fence permitted for recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopNativeCohortOperation {
    route_peer: CoopNativePeerId,
    operation_id: CoopNativeOperationId,
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    expected_host_generation: u64,
}

impl CoopNativeCohortOperation {
    #[must_use]
    pub fn route_peer(&self) -> &CoopNativePeerId {
        &self.route_peer
    }

    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }
}

/// A validated local cohort. It owns no transport, lease, host, or vote authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopNativeCohort {
    routes: BTreeMap<CoopNativePeerId, CoopNativeCohortRoute>,
    roster: BTreeSet<CoopNativePeerId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativeCohortError {
    ParticipantCount,
    ProfileMismatch,
    RouteShape,
    RouteFenceMismatch,
    DuplicateRoute,
    LocalPeerMismatch,
    RosterMismatch,
    RunMismatch,
    AuthorityMismatch,
    NotReady,
    CheckpointMismatch,
    CatalogMismatch,
    ActionNotCurrent,
    OperationMismatch,
    RecoveryNotOriginalRoute,
    RecoveryNotRequest,
    SettlementNotOriginalRoute,
    SettlementNotSettled,
    SettlementEvidence,
    RosterNotConverged,
}

impl fmt::Display for CoopNativeCohortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ParticipantCount => "native co-op cohort requires two through four routes",
            Self::ProfileMismatch => "native co-op route does not carry the exact v1 profile",
            Self::RouteShape => "native co-op route does not carry observation and catalog responses",
            Self::RouteFenceMismatch => "native co-op route observation and catalog fences differ",
            Self::DuplicateRoute => "native co-op cohort contains a duplicate local route",
            Self::LocalPeerMismatch => "canonical route peer is not the sole local peer",
            Self::RosterMismatch => "native co-op routes do not report the selected roster",
            Self::RunMismatch => "native co-op routes report different runs",
            Self::AuthorityMismatch => "native co-op routes report different authorities",
            Self::NotReady => "native co-op roster is disconnected, loading, divergent, or unknown",
            Self::CheckpointMismatch => "native co-op roster lacks a compatible checkpoint",
            Self::CatalogMismatch => "native co-op catalog is not bound to its observation",
            Self::ActionNotCurrent => "native co-op action is not in the current actor catalog",
            Self::OperationMismatch => "native co-op operation does not match its original binding",
            Self::RecoveryNotOriginalRoute => "native co-op recovery is not on the original route fence",
            Self::RecoveryNotRequest => "native co-op recovery must be an original recovery request",
            Self::SettlementNotOriginalRoute => "native co-op settlement is not on the original route fence",
            Self::SettlementNotSettled => "native co-op response is not a settled host response",
            Self::SettlementEvidence => "native co-op settlement lacks fresh original-operation evidence",
            Self::RosterNotConverged => "native co-op roster has not converged after settlement",
        })
    }
}

impl std::error::Error for CoopNativeCohortError {}

impl CoopNativeCohort {
    pub fn validate(routes: Vec<CoopNativeCohortRoute>) -> Result<Self, CoopNativeCohortError> {
        if !(2..=4).contains(&routes.len()) {
            return Err(CoopNativeCohortError::ParticipantCount);
        }

        let mut route_map = BTreeMap::new();
        let mut expected_roster = None;
        let mut expected_run = None;
        let mut expected_authority = None;
        let mut expected_checkpoint = None;
        let mut expected_state_digest = None;
        for route in routes {
            validate_profile(route.observation())?;
            validate_profile(route.catalog())?;
            let observation = route
                .observation()
                .observation()
                .filter(|_| route.observation().kind() == CoopNativeKind::Observation)
                .ok_or(CoopNativeCohortError::RouteShape)?;
            let catalog_response = route
                .catalog()
                .legal_catalog_response()
                .ok_or(CoopNativeCohortError::RouteShape)?;
            if !same_fence(route.observation(), route.catalog()) {
                return Err(CoopNativeCohortError::RouteFenceMismatch);
            }
            if catalog_response.observation() != observation
                || catalog_response.catalog().host_generation() != observation.host_generation()
                || catalog_response.catalog().actor_peer() != route.canonical_peer()
            {
                return Err(CoopNativeCohortError::CatalogMismatch);
            }
            let locals = observation
                .peers()
                .iter()
                .filter(|peer| peer.role() == CoopNativePeerRole::Local)
                .collect::<Vec<_>>();
            if locals.len() != 1 || locals[0].peer_token() != route.canonical_peer() {
                return Err(CoopNativeCohortError::LocalPeerMismatch);
            }
            validate_ready_observation(observation)?;
            let roster = observation
                .peers()
                .iter()
                .map(|peer| peer.peer_token().clone())
                .collect::<BTreeSet<_>>();
            if expected_roster.as_ref().is_some_and(|expected| expected != &roster) {
                return Err(CoopNativeCohortError::RosterMismatch);
            }
            expected_roster = Some(roster);
            if expected_run
                .as_ref()
                .is_some_and(|run| run != observation.run_id())
            {
                return Err(CoopNativeCohortError::RunMismatch);
            }
            expected_run = Some(observation.run_id().clone());
            if expected_authority
                .as_ref()
                .is_some_and(|authority| authority != observation.authority_id())
            {
                return Err(CoopNativeCohortError::AuthorityMismatch);
            }
            expected_authority = Some(observation.authority_id().clone());
            if expected_checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint != observation.checkpoint_id())
                || expected_state_digest
                    .as_ref()
                    .is_some_and(|digest| digest != observation.state_digest())
            {
                return Err(CoopNativeCohortError::CheckpointMismatch);
            }
            expected_checkpoint = Some(observation.checkpoint_id().clone());
            expected_state_digest = Some(observation.state_digest().to_owned());
            if route_map.insert(route.canonical_peer().clone(), route).is_some() {
                return Err(CoopNativeCohortError::DuplicateRoute);
            }
        }
        let roster = expected_roster.ok_or(CoopNativeCohortError::ParticipantCount)?;
        if roster.len() != route_map.len() || roster != route_map.keys().cloned().collect() {
            return Err(CoopNativeCohortError::RosterMismatch);
        }
        Ok(Self {
            routes: route_map,
            roster,
        })
    }

    #[must_use]
    pub fn roster(&self) -> &BTreeSet<CoopNativePeerId> {
        &self.roster
    }

    pub fn schedule_local_action(
        &self,
        route_peer: &CoopNativePeerId,
        request: &CoopNativeEnvelope,
    ) -> Result<CoopNativeCohortOperation, CoopNativeCohortError> {
        let route = self
            .routes
            .get(route_peer)
            .ok_or(CoopNativeCohortError::LocalPeerMismatch)?;
        validate_profile(request)?;
        let action = request
            .action_request()
            .ok_or(CoopNativeCohortError::ActionNotCurrent)?;
        if !same_fence(route.observation(), request)
            || action.actor_peer() != route_peer
            || action.expected_host_generation()
                != route
                    .observation()
                    .observation()
                    .ok_or(CoopNativeCohortError::RouteShape)?
                    .host_generation()
        {
            return Err(CoopNativeCohortError::ActionNotCurrent);
        }
        let catalog = route
            .catalog()
            .legal_catalog_response()
            .ok_or(CoopNativeCohortError::RouteShape)?
            .catalog();
        if catalog.host_generation() != action.expected_host_generation()
            || !catalog.actions().contains(action.action())
        {
            return Err(CoopNativeCohortError::ActionNotCurrent);
        }
        Ok(operation_for(route_peer.clone(), action.operation_id().clone(), request, action.expected_host_generation()))
    }

    pub fn validate_settlement(
        &self,
        operation: &CoopNativeCohortOperation,
        route_peer: &CoopNativePeerId,
        response: &CoopNativeEnvelope,
        refreshed_observations: &[CoopNativeEnvelope],
    ) -> Result<(), CoopNativeCohortError> {
        let route = self
            .routes
            .get(route_peer)
            .ok_or(CoopNativeCohortError::SettlementNotOriginalRoute)?;
        if operation.route_peer() != route_peer || !same_operation_fence(operation, response) || !same_fence(route.observation(), response) {
            return Err(CoopNativeCohortError::SettlementNotOriginalRoute);
        }
        let result = response
            .effect_response()
            .ok_or(CoopNativeCohortError::SettlementNotSettled)?;
        let receipt = result.receipt();
        let effect = result.effect().ok_or(CoopNativeCohortError::SettlementEvidence)?;
        validate_ready_observation(result.observation())
            .map_err(|_| CoopNativeCohortError::SettlementEvidence)?;
        if roster_of(result.observation()) != self.roster {
            return Err(CoopNativeCohortError::RosterNotConverged);
        }
        if result.status() != CoopNativeStatus::Settled
            || receipt.status() != CoopNativeStatus::Settled
            || result.operation_id() != operation.operation_id()
            || receipt.operation_id() != operation.operation_id()
            || effect.operation_id() != operation.operation_id()
            || receipt.before_host_generation() != operation.expected_host_generation()
            || receipt.after_host_generation() != Some(result.observation().host_generation())
            || result.observation().host_generation() <= operation.expected_host_generation()
            || effect.from_generation() != operation.expected_host_generation()
            || effect.to_generation() != result.observation().host_generation()
        {
            return Err(CoopNativeCohortError::SettlementEvidence);
        }
        self.validate_converged_roster(refreshed_observations, result.observation())
    }

}

include!("coop_native_cohort_settlement.rs");
include!("coop_native_cohort_recovery.rs");
include!("coop_native_cohort_validation.rs");
