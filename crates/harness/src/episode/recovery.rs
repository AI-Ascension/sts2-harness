// SPDX-License-Identifier: MIT

use super::observation::EpisodeObservation;
use super::transition::TransitionReceipt;
use super::{ReceiptQueryIdentity, ReceiptQueryResult};

/// Operations that are safe after contradiction or uncertain mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryOperation {
    Reobserve,
    Reconcile {
        operation_id: String,
    },
    /// Read one retained receipt using the complete original operation identity.
    ReceiptQuery {
        identity: Box<ReceiptQueryIdentity>,
    },
    ReleaseLease,
    StopEpisode,
}

/// Recovery result; it never contains a strategic action selected by fallback code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryResult {
    Observation(EpisodeObservation),
    Receipt(TransitionReceipt),
    ReceiptQuery(Box<ReceiptQueryResult>),
    Released,
    Stopped,
}

pub trait RecoveryPort {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError>;
    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError>;
    /// Reads retained evidence without observing, reconciling, or dispatching an action.
    fn query_receipt(
        &mut self,
        _identity: &ReceiptQueryIdentity,
    ) -> Result<ReceiptQueryResult, RecoveryError> {
        Err(RecoveryError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), RecoveryError>;
    fn stop_episode(&mut self) -> Result<(), RecoveryError>;
}

/// Bounded recovery controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryController {
    max_attempts: u8,
}

impl RecoveryController {
    pub fn new(max_attempts: u8) -> Result<Self, RecoveryError> {
        if max_attempts == 0 || max_attempts > 8 {
            return Err(RecoveryError::InvalidConfiguration);
        }
        Ok(Self { max_attempts })
    }

    pub fn recover<P: RecoveryPort>(
        &self,
        port: &mut P,
        operation: RecoveryOperation,
    ) -> Result<RecoveryResult, RecoveryError> {
        if let RecoveryOperation::Reconcile { operation_id } = &operation
            && operation_id.is_empty()
        {
            return Err(RecoveryError::InvalidOperation);
        }
        let mut attempts = 0;
        loop {
            attempts += 1;
            let result = match &operation {
                RecoveryOperation::Reobserve => port.reobserve().map(RecoveryResult::Observation),
                RecoveryOperation::Reconcile { operation_id } => {
                    port.reconcile(operation_id).map(RecoveryResult::Receipt)
                }
                RecoveryOperation::ReceiptQuery { identity } => port
                    .query_receipt(identity)
                    .map(|result| RecoveryResult::ReceiptQuery(Box::new(result))),
                RecoveryOperation::ReleaseLease => {
                    port.release_lease().map(|()| RecoveryResult::Released)
                }
                RecoveryOperation::StopEpisode => {
                    port.stop_episode().map(|()| RecoveryResult::Stopped)
                }
            };
            match result {
                Ok(result) => return Ok(result),
                Err(RecoveryError::PortFailure) if attempts < self.max_attempts => {}
                Err(RecoveryError::PortFailure) => return Err(RecoveryError::Exhausted),
                Err(error) => return Err(error),
            }
        }
    }

    /// Performs a bounded, same-operation retained receipt lookup.
    pub fn query_receipt<P: RecoveryPort>(
        &self,
        port: &mut P,
        identity: ReceiptQueryIdentity,
    ) -> Result<ReceiptQueryResult, RecoveryError> {
        match self.recover(
            port,
            RecoveryOperation::ReceiptQuery {
                identity: Box::new(identity),
            },
        )? {
            RecoveryResult::ReceiptQuery(result) => Ok(*result),
            _ => Err(RecoveryError::Terminal),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryError {
    InvalidConfiguration,
    InvalidOperation,
    Exhausted,
    PortFailure,
    Terminal,
    Unsupported,
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "recovery configuration is invalid",
            Self::InvalidOperation => "recovery operation identity is invalid",
            Self::Exhausted => "recovery attempt budget is exhausted",
            Self::PortFailure => "recovery port failed",
            Self::Terminal => "recovery boundary returned a terminal failure",
            Self::Unsupported => "recovery port does not support retained receipt queries",
        })
    }
}

impl std::error::Error for RecoveryError {}
