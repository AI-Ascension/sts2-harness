// SPDX-License-Identifier: MIT

//! Structural separation between a launch acknowledgement and gameplay readiness.
//!
//! A launch acknowledgement says a process exists. It does not say the game
//! reached a playable state, that a save was loaded, that a host thread is
//! serving, or that any gameplay predicate holds. Those are different owners'
//! claims (issue #96), and this module exists so the difference is enforced by
//! the type system rather than by reviewer discipline.
//!
//! [`LaunchAcknowledgement`] deliberately does not implement
//! [`GameplayReadinessEvidence`], so it cannot be passed where readiness
//! evidence is required. The two types also have no conversion between them in
//! either direction, so no helper can launder one into the other. A readiness
//! predicate is satisfied only by a value a readiness owner constructed from
//! its own authoritative observation.

/// Evidence that a process was launched, as reported by the lifecycle surface.
///
/// Carries the operation identity that produced it so a readiness owner can
/// attribute the acknowledgement without re-deriving it from the process
/// identity. It is *not* readiness evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAcknowledgement {
    pub operation_id: u64,
    pub instance_id: String,
    pub authority_epoch: u64,
    /// True only when the gateway reported the operation as started.
    pub started: bool,
}

/// Marker for evidence a readiness owner vouches for.
///
/// This trait is deliberately sealed: only types in this module implement it,
/// so no other module can declare an arbitrary value to be readiness evidence
/// and no lifecycle type can accidentally qualify.
pub trait GameplayReadinessEvidence: private::Sealed {
    /// The instance the evidence was observed for.
    fn instance_id(&self) -> &str;

    /// The authority epoch the observation was made under.
    fn authority_epoch(&self) -> u64;
}

mod private {
    pub trait Sealed {}
}

/// Readiness observed by an owner that actually inspected gameplay state.
///
/// Constructed only by [`ReadinessObservation::new`], which requires the
/// caller to supply the observation identity and digest of the gameplay
/// evidence it read. There is no constructor from a launch acknowledgement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessObservation {
    instance_id: String,
    authority_epoch: u64,
    observation_id: String,
    observation_digest: String,
}

impl ReadinessObservation {
    /// Records a readiness observation the owner actually made.
    ///
    /// `observation_id` and `observation_digest` are mandatory and non-empty:
    /// an owner cannot assert readiness without naming the observation it read.
    pub fn new(
        instance_id: impl Into<String>,
        authority_epoch: u64,
        observation_id: impl Into<String>,
        observation_digest: impl Into<String>,
    ) -> Result<Self, ReadinessError> {
        let instance_id = instance_id.into();
        let observation_id = observation_id.into();
        let observation_digest = observation_digest.into();
        if instance_id.is_empty()
            || observation_id.is_empty()
            || authority_epoch == 0
            || observation_digest.len() != 64
            || !observation_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ReadinessError::InvalidObservation);
        }
        Ok(Self {
            instance_id,
            authority_epoch,
            observation_id,
            observation_digest,
        })
    }

    /// The observation identity the owner read.
    #[must_use]
    pub fn observation_id(&self) -> &str {
        &self.observation_id
    }

    /// Digest of the gameplay evidence the owner read.
    #[must_use]
    pub fn observation_digest(&self) -> &str {
        &self.observation_digest
    }
}

impl private::Sealed for ReadinessObservation {}

impl GameplayReadinessEvidence for ReadinessObservation {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn authority_epoch(&self) -> u64 {
        self.authority_epoch
    }
}

/// Refusals a readiness predicate can report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessError {
    /// The observation identity or digest was missing or malformed.
    InvalidObservation,
}

impl std::fmt::Display for ReadinessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidObservation => {
                formatter.write_str("readiness observation is incomplete or malformed")
            }
        }
    }
}

impl std::error::Error for ReadinessError {}

/// A gameplay readiness predicate that requires separate authoritative evidence.
///
/// The predicate can be satisfied only by a [`GameplayReadinessEvidence`] value
/// bound to the same instance and authority epoch. A launch acknowledgement
/// cannot be substituted because it is not a readiness evidence type, and the
/// epoch/instance binding additionally rejects evidence captured under a
/// superseded authority epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleReadiness {
    instance_id: String,
    authority_epoch: u64,
    satisfied_by: Option<String>,
}

impl LifecycleReadiness {
    /// A predicate that is not yet satisfied for one instance and epoch.
    #[must_use]
    pub fn pending(instance_id: impl Into<String>, authority_epoch: u64) -> Self {
        Self {
            instance_id: instance_id.into(),
            authority_epoch,
            satisfied_by: None,
        }
    }

    /// Whether readiness has been established.
    #[must_use]
    pub const fn is_satisfied(&self) -> bool {
        self.satisfied_by.is_some()
    }

    /// The observation identity that satisfied the predicate, if any.
    #[must_use]
    pub fn satisfied_by(&self) -> Option<&str> {
        self.satisfied_by.as_deref()
    }

    /// Satisfies the predicate from separate authoritative readiness evidence.
    ///
    /// Fails closed when the evidence is for a different instance or a
    /// superseded authority epoch. There is no launch-based path into this
    /// function: the parameter type admits only readiness evidence.
    pub fn satisfy<E: GameplayReadinessEvidence>(
        &mut self,
        evidence: &E,
    ) -> Result<(), ReadinessError> {
        if evidence.instance_id() != self.instance_id
            || evidence.authority_epoch() != self.authority_epoch
        {
            return Err(ReadinessError::InvalidObservation);
        }
        self.satisfied_by = Some("authoritative_readiness_observation".to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn digest(seed: u8) -> String {
        std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
    }

    #[test]
    fn launch_acknowledgement_is_not_accepted_as_readiness_evidence() {
        // The only way to reach `satisfy` is through a type that implements
        // `GameplayReadinessEvidence`. `LaunchAcknowledgement` does not, and the
        // following would not compile if it were substituted for `evidence`:
        //
        //     let acknowledgement = LaunchAcknowledgement { .. };
        //     predicate.satisfy(&acknowledgement)?;
        //
        // The sealed trait plus the absent conversion pin that. This test
        // records the reachable half: a started launch leaves readiness pending.
        let acknowledgement = LaunchAcknowledgement {
            operation_id: 7,
            instance_id: "instance-1".to_owned(),
            authority_epoch: 3,
            started: true,
        };
        let predicate = LifecycleReadiness::pending(
            acknowledgement.instance_id.clone(),
            acknowledgement.authority_epoch,
        );
        assert!(acknowledgement.started);
        assert!(!predicate.is_satisfied());
    }

    #[test]
    fn readiness_requires_authoritative_evidence_at_the_current_epoch() {
        let mut predicate = LifecycleReadiness::pending("instance-1", 3);
        let stale =
            ReadinessObservation::new("instance-1", 2, "obs-1", digest(0xab)).expect("observation");
        assert_eq!(
            predicate.satisfy(&stale),
            Err(ReadinessError::InvalidObservation)
        );
        assert!(!predicate.is_satisfied());

        let foreign =
            ReadinessObservation::new("instance-2", 3, "obs-2", digest(0xcd)).expect("observation");
        assert_eq!(
            predicate.satisfy(&foreign),
            Err(ReadinessError::InvalidObservation)
        );
        assert!(!predicate.is_satisfied());

        let current =
            ReadinessObservation::new("instance-1", 3, "obs-3", digest(0xef)).expect("observation");
        predicate.satisfy(&current).expect("current evidence");
        assert!(predicate.is_satisfied());
    }

    #[test]
    fn readiness_observation_requires_a_named_digest() {
        assert_eq!(
            ReadinessObservation::new("instance-1", 1, "obs-1", "short"),
            Err(ReadinessError::InvalidObservation)
        );
        assert_eq!(
            ReadinessObservation::new("instance-1", 0, "obs-1", digest(0x01)),
            Err(ReadinessError::InvalidObservation)
        );
    }
}
