// SPDX-License-Identifier: MIT

//! Gateway-attested process birth and readiness, where a PID alone is never evidence.
//!
//! A PID can be reused, so a trial records an opaque birth token with a strictly positive
//! instance generation. A readiness proof is bound to one birth; a proof for a different
//! generation is stale and is refused before any action is admitted.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum bytes of a birth or readiness token.
pub const MAX_PROCESS_TOKEN_BYTES: usize = 256;

/// One gateway-attested process birth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProcessBirth {
    /// Opaque gateway-attested birth token; a PID is not sufficient.
    pub birth_token: String,
    /// Strictly positive generation of the instance that was born.
    pub instance_generation: u64,
}

/// A readiness proof bound to exactly one process birth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadinessProof {
    /// The birth this proof was observed for.
    pub birth: ProcessBirth,
    /// Opaque readiness token.
    pub proof_token: String,
    /// Generation the proof itself reports; must equal `birth.instance_generation`.
    pub instance_generation: u64,
}

/// Rejection reasons for a process birth or readiness proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessError {
    /// A birth or readiness token is empty, oversized or contains a NUL separator.
    InvalidToken,
    /// The instance generation is zero.
    ZeroGeneration,
    /// The readiness proof reports a different generation than its birth.
    StaleReadiness,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidToken => "process token is invalid",
            Self::ZeroGeneration => "process instance generation is zero",
            Self::StaleReadiness => "readiness proof does not match its birth",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ProcessError {}

impl ProcessBirth {
    /// Validates the birth token and generation.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidToken`] or [`ProcessError::ZeroGeneration`].
    pub fn validate(&self) -> Result<(), ProcessError> {
        if !token_ok(&self.birth_token) {
            return Err(ProcessError::InvalidToken);
        }
        if self.instance_generation == 0 {
            return Err(ProcessError::ZeroGeneration);
        }
        Ok(())
    }
}

impl ReadinessProof {
    /// Builds a proof for one birth.
    #[must_use]
    pub fn for_birth(birth: ProcessBirth, proof_token: &str, instance_generation: u64) -> Self {
        Self {
            birth,
            proof_token: proof_token.to_owned(),
            instance_generation,
        }
    }

    /// Validates the proof and binds it to its birth generation.
    ///
    /// # Errors
    ///
    /// Returns the birth rejection, [`ProcessError::InvalidToken`] for a malformed proof token,
    /// or [`ProcessError::StaleReadiness`] when the generations disagree.
    pub fn validate(&self) -> Result<(), ProcessError> {
        self.birth.validate()?;
        if !token_ok(&self.proof_token) {
            return Err(ProcessError::InvalidToken);
        }
        if self.instance_generation != self.birth.instance_generation {
            return Err(ProcessError::StaleReadiness);
        }
        Ok(())
    }
}

fn token_ok(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_PROCESS_TOKEN_BYTES && !value.contains('\0')
}
