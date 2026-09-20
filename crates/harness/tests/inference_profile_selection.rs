// SPDX-License-Identifier: MIT

//! Target-level inference-profile selection across both admission fences (#104).
//!
//! Everything here is synthetic and labelled as such: the catalogs are
//! in-memory owner doubles and the execution port only records that it was
//! reached, so a pass is component evidence for the fences, not provider
//! evidence. A target-level selection is the path the served runtime takes when
//! a consumer names the adapter its target advertises.

#![allow(clippy::expect_used)]

use std::sync::atomic::Ordering;

use sts2_harness::management::{
    AuthContext, INFERENCE_PROFILE_PROVENANCE_PREFIX, ManagementError, RunSubmissionResponse,
    decode_strict, is_provenance_reference,
};
use sts2_harness::workflow::WorkflowDefinition;

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;
#[path = "support/inference_profile_admission_fixtures.rs"]
mod inference_profile_admission_fixtures;
#[path = "support/inference_profile_catalog_doubles.rs"]
mod inference_profile_catalog_doubles;
#[path = "support/inference_profile_catalog_fixtures.rs"]
mod inference_profile_catalog_fixtures;

use inference_profile_catalog_doubles::*;
use inference_profile_catalog_fixtures::*;

fn actor() -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new("operator", ["workflow:*".to_owned()])?)
}

fn submit(fixture: &Fixture) -> Result<RunSubmissionResponse, ManagementError> {
    fixture
        .service
        .submit_run(&actor().expect("actor"), fixture.request.clone())
}

fn two_node_definition_typed() -> Result<WorkflowDefinition, Box<dyn std::error::Error>> {
    Ok(decode_strict(&serde_json::to_vec(&two_node_definition(
        "decision.synthetic.v1",
        "planner.synthetic.v1",
    ))?)?)
}

#[test]
fn a_target_level_selection_is_admitted_and_its_seal_is_reproducible_without_it()
-> Result<(), Box<dyn std::error::Error>> {
    // The served runtime publishes one reviewed provider-session adapter and its
    // target advertises exactly that name, so selecting it is the documented
    // path. The execution-side fence reads the durable admission, where the
    // selection has already been replaced by the resolved reference; it can only
    // reproduce the first fence's verdict if the seal covers the bindings and
    // not the selection.
    let fixture = fixture_with_selection(
        vec![baseline_catalog()],
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-selected",
        "synthetic.provider.v1",
    )?;
    let response = submit(&fixture)?;
    let persisted = fixture
        .store
        .get_run(&response.workflow_run_id)?
        .ok_or("run snapshot missing after submission")?;
    let provenance = persisted
        .admission
        .as_ref()
        .and_then(|binding| binding.target.inference_profile.clone())
        .ok_or("resolved provenance was not persisted")?;
    assert!(
        is_provenance_reference(&provenance),
        "a selected target must persist a resolved reference, got {provenance}"
    );
    assert_eq!(
        fixture.submissions.load(Ordering::SeqCst),
        1,
        "a selected target must reach exactly one admitted submission"
    );

    let catalog = baseline_catalog();
    let definition = two_node_definition_typed()?;
    let selected = sts2_harness::management::resolve_definition(
        &catalog,
        &definition,
        Some("synthetic.provider.v1"),
    )?;
    let reselected = sts2_harness::management::resolve_definition(&catalog, &definition, None)?;
    assert_eq!(
        selected.reference(),
        reselected.reference(),
        "the selection must not enter the seal, or no fence can reproduce it"
    );
    assert_eq!(selected.reference(), provenance);
    assert_eq!(
        selected.target_inference_profile.as_deref(),
        Some("synthetic.provider.v1")
    );

    // A selection the resolved adapters do not carry is still refused.
    let mismatch = sts2_harness::management::resolve_definition(
        &catalog,
        &definition,
        Some("other.provider.v1"),
    )
    .err()
    .ok_or("a non-matching selection must be refused")?;
    assert_eq!(mismatch.code, "inference_profile_adapter_mismatch");
    Ok(())
}

#[test]
fn a_selection_shaped_like_a_provenance_reference_is_still_checked_as_a_selection()
-> Result<(), Box<dyn std::error::Error>> {
    // `validate_identifier` accepts `.`, so a selection may legitimately begin
    // with — or exactly match — the provenance prefix. It stays a selection and
    // is admitted or refused as one; silently dropping it would skip the
    // target-level adapter check entirely.
    let selection = format!(
        "{INFERENCE_PROFILE_PROVENANCE_PREFIX}{}",
        "0123456789abcdef".repeat(4)
    );
    assert!(is_provenance_reference(&selection));
    assert!(
        !is_provenance_reference(&format!(
            "{INFERENCE_PROFILE_PROVENANCE_PREFIX}not-a-digest"
        )),
        "only the exact reference shape may be exempt from the target profile list"
    );
    let fixture = fixture_with_selection(
        vec![baseline_catalog()],
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-provenance-shaped-selection",
        &selection,
    )?;
    let error = submit(&fixture)
        .err()
        .ok_or("a provenance-shaped selection was dropped instead of checked")?;
    assert_eq!(error.code, "inference_profile_adapter_mismatch");
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);
    Ok(())
}
