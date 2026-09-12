// SPDX-License-Identifier: MIT

//! Explicit workflow-to-provider-session inspection adapter.
//!
//! A provider session's scope is independent from a workflow management run.
//! This adapter therefore accepts only an explicit mapping at construction; it
//! never searches by a similarly named Context run or native provider ID.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::provider_session::{
    BindingState, HistoryCoverage, NativeOperationState, ProviderSessionBroker,
};

use super::{
    AuthContext, ManagementError, ProviderSessionBindingSummary, ProviderSessionInspectionPort,
    ProviderSessionInspectionResult, ProviderSessionOperationSummary, RunSnapshot,
    validate_identifier,
};

/// Read-only management projection over broker instances explicitly associated
/// with workflow runs by the Harness scheduler. Construction fails on an
/// ambiguous mapping. The browser never supplies any provider identifier.
pub struct ProviderSessionBrokerInspectionPort {
    brokers_by_workflow_run: BTreeMap<String, Arc<Mutex<ProviderSessionBroker>>>,
}

impl ProviderSessionBrokerInspectionPort {
    pub fn new(
        mappings: impl IntoIterator<Item = (String, Arc<Mutex<ProviderSessionBroker>>)>,
    ) -> Result<Self, ManagementError> {
        let mut brokers_by_workflow_run = BTreeMap::new();
        for (workflow_run_id, broker) in mappings {
            validate_identifier("workflow_run_id", &workflow_run_id)?;
            if brokers_by_workflow_run
                .insert(workflow_run_id, broker)
                .is_some()
            {
                return Err(ManagementError::conflict(
                    "provider_session_mapping_duplicate",
                    "a workflow run may have only one provider-session inspection mapping",
                ));
            }
        }
        Ok(Self {
            brokers_by_workflow_run,
        })
    }
}

impl ProviderSessionInspectionPort for ProviderSessionBrokerInspectionPort {
    fn list(
        &self,
        _actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionInspectionResult, ManagementError> {
        let workflow_run_id = snapshot.workflow_run_id.clone();
        let broker = self
            .brokers_by_workflow_run
            .get(&snapshot.workflow_run_id)
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "provider_session_mapping_unavailable",
                    "no explicit provider-session mapping exists for this workflow run",
                )
            })?
            .lock()
            .map_err(|_| {
                ManagementError::unavailable(
                    "provider_session_inspection_unavailable",
                    "provider-session inspection lock is unavailable",
                )
            })?;
        let broker_snapshot = broker.snapshot();
        Ok(ProviderSessionInspectionResult {
            workflow_run_id,
            bindings: broker_snapshot
                .bindings
                .iter()
                .map(|binding| ProviderSessionBindingSummary {
                    binding_id: binding.binding_id.clone(),
                    state: binding_state(binding.state).to_owned(),
                    history_coverage: history_coverage(binding.history_coverage).to_owned(),
                    game_dispatch_capability: false,
                })
                .collect(),
            operations: broker_snapshot
                .operations
                .iter()
                .map(|operation| ProviderSessionOperationSummary {
                    operation_id: operation.operation_id.clone(),
                    state: operation_state(operation.state).to_owned(),
                    game_effects: 0,
                    auto_resume: false,
                })
                .collect(),
            next_cursor: None,
        })
    }
}

const fn binding_state(state: BindingState) -> &'static str {
    match state {
        BindingState::Candidate => "candidate",
        BindingState::Held => "held",
        BindingState::Active => "active",
        BindingState::Recovering => "recovering",
        BindingState::Quarantined => "quarantined",
        BindingState::Retired => "retired",
        BindingState::Closed => "closed",
    }
}

const fn history_coverage(coverage: HistoryCoverage) -> &'static str {
    match coverage {
        HistoryCoverage::ApplicationManifestVerified => "application_manifest_verified",
        HistoryCoverage::ReportedPartial => "reported_partial",
        HistoryCoverage::Unknown => "unknown",
    }
}

const fn operation_state(state: NativeOperationState) -> &'static str {
    match state {
        NativeOperationState::Planned => "planned",
        NativeOperationState::IntentPersisted => "intent_persisted",
        NativeOperationState::Sent => "sent",
        NativeOperationState::Acknowledged => "acknowledged",
        NativeOperationState::Completed => "completed",
        NativeOperationState::Rejected => "rejected",
        NativeOperationState::Unknown => "unknown",
        NativeOperationState::Cancelled => "cancelled",
        NativeOperationState::Quarantined => "quarantined",
    }
}
