// SPDX-License-Identifier: MIT

//! Capability admission at the production live entry point.
//!
//! `sts2-harness#94` acceptance criterion 2 requires that an unsupported node or capability causes
//! no effects and no synthetic success. The merged `live_workflow.rs` coverage exercises only the
//! *unknown node kind* arm of `node_diagnostics` (a `pause` node, which `NODE_CAPABILITIES` does not
//! list at all). Two distinct production arms were therefore unproven:
//!
//! 1. `node_capability_unavailable` in `crates/harness/src/management/validation.rs` — a node kind
//!    the live lane *does* support, whose required capability the owner does **not** advertise. This
//!    is the real "advertised vs wired" boundary: an owner that wires a node but forgets to advertise
//!    its capability must fail closed rather than execute the node.
//! 2. `capability_unavailable` in `crates/harness/src/management/workflow_ports.rs` — a definition
//!    whose own `capabilities.required` names something the owner does not advertise.
//!
//! Both must refuse at `submit_run` before `LiveWorkflowSessionFactory::open` is ever called, so the
//! factory's launch log proves no live effect occurred. Each test asserts the specific diagnostic
//! code so it cannot pass on an unrelated failure.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    LiveWorkflowOptions, LiveWorkflowSessionFactory, MANAGEMENT_SCHEMA_VERSION,
    MemoryWorkflowStore, ValidateRequest,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

/// A factory that advertises exactly the given capability list, so a test can withhold one
/// capability without changing the node kinds the lane supports.
struct AdvertisingFactory {
    inner: Arc<FakeFactory>,
    advertised: Vec<String>,
}

impl AdvertisingFactory {
    fn new<I, S>(advertised: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            inner: Arc::new(FakeFactory::new(false)),
            advertised: advertised
                .into_iter()
                .map(|item| item.as_ref().to_owned())
                .collect(),
        }
    }
}

impl LiveWorkflowSessionFactory for AdvertisingFactory {
    fn capabilities(&self) -> Value {
        let mut value = capabilities();
        value["capabilities"] = json!(self.advertised);
        value
    }

    fn target_catalog(
        &self,
        actor: &sts2_harness::management::AuthContext,
    ) -> Result<
        sts2_harness::management::TargetCatalogResponse,
        sts2_harness::management::ManagementError,
    > {
        self.inner.target_catalog(actor)
    }

    fn open(
        &self,
        request: &sts2_harness::management::RunRequest,
        actor: &sts2_harness::management::AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<
        Box<dyn sts2_harness::management::LiveWorkflowSession>,
        sts2_harness::management::ManagementError,
    > {
        self.inner
            .open(request, actor, definition, definition_digest)
    }

    fn open_admitted(
        &self,
        request: &sts2_harness::management::RunRequest,
        actor: &sts2_harness::management::AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<
        Box<dyn sts2_harness::management::LiveWorkflowSession>,
        sts2_harness::management::ManagementError,
    > {
        self.inner.open_admitted(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )
    }
}

fn advertised_without(withheld: &str) -> Vec<String> {
    LIVE_CAPABILITIES
        .iter()
        .filter(|item| **item != withheld)
        .map(|item| (*item).to_owned())
        .collect()
}

/// The diagnostic codes the served definition port reports for a definition validated against the
/// given advertised capability list. This is what distinguishes the two refusal arms: both collapse
/// to the aggregate `definition_invalid` at `submit_run`, so asserting the aggregate alone would not
/// prove which production branch fired.
fn validation(
    service: &sts2_harness::management::ManagementService,
    definition: &Value,
    advertised: &[String],
) -> sts2_harness::management::ValidateResponse {
    service
        .validate(
            &actor(),
            ValidateRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                definition: definition.clone(),
                capabilities: json!({
                    "capabilities": advertised,
                    "context_bindings": [{
                        "context_ref": "context.live.v1",
                        "node_kinds": ["decide"]
                    }]
                }),
            },
        )
        .expect("validate")
}

fn diagnostic_codes(
    service: &sts2_harness::management::ManagementService,
    definition: &Value,
    advertised: &[String],
) -> Vec<String> {
    let response = validation(service, definition, advertised);
    assert!(
        !response.valid,
        "the withheld capability must make the definition invalid"
    );
    response
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect()
}

#[test]
fn wired_but_unadvertised_node_capability_is_refused_before_live_launch() {
    // `decide` is a node kind this lane supports, so the definition is structurally valid; only its
    // advertised capability is missing. This is the branch that was previously untested.
    let withheld = "workflow.node.decide.v1";
    let advertised = advertised_without(withheld);
    let factory = Arc::new(AdvertisingFactory::new(advertised.clone()));
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");

    // Discrimination: the per-node arm fires, and only that arm. `decide` is a known node kind, so
    // the "unsupported node kind" arm cannot be what refused this definition.
    assert_eq!(
        diagnostic_codes(&service, &definition(false), &advertised),
        vec!["node_capability_unavailable".to_owned()],
        "the per-node advertised-vs-wired arm did not fire in isolation"
    );

    let error = service
        .submit_run(
            &actor(),
            request("request-capability-node", definition(false)),
        )
        .expect_err("a wired but unadvertised node capability must be refused");

    assert_eq!(
        error.code, "definition_invalid",
        "unexpected refusal: {error:?}"
    );
    assert!(
        factory.inner.entries().is_empty(),
        "an unadvertised node capability produced a live effect: {:?}",
        factory.inner.entries()
    );
    assert!(
        factory.inner.launches().is_empty(),
        "an unadvertised node capability opened a live session"
    );
}

#[test]
fn unadvertised_required_capability_is_refused_before_live_launch() {
    // The definition itself requires a capability the owner does not advertise. This drives the
    // definition-level `capability_unavailable` arm rather than the per-node arm.
    //
    // Withhold a capability the fixture already requires instead of appending a new one: appending a
    // capability that is already listed would instead trip the structural `duplicate_identifier`
    // diagnostic in `validate_definition`, which `parse_definition` raises *before*
    // `capability_diagnostics` runs. That would refuse for the wrong reason and prove nothing.
    let withheld = "actions.settlement.v1";
    let advertised = advertised_without(withheld);
    let factory = Arc::new(AdvertisingFactory::new(advertised.clone()));
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");

    // Discrimination: the definition-level arm fires, and only that arm. Every node kind the
    // definition uses is still advertised, so the per-node arm cannot be what refused it.
    assert_eq!(
        diagnostic_codes(&service, &definition(false), &advertised),
        vec!["capability_unavailable".to_owned()],
        "the definition-level required-capability arm did not fire in isolation"
    );

    let error = service
        .submit_run(
            &actor(),
            request("request-capability-required", definition(false)),
        )
        .expect_err("an unadvertised required capability must be refused");

    assert_eq!(
        error.code, "definition_invalid",
        "unexpected refusal: {error:?}"
    );
    assert!(
        factory.inner.entries().is_empty(),
        "an unadvertised required capability produced a live effect: {:?}",
        factory.inner.entries()
    );
    assert!(
        factory.inner.launches().is_empty(),
        "an unadvertised required capability opened a live session"
    );
}

#[test]
fn advertised_capabilities_still_admit_so_the_guard_is_not_blanket_rejection() {
    // A check that would fail if the production connection were removed: with every capability
    // advertised, the same definition must still be admitted and reach a live session. Without this,
    // the two refusals above would also pass for an owner that rejects everything.
    let factory = Arc::new(AdvertisingFactory::new(LIVE_CAPABILITIES));
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");

    assert!(
        validation(
            &service,
            &definition(false),
            &LIVE_CAPABILITIES
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        )
        .valid,
        "the positive control definition must validate clean against its advertised set"
    );

    let submitted = service
        .submit_run(
            &actor(),
            request("request-capability-admitted", definition(false)),
        )
        .expect("a fully advertised definition must be admitted");
    assert!(!submitted.workflow_run_id.is_empty());
    assert_eq!(
        factory.inner.launches().len(),
        1,
        "admitted run did not open exactly one live session"
    );
}
