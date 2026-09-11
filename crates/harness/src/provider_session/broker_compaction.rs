// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    pub fn plan_compaction(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        job_id: &str,
        budget_ref: Option<String>,
        generation_permission: bool,
    ) -> Result<CompactionJob, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/compact/start")
        {
            return Err(SessionError::Unsupported);
        }
        let binding = self.ensure_binding_not_expired(binding_id)?;
        if binding.state != BindingState::Held || !generation_permission || !valid_id(job_id) {
            return Err(SessionError::Forbidden);
        }
        let budget_ref = budget_ref.filter(|value| valid_id(value));
        if self.compaction_jobs.contains_key(job_id) {
            return Err(SessionError::Conflict);
        }
        if budget_ref.is_none()
            || self.compaction_jobs.values().any(|job| {
                job.binding_id == binding_id
                    && matches!(
                        job.state,
                        CompactionState::Planned
                            | CompactionState::Sent
                            | CompactionState::Acknowledged
                            | CompactionState::Transforming
                    )
            })
        {
            return Err(SessionError::Conflict);
        }
        if self.compaction_jobs.len() >= MAX_MAINTENANCE_JOBS {
            return Err(SessionError::Capacity);
        }
        let job = CompactionJob {
            schema: SESSION_COMPACTION_SCHEMA.to_owned(),
            job_id: job_id.to_owned(),
            scope: self.scope.clone(),
            binding_id: binding_id.to_owned(),
            source_history_epoch: binding.history_epoch,
            source_continuity_sha256: binding.continuity_sha256.clone(),
            dependency_ids: binding.dependency_ids.clone(),
            state: CompactionState::Planned,
            generation_permission,
            budget_reservation_ref: budget_ref,
            ack_received: false,
            terminal_evidence_ref: None,
            history_epoch_after: None,
            representation: CompactionRepresentation::Pending,
            automatic_adoption: false,
            game_effects: 0,
            scheduler_after: "held".to_owned(),
        };
        self.compaction_jobs.insert(job_id.to_owned(), job.clone());
        Ok(job)
    }

    pub fn send_compaction(
        &mut self,
        owner_token: &str,
        job_id: &str,
    ) -> Result<CompactionJob, SessionError> {
        self.authorize_owner(owner_token)?;
        let binding_id = self
            .compaction_jobs
            .get(job_id)
            .ok_or(SessionError::NotFound)?
            .binding_id
            .clone();
        self.ensure_binding_not_expired(&binding_id)?;
        let job = self
            .compaction_jobs
            .get_mut(job_id)
            .ok_or(SessionError::NotFound)?;
        if job.state != CompactionState::Planned || !job.generation_permission {
            return Err(SessionError::Conflict);
        }
        job.state = CompactionState::Sent;
        Ok(job.clone())
    }

    pub fn acknowledge_compaction(
        &mut self,
        owner_token: &str,
        job_id: &str,
    ) -> Result<CompactionJob, SessionError> {
        self.authorize_owner(owner_token)?;
        let binding_id = self
            .compaction_jobs
            .get(job_id)
            .ok_or(SessionError::NotFound)?
            .binding_id
            .clone();
        self.ensure_binding_not_expired(&binding_id)?;
        let job = self
            .compaction_jobs
            .get_mut(job_id)
            .ok_or(SessionError::NotFound)?;
        if job.state == CompactionState::Acknowledged {
            return Ok(job.clone());
        }
        if job.state != CompactionState::Sent {
            return Err(SessionError::Conflict);
        }
        job.state = CompactionState::Acknowledged;
        job.ack_received = true;
        Ok(job.clone())
    }

    pub fn complete_compaction(
        &mut self,
        owner_token: &str,
        job_id: &str,
        evidence_ref: &str,
    ) -> Result<CompactionJob, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(evidence_ref) {
            return Err(SessionError::InvalidRequest);
        }
        let binding_id = self
            .compaction_jobs
            .get(job_id)
            .ok_or(SessionError::NotFound)?
            .binding_id
            .clone();
        self.ensure_binding_not_expired(&binding_id)?;
        let job = self
            .compaction_jobs
            .get_mut(job_id)
            .ok_or(SessionError::NotFound)?;
        if job.state == CompactionState::Completed {
            return if job.terminal_evidence_ref.as_deref() == Some(evidence_ref) {
                Ok(job.clone())
            } else {
                Err(SessionError::Conflict)
            };
        }
        if !matches!(
            job.state,
            CompactionState::Acknowledged | CompactionState::Transforming
        ) {
            return Err(SessionError::Conflict);
        }
        let binding_id = job.binding_id.clone();
        let binding = self
            .bindings
            .get_mut(&binding_id)
            .ok_or(SessionError::NotFound)?;
        if binding.history_epoch != job.source_history_epoch
            || binding.continuity_sha256 != job.source_continuity_sha256
        {
            job.state = CompactionState::Failed;
            return Err(SessionError::Stale);
        }
        binding.history_epoch = binding.history_epoch.saturating_add(1);
        binding.compaction_epoch = binding.compaction_epoch.saturating_add(1);
        binding.state = BindingState::Held;
        binding.game_dispatch_capability = false;
        job.history_epoch_after = Some(binding.history_epoch);
        job.state = CompactionState::Completed;
        job.representation = CompactionRepresentation::OpaqueNative;
        job.terminal_evidence_ref = Some(evidence_ref.to_owned());
        let result = job.clone();
        self.emit(
            &binding_id,
            None,
            SessionEventKind::TransformObserved,
            SessionEventStatus::Observed,
            1,
        );
        Ok(result)
    }

    pub fn cancel_compaction(
        &mut self,
        owner_token: &str,
        job_id: &str,
    ) -> Result<CompactionJob, SessionError> {
        self.authorize_owner(owner_token)?;
        let binding_id = self
            .compaction_jobs
            .get(job_id)
            .ok_or(SessionError::NotFound)?
            .binding_id
            .clone();
        self.ensure_binding_not_expired(&binding_id)?;
        let job = self
            .compaction_jobs
            .get_mut(job_id)
            .ok_or(SessionError::NotFound)?;
        if !matches!(
            job.state,
            CompactionState::Planned | CompactionState::Sent | CompactionState::Acknowledged
        ) {
            return Err(SessionError::Conflict);
        }
        if job.state == CompactionState::Acknowledged {
            job.state = CompactionState::Unknown;
            let binding_id = job.binding_id.clone();
            if let Some(binding) = self.bindings.get_mut(&binding_id) {
                binding.state = BindingState::Recovering;
                binding.game_dispatch_capability = false;
            }
        } else {
            job.state = CompactionState::Cancelled;
        }
        Ok(job.clone())
    }
}
