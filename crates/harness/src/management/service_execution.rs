// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

use super::super::auth::AuthContext;
use super::super::contract::{
    CommandRequest, PendingOperation, RecoveryAdmission, RunEvent, RunRequest, RunSnapshot,
    TargetAdmissionBinding,
};
use super::super::store::WorkflowStore;
use super::ManagementError;

/// A service-owned capability for crossing the live submission effect
/// boundary. Its constructor and operation are crate-private so callers
/// cannot provide a no-op or otherwise unverified callback to the live
/// execution port.
pub struct RunReservation {
    store: Arc<dyn WorkflowStore>,
    request_id: String,
    request_digest: String,
    definition_digest: String,
    binding: Option<TargetAdmissionBinding>,
    snapshot: Mutex<Option<RunSnapshot>>,
}

impl RunReservation {
    pub(crate) fn new(
        store: Arc<dyn WorkflowStore>,
        request_id: String,
        request_digest: String,
        definition_digest: String,
        binding: Option<TargetAdmissionBinding>,
    ) -> Self {
        Self {
            store,
            request_id,
            request_digest,
            definition_digest,
            binding,
            snapshot: Mutex::new(None),
        }
    }

    pub(crate) fn reserve(&self, candidate: &RunAdmission) -> Result<(), ManagementError> {
        let mut durable = candidate.clone();
        durable.snapshot = super::target_admission::bind_snapshot_admission(
            durable.snapshot,
            self.binding.as_ref(),
        )?;
        super::support::verify_admission(&durable, &self.definition_digest)?;
        let persisted_snapshot = durable.snapshot.clone();
        self.store.create_run(
            &self.request_id,
            &self.request_digest,
            durable.snapshot,
            durable.initial_events,
        )?;
        *self.snapshot.lock().map_err(|_| {
            ManagementError::store(
                "reservation_state_lock",
                "live reservation state lock is poisoned",
            )
        })? = Some(persisted_snapshot);
        Ok(())
    }

    pub(crate) fn take_snapshot(&self) -> Result<Option<RunSnapshot>, ManagementError> {
        self.snapshot
            .lock()
            .map(|mut snapshot| snapshot.take())
            .map_err(|_| {
                ManagementError::store(
                    "reservation_state_lock",
                    "live reservation state lock is poisoned",
                )
            })
    }
}

pub trait WorkflowExecutionPort: Send + Sync {
    fn submit(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError>;

    /// Submit a run with the exact binding returned by the actor-scoped
    /// preflight. Adapters that do not need a separate admission boundary
    /// inherit a compatibility implementation which still carries the
    /// binding into the durable snapshot.
    fn submit_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
    ) -> Result<RunAdmission, ManagementError> {
        let mut result = self.submit(request, actor, definition_digest)?;
        if let Some(binding) = admission {
            if let Some(existing) = result.snapshot.admission.as_ref()
                && existing != binding
            {
                return Err(ManagementError::conflict(
                    "port_admission_mismatch",
                    "execution port returned a different target admission",
                ));
            }
            result.snapshot.admission = Some(binding.clone());
        }
        Ok(result)
    }

    /// Submit while giving the execution adapter a durable reservation hook.
    ///
    /// Live adapters override this method and invoke `reserve` before opening
    /// a session or launching an episode. The default keeps compatibility with
    /// existing adapters; those adapters still receive the exact binding and
    /// must not cross a mutating boundary from the callback itself.
    fn submit_admitted_with_reservation(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
        reserve: &RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        let result = self.submit_admitted(request, actor, definition_digest, admission)?;
        reserve.reserve(&result)?;
        Ok(result)
    }

    /// Returns an execution-adapter-specific recovery admission when the
    /// durable snapshot alone cannot distinguish same-process liveness from a
    /// restarted coordinator. `None` keeps the generic synthetic policy.
    fn recovery_admission(&self, _snapshot: &RunSnapshot) -> Option<RecoveryAdmission> {
        None
    }

    /// Aborts a live submission whose durable finalization failed. Adapters
    /// without an external session have nothing to clean up; live adapters
    /// remove the in-memory session and attempt stop/release before the
    /// service records the reserved run as needing operator recovery.
    fn abort_submission(&self, _run_id: &str) -> Result<(), ManagementError> {
        Ok(())
    }

    fn apply_command(&self, context: CommandContext)
    -> Result<CommandApplication, ManagementError>;

    /// Applies a command while allowing an execution adapter to durably record
    /// an operation intent immediately before crossing a mutating boundary.
    ///
    /// Existing adapters inherit the direct command path. Live adapters use
    /// this hook to preserve one operation identity across transport errors.
    fn apply_command_with_intent(
        &self,
        context: CommandContext,
        _record_intent: &dyn Fn(PendingOperation) -> Result<(), ManagementError>,
    ) -> Result<CommandApplication, ManagementError> {
        self.apply_command(context)
    }
}

#[derive(Clone, Debug)]
pub struct RunAdmission {
    pub snapshot: RunSnapshot,
    pub initial_events: Vec<RunEvent>,
}

#[derive(Clone)]
pub struct CommandContext {
    pub request: CommandRequest,
    pub snapshot: RunSnapshot,
    pub actor: AuthContext,
    /// Actor-scoped context owner used at dispatch time. It travels with the
    /// command so a shared execution adapter never stores or replaces it.
    pub context_owner: Arc<dyn super::super::context_owner::ContextOwnerPort>,
}

impl std::fmt::Debug for CommandContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandContext")
            .field("request", &self.request)
            .field("snapshot", &self.snapshot)
            .field("actor", &self.actor)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct CommandApplication {
    pub snapshot: RunSnapshot,
    pub outcome: super::super::contract::CommandOutcome,
    pub reason_code: String,
}
