// SPDX-License-Identifier: MIT

//! The synthetic management adapter: the fixture service constructors the CLI and process driver
//! build, plus the wiring for the port implementations that live in the `workflow_ports_*`
//! siblings.
//!
//! Split out of one 931-line file so each file is genuinely sized by the policy gate instead of
//! being skipped by a `policy.toml` exemption. The implementations group along their own seams —
//! definition admission, context inspection, capability reporting, in-memory and persistent
//! execution, and replay — and share a small support module for error translation.
//! Refs sts2-harness#570.

use std::sync::Arc;

use super::workflow_ports_capability::SyntheticCapabilityPort;

use super::workflow_ports_context::SyntheticContextInspectionPort;

pub(super) use super::workflow_ports_definition::SyntheticDefinitionPort;

// Re-exported rather than reached through the sibling module: eight other
// `management` modules call these as `workflow_ports::parse_definition` and
// `workflow_ports::raw_digest`, so the split must not move a call site.
pub(super) use super::workflow_ports_definition::{parse_definition, raw_digest};

use super::workflow_ports_execution::SyntheticExecutionPort;

use super::workflow_ports_execution_persistent::PersistentSyntheticExecutionPort;

use super::workflow_ports_replay::SyntheticReplayPort;

/// The adapter owns no gameplay authority and returns only deterministic fixture
/// outcomes through the same strict runtime boundary used by component tests.
pub fn synthetic_file_store(
    store: super::store::FileWorkflowStore,
) -> super::service::ManagementService {
    synthetic_store(Arc::new(store))
}

pub fn synthetic_store(
    store: Arc<dyn super::store::WorkflowStore>,
) -> super::service::ManagementService {
    super::service::ManagementService::new(store)
        .with_definition_port(Arc::new(SyntheticDefinitionPort))
        .with_execution_port(Arc::new(SyntheticExecutionPort::default()))
        .with_replay_port(Arc::new(SyntheticReplayPort))
        .with_capability_port(Arc::new(SyntheticCapabilityPort))
        .with_context_inspection_port(Arc::new(SyntheticContextInspectionPort))
        .with_context_owner_port(Arc::new(
            super::synthetic_context_owner::SyntheticContextOwnerPort,
        ))
        .with_inference_profile_revision_journal(Arc::new(
            super::MemoryInferenceProfileRevisionJournal::default(),
        ))
}

pub fn synthetic_sqlite_store(
    store: Arc<super::store::SqliteWorkflowStore>,
) -> super::service::ManagementService {
    let service_store: Arc<dyn super::store::WorkflowStore> = store.clone();
    let journal: Arc<dyn super::InferenceProfileRevisionJournal> = Arc::new(
        super::SqliteInferenceProfileRevisionJournal::new(store.clone()),
    );
    super::service::ManagementService::new(service_store)
        .with_authoring_store(store.clone())
        .with_definition_port(Arc::new(SyntheticDefinitionPort))
        .with_execution_port(Arc::new(PersistentSyntheticExecutionPort::new(store)))
        .with_replay_port(Arc::new(SyntheticReplayPort))
        .with_capability_port(Arc::new(SyntheticCapabilityPort))
        .with_context_inspection_port(Arc::new(SyntheticContextInspectionPort))
        .with_context_owner_port(Arc::new(
            super::synthetic_context_owner::SyntheticContextOwnerPort,
        ))
        .with_inference_profile_revision_journal(journal)
}
