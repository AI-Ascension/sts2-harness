// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex, MutexGuard};

use sts2_harness::exo_lifecycle::{
    AuthorityGuard, InvocationManifest, LifecycleAuthorityPort, LifecycleError, OwnerClaim,
};
use sts2_harness::provider_session::SessionScope;
use sts2_harness::{
    EpisodeLegalActionSet, EpisodeObservation, ExecutionLineage, ExoDecisionRequest,
};

#[derive(Clone, Default)]
pub(super) struct RuntimeLifecycleAuthorityState(Arc<Mutex<AuthoritySnapshot>>);

#[derive(Default)]
struct AuthoritySnapshot {
    enabled: bool,
    lease: Option<LeaseAuthority>,
    turn: Option<TurnAuthority>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LeaseAuthority {
    pub(super) id: String,
    pub(super) epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TurnAuthority {
    pub(super) lease: LeaseAuthority,
    pub(super) state_id: String,
    pub(super) generation: u64,
    pub(super) catalog_digest: String,
}

impl RuntimeLifecycleAuthorityState {
    pub(super) fn enable(&self) -> Result<(), LifecycleError> {
        let mut state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        if !state.enabled {
            state.enabled = true;
            state.lease = None;
            state.turn = None;
        }
        Ok(())
    }

    pub(super) fn activate(&self, lease_id: &str, lease_epoch: u64) -> Result<(), LifecycleError> {
        let mut state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        if !state.enabled {
            return Ok(());
        }
        if lease_id.is_empty() || lease_epoch == 0 {
            return Err(LifecycleError::Invalid);
        }
        state.lease = Some(LeaseAuthority {
            id: lease_id.to_owned(),
            epoch: lease_epoch,
        });
        state.turn = None;
        Ok(())
    }

    pub(super) fn observe(
        &self,
        observation: &EpisodeObservation,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), LifecycleError> {
        let mut state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        if !state.enabled {
            return Ok(());
        }
        if observation.state_id() != actions.state_id()
            || observation.generation() != actions.generation()
        {
            return Err(LifecycleError::Stale);
        }
        let catalog_digest = catalog_digest(actions)?;
        let lease = state.lease.clone().ok_or(LifecycleError::Fenced)?;
        state.turn = Some(TurnAuthority {
            lease,
            state_id: observation.state_id().to_owned(),
            generation: observation.generation(),
            catalog_digest,
        });
        Ok(())
    }

    pub(super) fn update_catalog(
        &self,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), LifecycleError> {
        let mut state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        if !state.enabled {
            return Ok(());
        }
        let catalog_digest = catalog_digest(actions)?;
        let turn = state.turn.as_mut().ok_or(LifecycleError::Fenced)?;
        if turn.state_id != actions.state_id() || turn.generation != actions.generation() {
            return Err(LifecycleError::Stale);
        }
        turn.catalog_digest = catalog_digest;
        Ok(())
    }

    pub(super) fn bind_request(
        &self,
        request: &ExoDecisionRequest,
    ) -> Result<TurnAuthority, LifecycleError> {
        let state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        let turn = state.turn.as_ref().ok_or(LifecycleError::Fenced)?;
        if request.state_id != turn.state_id
            || request.generation != turn.generation
            || catalog_digest_ids(&request.legal_action_ids)? != turn.catalog_digest
        {
            return Err(LifecycleError::Stale);
        }
        Ok(turn.clone())
    }

    pub(super) fn invalidate(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.turn = None;
            state.lease = None;
        }
    }

    pub(super) fn clear_turn(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.turn = None;
        }
    }

    fn lock_for_manifest<'a>(
        &'a self,
        scope: &SessionScope,
        lineage: &ExecutionLineage,
        config_digest: &str,
        manifest: &InvocationManifest,
    ) -> Result<AuthorityGuardBox<'a>, LifecycleError> {
        let state = self.0.lock().map_err(|_| LifecycleError::Fenced)?;
        if !state.enabled {
            return Err(LifecycleError::Fenced);
        }
        let turn = state.turn.as_ref().ok_or(LifecycleError::Fenced)?;
        if manifest.scope != *scope
            || manifest.scope.run_id != lineage.run_id
            || manifest.scope.episode_id != lineage.episode_id
            || manifest.episode_attempt_id != lineage.attempt_id
            || manifest.trajectory_id != lineage.trajectory_id
            || manifest.config_digest != config_digest
            || manifest.authority.lease_id != turn.lease.id
            || manifest.authority.lease_epoch != turn.lease.epoch
            || manifest.authority.state_id != turn.state_id
            || manifest.authority.generation != turn.generation
            || manifest.authority.catalog_digest != turn.catalog_digest
        {
            return Err(LifecycleError::Stale);
        }
        Ok(Box::new(StateGuard { _state: state }))
    }
}

type AuthorityGuardBox<'a> = Box<dyn AuthorityGuard + 'a>;

struct StateGuard<'a> {
    _state: MutexGuard<'a, AuthoritySnapshot>,
}
impl AuthorityGuard for StateGuard<'_> {}

struct ClaimGuard;
impl AuthorityGuard for ClaimGuard {}

pub(super) struct RuntimeAuthority {
    pub(super) scope: SessionScope,
    pub(super) state: RuntimeLifecycleAuthorityState,
    pub(super) lineage: ExecutionLineage,
    pub(super) config_digest: String,
    pub(super) owner_binding_digest: String,
}

impl LifecycleAuthorityPort for RuntimeAuthority {
    fn claim<'a>(
        &'a self,
        request: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        if request.config.scope != self.scope
            || request.config.owner_binding_digest != self.owner_binding_digest
        {
            return Err(LifecycleError::Fenced);
        }
        Ok(Box::new(ClaimGuard))
    }

    fn admit<'a>(
        &'a self,
        manifest: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.state
            .lock_for_manifest(&self.scope, &self.lineage, &self.config_digest, manifest)
    }

    fn consume<'a>(
        &'a self,
        manifest: &InvocationManifest,
        result_digest: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        if result_digest.len() != 64
            || !result_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(LifecycleError::Invalid);
        }
        self.admit(manifest)
    }
}

fn catalog_digest(actions: &EpisodeLegalActionSet) -> Result<String, LifecycleError> {
    let ids: Vec<_> = actions
        .actions()
        .iter()
        .map(|action| action.action_id().to_owned())
        .collect();
    catalog_digest_ids(&ids)
}

fn catalog_digest_ids(ids: &[String]) -> Result<String, LifecycleError> {
    serde_json::to_vec(ids)
        .map(sts2_harness::sha256_hex)
        .map_err(|_| LifecycleError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_digest_matches_the_provider_request_projection() {
        let ids = vec![String::from("combat.end-turn"), String::from("combat.play")];
        assert_eq!(
            catalog_digest_ids(&ids).expect("catalog digest"),
            sts2_harness::sha256_hex(
                serde_json::to_vec(&json!(["combat.end-turn", "combat.play"]))
                    .expect("encoded ids")
            )
        );
    }
}
