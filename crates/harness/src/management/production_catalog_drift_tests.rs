// SPDX-License-Identifier: MIT

//! `#94` execution-side target admission fence, exercised through the production adapter.
//!
//! The sibling boundary tests call [`super::super::execution::admission`]'s fence functions
//! directly. That pins the fence's *logic* but not its *wiring*: if the adapter stopped calling
//! `validate_live_catalog` the logic tests would stay green while a drifted target slipped through.
//! This module drives the real [`LiveWorkflowExecutionPort::submit_admitted_with_reservation`] with
//! a catalog that changed revision after preflight, so removing the production call site turns this
//! test red — the composition-level effect check prior PR #410 disclosed as missing.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use super::duplicate_run_tests::{
    CountingStore, admitted_request, counts, observation, submit, target_descriptor,
};
use super::{Counters, Policy, Provider, RuntimeFactory, Shared};
use crate::episode::RuntimeLeaseBinding;
use crate::management::{
    AuthContext, LiveTargetCatalogPort, LiveWorkflowExecutionPort, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, ManagementError, ProductionLiveWorkflowSessionFactory,
    RuntimeAuthorityBinding, TARGET_CATALOG_SCHEMA_VERSION, TargetCatalogResponse, WorkflowStore,
    digest_value, live_run_id,
};
use crate::provider_session::NativeCapabilities;

const INSTANCE_ID: &str = "instance-1";
const SESSION_ID: &str = "runtime-session";

/// A discovery double that serves the admitted target under a *different* catalog revision, so the
/// target drifted between preflight and submission. The descriptor is otherwise the exact one the
/// binding names, so only the catalog-revision fence can catch the divergence.
struct DriftedCatalog;

impl LiveTargetCatalogPort for DriftedCatalog {
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: "live.catalog.revoked".to_owned(),
            targets: vec![target_descriptor()],
        })
    }
}

/// The production served composition, with the gateway/MCP runtime and the provider as fixtures and
/// a caller-supplied discovery double so a fence's call site can be exercised against a catalog
/// other than the happy-path one.
fn live_port_with_catalog(
    counters: &Shared<Counters>,
    run_id: &str,
    catalog: Arc<dyn LiveTargetCatalogPort>,
) -> LiveWorkflowExecutionPort {
    let factory: Arc<dyn LiveWorkflowSessionFactory> = Arc::new(
        ProductionLiveWorkflowSessionFactory::new(
            serde_json::json!({"capabilities": []}),
            catalog,
            Arc::new(RuntimeFactory {
                authority: RuntimeAuthorityBinding {
                    instance_id: INSTANCE_ID.to_owned(),
                    session_id: SESSION_ID.to_owned(),
                    lease_id: "configured-lease".to_owned(),
                    lease_epoch: 1,
                    run_id: run_id.to_owned(),
                    episode_id: "catalog-drift-episode".to_owned(),
                    trajectory_id: "catalog-drift-trajectory".to_owned(),
                    trace_id: "catalog-drift-trace".to_owned(),
                    artifact_id: "catalog-drift-artifact".to_owned(),
                    agent_id: "catalog-drift-agent".to_owned(),
                    adapter_revision: "catalog-drift-adapter".to_owned(),
                    model_revision: "catalog-drift-model".to_owned(),
                    configuration_digest: "b".repeat(64),
                    output_schema_digest: "c".repeat(64),
                },
                acquired: RuntimeLeaseBinding {
                    instance_id: INSTANCE_ID.to_owned(),
                    session_id: SESSION_ID.to_owned(),
                    run_id: run_id.to_owned(),
                    lease_id: "gateway-recovery-lease".to_owned(),
                    lease_epoch: 3,
                },
                observation: observation("combat-1", 1),
                counters: Arc::clone(counters),
            }),
            Arc::new(Provider {
                counters: Arc::clone(counters),
            }),
            Arc::new(Policy),
            NativeCapabilities::fixture(),
        )
        .expect("production factory"),
    );
    LiveWorkflowExecutionPort::new(factory, LiveWorkflowOptions::default()).expect("live port")
}

#[test]
fn live_catalog_drift_at_the_boundary_is_refused_before_the_first_effect() {
    // A target whose catalog revision changed between preflight and submission must be refused
    // before the submission is reserved, a runtime is opened or a provider is contacted.
    let (request, definition_digest) = admitted_request();
    let request_digest = digest_value(&serde_json::to_value(&request).expect("request encode"))
        .expect("request digest");
    let run_id = live_run_id(&request, &definition_digest).expect("run identity");
    let actor = AuthContext::new("catalog-drift-actor", ["workflow:*".to_owned()]).expect("actor");

    let counters = Arc::new(Mutex::new(Counters::default()));
    let port = live_port_with_catalog(&counters, &run_id, Arc::new(DriftedCatalog));
    let store = Arc::new(CountingStore::new());
    let reservation_store: Arc<dyn WorkflowStore> = store.clone();

    let error = submit(
        &port,
        &request,
        &actor,
        &definition_digest,
        &request_digest,
        Arc::clone(&reservation_store),
    )
    .expect_err("a target whose catalog drifted after preflight must be refused");

    assert_eq!(error.code, "target_catalog_stale");
    assert_eq!(
        counts(&counters),
        (0, 0, 0, 0),
        "the refusal must not open a runtime or reach a provider"
    );
    assert_eq!(
        store.creates(),
        0,
        "the refusal must not reserve a durable submission"
    );
}
