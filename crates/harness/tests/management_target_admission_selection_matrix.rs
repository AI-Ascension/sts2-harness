// SPDX-License-Identifier: MIT

//! Target-selector refusals at the served admission boundary.
//!
//! `sts2-harness#94` acceptance criterion 2 requires that an unavailable
//! capability or profile, wrong instance, stale lease or invalid binding
//! produce *no* effect and no synthetic success. The target-level half of that
//! fence is `service_target_validation::validate_target_selection`: every
//! caller-supplied selector of a run target — execution profile, game profile,
//! save profile, inference profile, context capability and provider capability —
//! must be advertised by the catalog descriptor the operator serves, otherwise
//! the submission is refused with a capability-class error. Beside it,
//! `service_target_admission::validate_run_target_admission` holds the two
//! binding-versus-request consistency checks: the binding must name the
//! execution profile the run request names, and the game profile the admitted
//! workflow actually declares.
//!
//! The selector fence is reached twice on the served path: once by the read-only
//! preflight an operator inspects, and again by `revalidate_target_admission`,
//! which the service runs immediately before the execution port may launch work.
//! All eight refusals below were unpinned: each code appeared only at the site
//! that raises it, and the only execution-boundary suite in the tree exercises
//! four unrelated identity and digest codes. A selector check could therefore be
//! dropped and a submission would still be admitted against a target that never
//! advertised the selector.
//!
//! Everything here is synthetic and labelled as such: the catalog is an
//! in-memory per-actor double and the execution port only counts the
//! submissions it receives, so a pass is component evidence for the served
//! fence, not provider, game, host or lease evidence. The live adapter re-checks
//! a parallel subset of these selectors in `management::execution_admission`;
//! that path is not reached by this synthetic port.

#![allow(clippy::expect_used)]

use sts2_harness::management::{ErrorClass, RunTargetConfiguration};

#[path = "support/target_catalog_matrix_service.rs"]
mod fixture;
#[path = "support/target_catalog_matrix.rs"]
mod matrix;
#[path = "support/live_workflow.rs"]
mod support;

use fixture::{Outcome, operator, preflight, run_request, two_target_fixture};
use matrix::{ALPHA, alpha_descriptor, beta_descriptor, selection};

/// One caller-supplied selector the served descriptor does not advertise, the
/// mutation that forges it onto a target configuration, and the exact refusal it
/// must earn.
type Unadvertised = (&'static str, fn(&mut RunTargetConfiguration), &'static str);

const EXECUTION_PROFILE: Unadvertised = (
    "execution profile",
    |target| target.execution_profile = "live.workflow.v9".to_owned(),
    "target_profile_unavailable",
);
const GAME_PROFILE: Unadvertised = (
    "game profile",
    |target| target.game_profile = "sts2-live-v9".to_owned(),
    "target_game_profile_unavailable",
);
const SAVE_PROFILE: Unadvertised = (
    "save profile",
    |target| target.save_profile = Some("save-forged-v1".to_owned()),
    "target_save_profile_unavailable",
);
const INFERENCE_PROFILE: Unadvertised = (
    "inference profile",
    |target| target.inference_profile = Some("inference-forged-v1".to_owned()),
    "target_inference_profile_unavailable",
);
const CONTEXT_CAPABILITY: Unadvertised = (
    "context capability",
    |target| target.context_capability = Some("workflow.context.forged.v1".to_owned()),
    "target_context_capability_unavailable",
);
const PROVIDER_CAPABILITY: Unadvertised = (
    "provider capability",
    |target| target.provider_capability = Some("workflow.provider.forged.v1".to_owned()),
    "target_provider_capability_unavailable",
);

/// All six selectors, as preflight must refuse them.
const UNADVERTISED: [Unadvertised; 6] = [
    EXECUTION_PROFILE,
    GAME_PROFILE,
    SAVE_PROFILE,
    INFERENCE_PROFILE,
    CONTEXT_CAPABILITY,
    PROVIDER_CAPABILITY,
];

/// The five selectors a forged *binding* can carry past the
/// binding-versus-request consistency fence. The sixth, the game profile, cannot
/// be forged on a binding at all: `validate_run_target_admission` pins it to the
/// game profile the workflow itself declares, which is also the value the served
/// descriptor advertises, so a forged value is refused as a contradiction rather
/// than as an unadvertised namespace.
const UNADVERTISED_AT_SUBMIT: [Unadvertised; 5] = [
    EXECUTION_PROFILE,
    SAVE_PROFILE,
    INFERENCE_PROFILE,
    CONTEXT_CAPABILITY,
    PROVIDER_CAPABILITY,
];

/// Every selector the served descriptors do advertise is admitted, so the
/// refusals below are the fence working rather than a fixture that admits
/// nothing.
///
/// Alpha is served with no optional namespace and beta with all of them, so the
/// two positive controls exercise both sides of each optional check: an absent
/// selector stays absent, and a present one must still be advertised.
#[test]
fn advertised_selectors_preflight_and_unadvertised_selectors_are_refused() -> Outcome {
    let (_catalog, port, service) = two_target_fixture();
    let full = operator("operator");

    let alpha = preflight(
        &service,
        &full,
        "request-alpha-control",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    assert_eq!(alpha.target.save_profile, None);
    assert_eq!(alpha.target.inference_profile, None);
    assert_eq!(alpha.target.context_capability, None);

    let beta_target = selection(&beta_descriptor(), "live.workflow.v2");
    assert_eq!(
        beta_target.context_capability.as_deref(),
        Some("workflow.context.context.live.v1")
    );
    assert_eq!(beta_target.save_profile.as_deref(), Some("save-beta-v1"));
    assert_eq!(
        beta_target.inference_profile.as_deref(),
        Some("inference-beta-v1")
    );
    let beta = preflight(&service, &full, "request-beta-control", beta_target)?;
    assert_eq!(beta.target.provider_capability, None);
    assert_eq!(port.submissions(), 0, "admission performs no effect");

    for (index, (label, forge, code)) in UNADVERTISED.into_iter().enumerate() {
        let mut target = selection(&alpha_descriptor(), "live.workflow.v1");
        forge(&mut target);
        let error = preflight(
            &service,
            &full,
            &format!("request-unadvertised-{index}"),
            target,
        )
        .expect_err(&format!("an unadvertised {label} must not preflight"));
        assert_eq!(error.code, code, "unadvertised {label}");
        assert_eq!(
            error.class,
            ErrorClass::Capability,
            "unadvertised {label} is a capability refusal"
        );
        assert_eq!(port.submissions(), 0, "no effect for unadvertised {label}");
    }
    Ok(())
}

/// The same selectors are re-checked on the submit path, which is the last
/// admission fence before an execution port is allowed to launch work.
///
/// This is the half a catalog-drift test cannot reach: the served descriptor is
/// left byte-for-byte untouched and the *already-issued* binding is tampered
/// with instead — the shape a caller could produce by editing the admission it
/// was handed. Because the descriptor is unchanged, the only fence that can
/// produce these codes is `validate_target_selection`, re-run by
/// `revalidate_target_admission` immediately before the port. Each tampered
/// binding must be refused and must leave the execution port untouched.
#[test]
fn an_unadvertised_selector_on_an_admitted_binding_is_refused_at_submit() -> Outcome {
    let (_catalog, port, service) = two_target_fixture();
    let full = operator("operator");

    // Control: the untampered binding for the same target does reach the port,
    // so each refusal below is the fence rather than an unreachable fixture.
    let intact = preflight(
        &service,
        &full,
        "request-intact",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    service.submit_run(&full, run_request("request-intact", intact))?;
    let submissions = port.submissions();
    assert_eq!(submissions, 1, "the control binding must reach the port");

    for (index, (label, forge, code)) in UNADVERTISED_AT_SUBMIT.into_iter().enumerate() {
        let request_id = format!("request-tampered-{index}");
        let mut binding = preflight(
            &service,
            &full,
            &request_id,
            selection(&alpha_descriptor(), "live.workflow.v1"),
        )?;
        forge(&mut binding.target);
        let error = service
            .submit_run(&full, run_request(&request_id, binding))
            .expect_err(&format!(
                "a binding carrying an unadvertised {label} must not submit"
            ));
        assert_eq!(error.code, code, "tampered {label}");
        assert_eq!(
            error.class,
            ErrorClass::Capability,
            "tampered {label} is a capability refusal"
        );
        assert_eq!(
            port.submissions(),
            submissions,
            "no launch for tampered {label}"
        );
    }
    Ok(())
}

/// The sixth selector is pinned on the submit path by moving the served
/// descriptor instead of forging the binding.
///
/// The binding is obtained while the target still advertised `sts2-live-v1`, so
/// the selection is exactly what an operator would have been handed — and the
/// binding still carries it, because the workflow itself declares it. The target
/// then advertises a different game profile, so the exact profile this workflow
/// needs is no longer available, and the re-check before the port must refuse
/// rather than launch against a target that has moved out from under the run.
/// The replacement profile keeps the descriptor itself well formed: the refusal
/// below is the selector fence rejecting the binding, not the catalog rejecting
/// an empty namespace. Staleness fences sit *after* the selector fence, so the
/// exact capability code is the evidence that the selector fence — not
/// descriptor-drift detection — produced the refusal.
#[test]
fn a_descriptor_that_stops_advertising_the_admitted_game_profile_refuses_at_submit() -> Outcome {
    let (catalog, port, service) = two_target_fixture();
    let full = operator("operator");

    let control = preflight(
        &service,
        &full,
        "request-game-profile-control",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    service.submit_run(&full, run_request("request-game-profile-control", control))?;
    let submissions = port.submissions();
    assert_eq!(submissions, 1, "the control binding must reach the port");

    let binding = preflight(
        &service,
        &full,
        "request-game-profile-withdrawn",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    assert_eq!(binding.target.game_profile, "sts2-live-v1");

    catalog.mutate_target("operator", ALPHA, |descriptor| {
        descriptor.game_profiles = vec!["sts2-live-v9".to_owned()];
    });

    let error = service
        .submit_run(
            &full,
            run_request("request-game-profile-withdrawn", binding),
        )
        .expect_err("a game profile the target no longer advertises must not submit");
    assert_eq!(error.code, "target_game_profile_unavailable");
    assert_eq!(
        error.class,
        ErrorClass::Capability,
        "a game profile the target no longer advertises is a capability refusal"
    );
    assert_eq!(
        port.submissions(),
        submissions,
        "no launch for a game profile the target no longer advertises"
    );
    Ok(())
}

/// The two binding-versus-request consistency refusals, which run *before* the
/// selector fence and so must be pinned separately: a binding that contradicts
/// the run request it is attached to, and a binding whose game profile
/// contradicts the admitted workflow.
///
/// Both leave the served descriptor untouched, so nothing but the consistency
/// fence can produce these codes. The control submission proves the same
/// instance, profile and workflow are otherwise admissible.
#[test]
fn a_binding_that_contradicts_the_workflow_request_is_refused_at_submit() -> Outcome {
    let (_catalog, port, service) = two_target_fixture();
    let full = operator("operator");

    let control = preflight(
        &service,
        &full,
        "request-contradiction-control",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    service.submit_run(&full, run_request("request-contradiction-control", control))?;
    let submissions = port.submissions();
    assert_eq!(submissions, 1, "the control binding must reach the port");

    // (a) the run request names an execution profile the binding does not carry.
    let binding = preflight(
        &service,
        &full,
        "request-profile-contradiction",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    let mut request = run_request("request-profile-contradiction", binding);
    request.profile = "live.workflow.v2".to_owned();
    let error = service
        .submit_run(&full, request)
        .expect_err("a run request may not contradict its own admission");
    assert_eq!(error.code, "target_profile_mismatch");
    assert_eq!(
        error.class,
        ErrorClass::Conflict,
        "a contradicting profile is a conflict, not a capability refusal"
    );

    // (b) the binding names a game profile the admitted workflow does not use.
    let mut binding = preflight(
        &service,
        &full,
        "request-game-contradiction",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    binding.target.game_profile = "sts2-live-v9".to_owned();
    let error = service
        .submit_run(&full, run_request("request-game-contradiction", binding))
        .expect_err("a binding may not contradict the admitted workflow");
    assert_eq!(error.code, "target_game_profile_mismatch");
    assert_eq!(
        error.class,
        ErrorClass::Conflict,
        "a contradicting game profile is a conflict, not a capability refusal"
    );

    assert_eq!(
        port.submissions(),
        submissions,
        "no launch for either contradicting binding"
    );
    Ok(())
}
