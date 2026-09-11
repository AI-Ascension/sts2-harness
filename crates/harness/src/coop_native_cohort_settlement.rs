// SPDX-License-Identifier: MIT

impl CoopNativeCohort {
    fn validate_converged_roster(
        &self,
        observations: &[CoopNativeEnvelope],
        settled: &CoopNativeObservation,
    ) -> Result<(), CoopNativeCohortError> {
        if observations.len() != self.routes.len() {
            return Err(CoopNativeCohortError::RosterNotConverged);
        }
        let mut by_local_peer = BTreeMap::new();
        for envelope in observations {
            validate_profile(envelope)?;
            let observation = envelope
                .observation()
                .filter(|_| envelope.kind() == CoopNativeKind::Observation)
                .ok_or(CoopNativeCohortError::RosterNotConverged)?;
            validate_ready_observation(observation).map_err(|_| CoopNativeCohortError::RosterNotConverged)?;
            let local = sole_local_peer(observation).ok_or(CoopNativeCohortError::RosterNotConverged)?;
            let route = self
                .routes
                .get(local)
                .ok_or(CoopNativeCohortError::RosterNotConverged)?;
            if !same_fence(route.observation(), envelope)
                || observation.run_id() != settled.run_id()
                || observation.authority_id() != settled.authority_id()
                || observation.checkpoint_id() != settled.checkpoint_id()
                || observation.state_digest() != settled.state_digest()
                || roster_of(observation) != self.roster
            {
                return Err(CoopNativeCohortError::RosterNotConverged);
            }
            if by_local_peer.insert(local.clone(), observation).is_some() {
                return Err(CoopNativeCohortError::RosterNotConverged);
            }
        }
        if by_local_peer.keys().cloned().collect::<BTreeSet<_>>() != self.roster {
            return Err(CoopNativeCohortError::RosterNotConverged);
        }
        let checksum = settled.native_checksum().ok_or(CoopNativeCohortError::RosterNotConverged)?;
        if settled.checksum_status() != CoopNativeChecksumStatus::Matched
            || by_local_peer.values().any(|observation| {
                observation.checksum_status() != CoopNativeChecksumStatus::Matched
                    || observation.native_checksum() != Some(checksum)
            })
        {
            return Err(CoopNativeCohortError::RosterNotConverged);
        }
        Ok(())
    }
}
