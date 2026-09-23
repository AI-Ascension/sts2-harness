// SPDX-License-Identifier: MIT

//! The per-trial cold-launch stage machine.
//!
//! Each stage records its effect before the next begins, and a stage may only advance from the
//! one it expects. A lost reply is reconciled against the recorded birth, and a controller
//! restart is never `Launched` because no new birth was attested.

use super::baseline::PristineBaseline;
use super::error::ColdLaunchError;
use super::lease::Destination;
use super::process::{ProcessBirth, ProcessError, ReadinessProof};
use super::stage::{ColdLaunchStage, MAX_COLD_TRIAL_KEY_BYTES};

/// One admitted cold-launch trial and the stage it has reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrialLifecycle {
    trial_key: String,
    baseline: PristineBaseline,
    stage: ColdLaunchStage,
    destination: Option<Destination>,
    birth: Option<ProcessBirth>,
    readiness: Option<ReadinessProof>,
}

impl TrialLifecycle {
    /// Admits a trial against a validated baseline.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::InvalidTrialKey`] for an empty or oversized key and the
    /// baseline rejection for an invalid declaration.
    pub fn admit(trial_key: &str, baseline: PristineBaseline) -> Result<Self, ColdLaunchError> {
        if trial_key.is_empty() || trial_key.len() > MAX_COLD_TRIAL_KEY_BYTES {
            return Err(ColdLaunchError::InvalidTrialKey);
        }
        baseline.validate()?;
        Ok(Self {
            trial_key: trial_key.to_owned(),
            baseline,
            stage: ColdLaunchStage::Admitted,
            destination: None,
            birth: None,
            readiness: None,
        })
    }

    /// Returns the trial key.
    #[must_use]
    pub fn trial_key(&self) -> &str {
        &self.trial_key
    }

    /// Admits a trial only when its baseline is interchangeable with an admitted reference.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::BaselineMismatch`] when the declared baseline differs from the
    /// reference in any category, so a mismatched build or profile cannot reach action admission.
    pub fn admit_against(
        trial_key: &str,
        baseline: PristineBaseline,
        reference: &PristineBaseline,
    ) -> Result<Self, ColdLaunchError> {
        let trial = Self::admit(trial_key, baseline)?;
        let reasons = trial.baseline.compare(reference);
        if reasons.is_empty() {
            Ok(trial)
        } else {
            Err(ColdLaunchError::BaselineMismatch { reasons })
        }
    }

    /// Returns the immutable baseline this trial was admitted against.
    #[must_use]
    pub fn baseline(&self) -> &PristineBaseline {
        &self.baseline
    }

    /// Returns the current stage.
    #[must_use]
    pub fn stage(&self) -> ColdLaunchStage {
        self.stage
    }

    /// Returns the leased destination, if any is still held.
    #[must_use]
    pub fn destination(&self) -> Option<&Destination> {
        self.destination.as_ref()
    }

    /// Returns the attested process birth, if any.
    #[must_use]
    pub fn birth(&self) -> Option<&ProcessBirth> {
        self.birth.as_ref()
    }

    /// Returns the readiness proof, if any.
    #[must_use]
    pub fn readiness(&self) -> Option<&ReadinessProof> {
        self.readiness.as_ref()
    }

    /// Reports whether cleanup failed, independently of the gameplay outcome.
    #[must_use]
    pub fn cleanup_failed(&self) -> bool {
        self.stage == ColdLaunchStage::CleanupFailed
    }

    /// Reports whether the trial stopped and cleaned successfully.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.stage == ColdLaunchStage::Cleaned
    }

    /// Reserves a fresh writable destination for this trial alone.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `Admitted` and
    /// [`ColdLaunchError::ForeignReconciliation`] when the destination belongs to another trial.
    pub fn reserve_destination(&mut self, destination: Destination) -> Result<(), ColdLaunchError> {
        self.require(
            ColdLaunchStage::Admitted,
            ColdLaunchStage::DestinationReserved,
        )?;
        if destination.trial_key != self.trial_key {
            return Err(ColdLaunchError::ForeignReconciliation {
                trial_key: destination.trial_key,
            });
        }
        self.destination = Some(destination);
        self.stage = ColdLaunchStage::DestinationReserved;
        Ok(())
    }

    /// Records that a separate writable clone was provisioned from the immutable baseline.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `DestinationReserved`.
    pub fn record_provisioned(&mut self) -> Result<(), ColdLaunchError> {
        self.require(
            ColdLaunchStage::DestinationReserved,
            ColdLaunchStage::Provisioned,
        )?;
        self.stage = ColdLaunchStage::Provisioned;
        Ok(())
    }

    /// Records a new native process birth; an in-process reset is not a launch.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `Provisioned` and the process
    /// rejection for a malformed birth.
    pub fn record_launched(&mut self, birth: ProcessBirth) -> Result<(), ColdLaunchError> {
        self.require(ColdLaunchStage::Provisioned, ColdLaunchStage::Launched)?;
        birth.validate()?;
        self.birth = Some(birth);
        self.stage = ColdLaunchStage::Launched;
        Ok(())
    }

    /// Records a readiness proof for the current birth generation.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `Launched`, the process rejection
    /// for a malformed proof, and [`ProcessError::StaleReadiness`] when the proof does not match
    /// the attested birth.
    pub fn record_ready(&mut self, proof: ReadinessProof) -> Result<(), ColdLaunchError> {
        self.require(ColdLaunchStage::Launched, ColdLaunchStage::Ready)?;
        proof.validate()?;
        match &self.birth {
            Some(birth) if birth == &proof.birth => {}
            _ => return Err(ColdLaunchError::Process(ProcessError::StaleReadiness)),
        }
        self.readiness = Some(proof);
        self.stage = ColdLaunchStage::Ready;
        Ok(())
    }

    /// Records that authored setup settled before action admission.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `Ready`.
    pub fn record_setup_settled(&mut self) -> Result<(), ColdLaunchError> {
        self.require(ColdLaunchStage::Ready, ColdLaunchStage::SetupSettled)?;
        self.stage = ColdLaunchStage::SetupSettled;
        Ok(())
    }

    /// Records that the seeded run started.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `SetupSettled`.
    pub fn record_running(&mut self) -> Result<(), ColdLaunchError> {
        self.require(ColdLaunchStage::SetupSettled, ColdLaunchStage::Running)?;
        self.stage = ColdLaunchStage::Running;
        Ok(())
    }

    /// Records that the process was stopped, retaining evidence.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] before the process is ready.
    pub fn record_stopped(&mut self) -> Result<(), ColdLaunchError> {
        match self.stage {
            ColdLaunchStage::Ready | ColdLaunchStage::SetupSettled | ColdLaunchStage::Running => {}
            other => return Err(self.illegal(other, ColdLaunchStage::Stopped)),
        }
        self.stage = ColdLaunchStage::Stopped;
        Ok(())
    }

    /// Records successful cleanup; the destination may now be released.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] outside `Stopped`.
    pub fn record_cleaned(&mut self) -> Result<(), ColdLaunchError> {
        self.require(ColdLaunchStage::Stopped, ColdLaunchStage::Cleaned)?;
        self.destination = None;
        self.stage = ColdLaunchStage::Cleaned;
        Ok(())
    }

    /// Records failed cleanup, which is distinct from the gameplay outcome.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] before the process is ready.
    pub fn record_cleanup_failed(&mut self) -> Result<(), ColdLaunchError> {
        match self.stage {
            ColdLaunchStage::Ready
            | ColdLaunchStage::SetupSettled
            | ColdLaunchStage::Running
            | ColdLaunchStage::Stopped => {}
            other => return Err(self.illegal(other, ColdLaunchStage::CleanupFailed)),
        }
        self.stage = ColdLaunchStage::CleanupFailed;
        Ok(())
    }

    /// Quarantines an uncertain destination; it is never reused.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::IllegalTransition`] when the trial already completed or is
    /// already quarantined.
    pub fn quarantine(&mut self) -> Result<(), ColdLaunchError> {
        if matches!(
            self.stage,
            ColdLaunchStage::Cleaned | ColdLaunchStage::Quarantined
        ) {
            return Err(self.illegal(self.stage, ColdLaunchStage::Quarantined));
        }
        self.stage = ColdLaunchStage::Quarantined;
        Ok(())
    }

    /// Reconciles a lost launch reply against the recorded birth.
    ///
    /// A reply for the recorded birth is idempotent, a reply for another birth is refused, and a
    /// reply arriving with no recorded birth is adopted only from `Provisioned`, so a controller
    /// restart cannot silently become a launch.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::ForeignReconciliation`] when the reply cannot be adopted and
    /// the process rejection for a malformed adopted birth.
    pub fn reconcile_lost_reply(
        &mut self,
        reply_birth: &ProcessBirth,
    ) -> Result<(), ColdLaunchError> {
        match &self.birth {
            Some(birth) if birth == reply_birth => Ok(()),
            Some(_) => Err(ColdLaunchError::ForeignReconciliation {
                trial_key: self.trial_key.clone(),
            }),
            None if self.stage == ColdLaunchStage::Provisioned => {
                reply_birth.validate()?;
                self.birth = Some(reply_birth.clone());
                self.stage = ColdLaunchStage::Launched;
                Ok(())
            }
            None => Err(ColdLaunchError::ForeignReconciliation {
                trial_key: self.trial_key.clone(),
            }),
        }
    }

    fn require(
        &self,
        expected: ColdLaunchStage,
        next: ColdLaunchStage,
    ) -> Result<(), ColdLaunchError> {
        if self.stage == expected {
            Ok(())
        } else {
            Err(self.illegal(self.stage, next))
        }
    }

    fn illegal(&self, from: ColdLaunchStage, to: ColdLaunchStage) -> ColdLaunchError {
        ColdLaunchError::IllegalTransition {
            from: from.label(),
            to: to.label(),
        }
    }
}
