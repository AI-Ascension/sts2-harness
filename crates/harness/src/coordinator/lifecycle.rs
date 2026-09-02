// SPDX-License-Identifier: MIT

use super::{ActiveEpisode, EpisodeHandle, Harness, HarnessParts};
use crate::artifact::ArtifactPort;
use crate::error::{CloseFailure, CloseReport, Component, HarnessError, PortError};
use crate::provider::ProviderPort;
use crate::records::RecordPort;
use crate::replay::ReplayPort;
use crate::routing::InstanceRouter;

impl<R, P, S, A, X> Harness<R, P, S, A, X>
where
    R: InstanceRouter,
    P: ProviderPort,
    S: RecordPort,
    A: ArtifactPort,
    X: ReplayPort,
{
    pub fn close_episode(&mut self, episode: &EpisodeHandle) -> Result<(), HarnessError> {
        self.ensure_open()?;
        let binding = self.active_episode(episode)?.handle.binding.clone();
        self.router
            .unbind(&binding)
            .map_err(HarnessError::Routing)?;
        self.episodes.remove(&episode.episode_id);
        Ok(())
    }

    pub fn close(&mut self) -> Result<CloseReport, HarnessError> {
        if let Some(outcome) = &self.closed_outcome {
            return outcome.clone();
        }

        let bindings = self
            .episodes
            .values()
            .map(|active| active.handle.binding.clone())
            .collect::<Vec<_>>();
        self.episodes.clear();
        let mut report = CloseReport {
            unbound_episodes: 0,
            closed_components: Vec::new(),
            failures: Vec::new(),
        };
        for binding in &bindings {
            match self.router.unbind(binding) {
                Ok(()) => report.unbound_episodes += 1,
                Err(error) => report.failures.push(CloseFailure {
                    component: Component::Routing,
                    error,
                }),
            }
        }
        close_component(&mut report, Component::Routing, self.router.close());
        close_component(&mut report, Component::Provider, self.provider.close());
        close_component(&mut report, Component::Storage, self.storage.close());
        close_component(&mut report, Component::Replay, self.replay.close());
        close_component(&mut report, Component::Artifact, self.artifacts.close());
        let outcome = if report.is_clean() {
            Ok(report)
        } else {
            Err(HarnessError::Cleanup(report))
        };
        self.closed_outcome = Some(outcome.clone());
        outcome
    }

    #[must_use]
    pub fn into_parts(self) -> HarnessParts<R, P, S, A, X> {
        HarnessParts {
            router: self.router,
            provider: self.provider,
            storage: self.storage,
            artifacts: self.artifacts,
            replay: self.replay,
        }
    }

    pub(super) fn ensure_open(&self) -> Result<(), HarnessError> {
        if self.closed_outcome.is_some() {
            Err(HarnessError::Closed)
        } else {
            Ok(())
        }
    }

    pub(super) fn active_episode(
        &self,
        episode: &EpisodeHandle,
    ) -> Result<&ActiveEpisode, HarnessError> {
        let active = self
            .episodes
            .get(&episode.episode_id)
            .ok_or_else(|| HarnessError::Invalid("episode is not active".to_owned()))?;
        if active.handle != *episode {
            return Err(HarnessError::Invalid(
                "episode handle does not match the active episode".to_owned(),
            ));
        }
        Ok(active)
    }

    pub(super) fn active_episode_mut(
        &mut self,
        episode: &EpisodeHandle,
    ) -> Result<&mut ActiveEpisode, HarnessError> {
        let active = self
            .episodes
            .get_mut(&episode.episode_id)
            .ok_or_else(|| HarnessError::Invalid("episode is not active".to_owned()))?;
        if active.handle != *episode {
            return Err(HarnessError::Invalid(
                "episode handle does not match the active episode".to_owned(),
            ));
        }
        Ok(active)
    }
}

fn close_component(report: &mut CloseReport, component: Component, result: Result<(), PortError>) {
    match result {
        Ok(()) => report.closed_components.push(component),
        Err(error) => report.failures.push(CloseFailure { component, error }),
    }
}
