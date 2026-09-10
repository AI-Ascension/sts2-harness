// SPDX-License-Identifier: MIT

use super::wire::{
    CoopNativeEffectResponse, CoopNativeEnvelope, CoopNativeKind, CoopNativeLegalCatalogRequest,
    CoopNativeLegalCatalogResponse, CoopNativeLocalActionRequest, CoopNativeObservation,
    CoopNativeRecoveryResponse, CoopNativeRejoinRequest, CoopNativeSharedVoteRequest,
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativePortError {
    Rejected,
    Unavailable,
    InvalidEnvelope,
}

impl fmt::Display for CoopNativePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => "native co-op port rejected the envelope",
            Self::Unavailable => "native co-op port is unavailable",
            Self::InvalidEnvelope => "native co-op port received an incomplete typed envelope",
        })
    }
}

impl std::error::Error for CoopNativePortError {}

/// A transport-free typed boundary for the eight component envelope shapes.
pub trait CoopNativePort {
    fn consume(&mut self, envelope: &CoopNativeEnvelope) -> Result<(), CoopNativePortError> {
        match envelope.kind() {
            CoopNativeKind::Observation => envelope
                .observation()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_observation(value)
                }),
            CoopNativeKind::LegalCatalogRequest => envelope
                .legal_catalog_request()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_legal_catalog_request(value)
                }),
            CoopNativeKind::LegalCatalogResponse => envelope
                .legal_catalog_response()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_legal_catalog_response(value)
                }),
            CoopNativeKind::LocalActionRequest => envelope
                .action_request()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_local_action(value)
                }),
            CoopNativeKind::SharedVoteRequest => envelope
                .vote_request()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_shared_vote(value)
                }),
            CoopNativeKind::RejoinRequest => envelope
                .rejoin_request()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_rejoin(value)
                }),
            CoopNativeKind::EffectResponse => envelope
                .effect_response()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_effect(value)
                }),
            CoopNativeKind::RecoveryResponse => envelope
                .recovery_response()
                .map_or(Err(CoopNativePortError::InvalidEnvelope), |value| {
                    self.consume_recovery(value)
                }),
        }
    }

    fn consume_observation(
        &mut self,
        observation: &CoopNativeObservation,
    ) -> Result<(), CoopNativePortError> {
        self.on_observation(observation)
    }

    fn on_observation(
        &mut self,
        _observation: &CoopNativeObservation,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_legal_catalog_request(
        &mut self,
        request: &CoopNativeLegalCatalogRequest,
    ) -> Result<(), CoopNativePortError> {
        self.on_legal_catalog_request(request)
    }

    fn on_legal_catalog_request(
        &mut self,
        _request: &CoopNativeLegalCatalogRequest,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_legal_catalog_response(
        &mut self,
        response: &CoopNativeLegalCatalogResponse,
    ) -> Result<(), CoopNativePortError> {
        self.on_legal_catalog_response(response)
    }

    fn on_legal_catalog_response(
        &mut self,
        _response: &CoopNativeLegalCatalogResponse,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_local_action(
        &mut self,
        request: &CoopNativeLocalActionRequest,
    ) -> Result<(), CoopNativePortError> {
        self.consume_action_request(request)
    }

    fn consume_action_request(
        &mut self,
        request: &CoopNativeLocalActionRequest,
    ) -> Result<(), CoopNativePortError> {
        self.on_local_action(request)
    }

    fn on_local_action(
        &mut self,
        _request: &CoopNativeLocalActionRequest,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_shared_vote(
        &mut self,
        request: &CoopNativeSharedVoteRequest,
    ) -> Result<(), CoopNativePortError> {
        self.consume_vote_request(request)
    }

    fn consume_vote_request(
        &mut self,
        request: &CoopNativeSharedVoteRequest,
    ) -> Result<(), CoopNativePortError> {
        self.on_shared_vote(request)
    }

    fn on_shared_vote(
        &mut self,
        _request: &CoopNativeSharedVoteRequest,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_rejoin(
        &mut self,
        request: &CoopNativeRejoinRequest,
    ) -> Result<(), CoopNativePortError> {
        self.consume_rejoin_request(request)
    }

    fn consume_rejoin_request(
        &mut self,
        request: &CoopNativeRejoinRequest,
    ) -> Result<(), CoopNativePortError> {
        self.on_rejoin(request)
    }

    fn on_rejoin(
        &mut self,
        _request: &CoopNativeRejoinRequest,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_effect(
        &mut self,
        response: &CoopNativeEffectResponse,
    ) -> Result<(), CoopNativePortError> {
        self.consume_effect_response(response)
    }

    fn consume_effect_response(
        &mut self,
        response: &CoopNativeEffectResponse,
    ) -> Result<(), CoopNativePortError> {
        self.on_effect(response)
    }

    fn on_effect(
        &mut self,
        _response: &CoopNativeEffectResponse,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }

    fn consume_recovery(
        &mut self,
        response: &CoopNativeRecoveryResponse,
    ) -> Result<(), CoopNativePortError> {
        self.consume_recovery_response(response)
    }

    fn consume_recovery_response(
        &mut self,
        response: &CoopNativeRecoveryResponse,
    ) -> Result<(), CoopNativePortError> {
        self.on_recovery(response)
    }

    fn on_recovery(
        &mut self,
        _response: &CoopNativeRecoveryResponse,
    ) -> Result<(), CoopNativePortError> {
        Ok(())
    }
}
